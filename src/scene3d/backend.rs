//! Which backend draws 3D scenes, and the per-renderer workspace they draw with.
//!
//! The CPU rasterizer ([`super::raster`]) is the default and the reference: it is deterministic
//! across threads, render segments, and CPUs. `AETHER_3D_BACKEND=gpu` (or `--3d-backend gpu`)
//! asks for the wgpu backend, included by default. When the `gpu` feature is
//! missing, no adapter is found, or any GPU call fails, the scene is drawn on the CPU instead and
//! the reason is kept in [`status`]; after a GPU failure the whole process stays on the CPU so a
//! render never mixes backends from one frame to the next. `auto` uses the GPU when one
//! initializes and the CPU otherwise.
//!
//! Strict mode (`--require-gpu`, [`set_strict`]) is for exports that must not touch the CPU rasterizer:
//! the adapter must be an NVIDIA Vulkan device, any GPU failure aborts the render with an error
//! instead of falling back, and [`proof`] reports what drew so a caller can verify it.
//!
//! `cuda` (`--3d-backend cuda`, the `cuda` feature) is a separate, explicit request for the CUDA
//! backend ([`super::cuda`]): it never selects wgpu, and strict mode then requires a CUDA device.
//! `gpu` and `auto` keep meaning wgpu.

use super::camera::Placement;
use super::frame::Triangle;
use super::raster::{rasterize_with_beam, Region, RowSink, Scratch};
use super::{beam, NodeKind, Scene3d};
use anyhow::{bail, Result};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::Mutex;

/// The backend a user asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    Cpu,
    Gpu,
    Auto,
    /// The CUDA rasterizer, only when asked for by name.
    Cuda,
}

impl std::str::FromStr for Request {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value.trim() {
            "cpu" => Ok(Self::Cpu),
            "gpu" => Ok(Self::Gpu),
            "auto" => Ok(Self::Auto),
            "cuda" => Ok(Self::Cuda),
            other => bail!("3D backend must be cpu, gpu, cuda, or auto (got {other})"),
        }
    }
}

const UNSET: u8 = 0;
static REQUEST: AtomicU8 = AtomicU8::new(UNSET);
static FALLBACK: Mutex<Option<String>> = Mutex::new(None);
static STRICT: AtomicBool = AtomicBool::new(false);
static GPU_DRAWS: AtomicU64 = AtomicU64::new(0);

/// Makes any GPU failure abort the render and requires an NVIDIA Vulkan adapter (`--require-gpu`).
pub fn set_strict(strict: bool) {
    STRICT.store(strict, Ordering::Relaxed);
}

/// Whether strict GPU mode is on.
pub fn strict() -> bool {
    STRICT.load(Ordering::Relaxed)
}

/// One line of JSON for strict mode: the backend that drew, the adapter, how many draws the GPU
/// made, and whether anything fell back. A caller accepts a GPU export only when `backend` is
/// `gpu`, `api` is `Vulkan`, `vendor` is 4318 (NVIDIA), `draws` is positive, and `fallbacks` is 0.
pub fn proof() -> String {
    #[cfg(all(feature = "cuda", unix))]
    if matches!(request(), Ok(Request::Cuda)) {
        return cuda_proof();
    }
    let (backend, reason) = status();
    #[cfg(feature = "gpu")]
    let adapter = super::gpu::adapter_report();
    #[cfg(not(feature = "gpu"))]
    let adapter: Option<(String, String, u32)> = None;
    let (name, api, vendor) = adapter.unwrap_or_default();
    format!(
        "{{\"backend\":{backend:?},\"adapter\":{name:?},\"api\":{api:?},\"vendor\":{vendor},\"draws\":{},\"fallbacks\":{}}}",
        GPU_DRAWS.load(Ordering::Relaxed),
        u32::from(reason.is_some())
    )
}

/// The strict-mode proof of the CUDA backend: `backend` is `cuda`, `api` is `CUDA`, and the `cuda`
/// object carries the driver, device, kernel, and measured times. A caller accepts it only when
/// `backend` is `cuda`, `vendor` is 4318, `draws` and `cuda.launches` are positive (kernels that
/// ran to completion and were read back), and `fallbacks` is 0.
#[cfg(all(feature = "cuda", unix))]
fn cuda_proof() -> String {
    let (backend, reason) = status();
    let name = super::cuda::adapter_name().unwrap_or_default();
    let detail = super::cuda::proof_fields().unwrap_or_else(|| "null".into());
    format!(
        "{{\"backend\":{backend:?},\"adapter\":{name:?},\"api\":\"CUDA\",\"vendor\":{},\"draws\":{},\"fallbacks\":{},\"cuda\":{detail}}}",
        if name.is_empty() { 0 } else { super::cuda::NVIDIA },
        GPU_DRAWS.load(Ordering::Relaxed),
        u32::from(reason.is_some())
    )
}

fn encode(request: Request) -> u8 {
    match request {
        Request::Cpu => 1,
        Request::Gpu => 2,
        Request::Auto => 3,
        Request::Cuda => 4,
    }
}

/// Sets the backend for this process (the CLI flag); it takes precedence over the environment.
pub fn set_request(request: Request) {
    REQUEST.store(encode(request), Ordering::Relaxed);
}

/// The requested backend: [`set_request`], else `AETHER_3D_BACKEND`, else the CPU.
pub fn request() -> Result<Request> {
    match REQUEST.load(Ordering::Relaxed) {
        1 => Ok(Request::Cpu),
        2 => Ok(Request::Gpu),
        3 => Ok(Request::Auto),
        4 => Ok(Request::Cuda),
        _ => match std::env::var("AETHER_3D_BACKEND") {
            Ok(value) if !value.trim().is_empty() => value
                .parse()
                .map_err(|e| anyhow::anyhow!("AETHER_3D_BACKEND: {e}")),
            _ => Ok(Request::Cpu),
        },
    }
}

/// Records why the GPU is not used (the first reason wins).
pub(crate) fn fall_back(reason: String) {
    let mut slot = FALLBACK.lock().unwrap_or_else(|e| e.into_inner());
    if slot.is_none() {
        *slot = Some(reason);
    }
}

const BEAM_CPU_REASON: &str = "native <hf-beam> light shafts require the CPU 3D backend";

/// The backend 3D scenes are drawn with in this process so far (`"cpu"` or `"gpu"`) and, when the
/// GPU was asked for but not used, why.
pub fn status() -> (&'static str, Option<String>) {
    let reason = FALLBACK.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let gpu = matches!(request(), Ok(Request::Gpu | Request::Auto)) && reason.is_none();
    #[cfg(feature = "gpu")]
    let gpu = gpu && super::gpu::available();
    #[cfg(not(feature = "gpu"))]
    let gpu = gpu && false;
    #[cfg(all(feature = "cuda", unix))]
    let cuda =
        matches!(request(), Ok(Request::Cuda)) && reason.is_none() && super::cuda::available();
    #[cfg(not(all(feature = "cuda", unix)))]
    let cuda = false;
    (
        if cuda {
            "cuda"
        } else if gpu {
            "gpu"
        } else {
            "cpu"
        },
        reason,
    )
}

/// Largest resolved region (bytes) the memo keeps; bigger draws are not remembered.
const MEMO_BYTES: usize = 64 << 20;

/// What makes two draws identical: the scene, the clip time, the placement, the target size.
#[derive(Clone, Copy, PartialEq)]
struct DrawKey {
    scene: usize,
    time: u64,
    placement: [u64; 5],
    target: [usize; 2],
}

/// The last draw's resolved rows, replayed when the very same draw repeats (an effect layer such
/// as `data-hf-bloom` or a transition draws its subtree on black and then on white). Replaying
/// gives exactly the pixels a new draw would, so it never changes output.
#[derive(Default)]
struct Memo {
    key: Option<DrawKey>,
    region: Option<Region>,
    pixels: Vec<[u8; 4]>,
    /// Rows that hold geometry.
    rows: Vec<bool>,
}

/// Buffers one renderer reuses for every 3D frame it draws (bounded: the frame's triangles, one
/// CPU tile, the last resolved region (at most 64 MiB), and, on the GPU, one target the size of
/// the last region drawn).
#[derive(Default)]
pub struct Workspace {
    cpu_only: bool,
    triangles: Vec<Triangle>,
    raster: Scratch,
    memo: Memo,
    #[cfg(feature = "gpu")]
    gpu: Option<super::gpu::Target>,
    #[cfg(all(feature = "cuda", unix))]
    cuda: Option<super::cuda::Target>,
}

impl Workspace {
    /// Selects one backend for every viewport in the renderer.
    pub(crate) fn new(cpu_only: bool) -> Result<Self> {
        if cpu_only && strict() {
            bail!("--require-gpu: {BEAM_CPU_REASON}");
        }
        Ok(Self {
            cpu_only,
            ..Self::default()
        })
    }

    pub(crate) fn status(&self) -> (&'static str, Option<String>) {
        if self.cpu_only {
            let reason = matches!(request(), Ok(Request::Gpu | Request::Auto | Request::Cuda));
            ("cpu", reason.then(|| BEAM_CPU_REASON.to_owned()))
        } else {
            status()
        }
    }

    /// Bytes currently held on the CPU.
    pub fn bytes(&self) -> usize {
        self.triangles.capacity() * std::mem::size_of::<Triangle>()
            + self.raster.bytes()
            + self.memo.pixels.capacity() * 4
            + self.memo.rows.capacity()
    }

    /// Draws `scene` at clip time `t`, placed at `placement`, onto a `target` px surface, calling
    /// `sink(x, y, row)` with straight-alpha RGBA rows. Returns the region it covered, if any.
    pub fn draw(
        &mut self,
        scene: &Scene3d,
        t: f64,
        placement: &Placement,
        target: [usize; 2],
        sink: &mut RowSink,
    ) -> Result<Option<Region>> {
        let Some(region) = Region::of(placement, target, scene.antialias) else {
            return Ok(None);
        };
        let key = DrawKey {
            scene: std::ptr::from_ref(scene) as usize,
            time: t.to_bits(),
            placement: [
                placement.origin[0].to_bits(),
                placement.origin[1].to_bits(),
                placement.scale.to_bits(),
                placement.size[0].to_bits(),
                placement.size[1].to_bits(),
            ],
            target,
        };
        let memo = &mut self.memo;
        if memo.key == Some(key) && memo.region == Some(region) {
            let width = region.width();
            for (y, row) in memo.pixels.chunks_exact(width).enumerate() {
                if memo.rows[y] {
                    sink(region.rect[0], region.rect[1] + y, row);
                }
            }
            return Ok(Some(region));
        }
        let (width, height) = (region.width(), region.height());
        let remember = width * height * 4 <= MEMO_BYTES;
        memo.key = None;
        if remember {
            memo.pixels.clear();
            memo.pixels.resize(width * height, [0; 4]);
            memo.rows.clear();
            memo.rows.resize(height, false);
        }
        let shaft = scene.beam();
        let cpu_only = self.cpu_only || shaft.is_some();
        if cpu_only && strict() {
            bail!("--require-gpu: {BEAM_CPU_REASON}");
        }
        let view = scene.frame_into(t, &mut self.triangles);
        let beam = shaft.and_then(|(index, beam)| {
            let world = scene.world(t);
            let blocker = beam.occluder.and_then(|i| match &scene.nodes[i].kind {
                NodeKind::Mesh { mesh, .. } => Some((world[i], mesh.bounds())),
                _ => None,
            });
            beam::Prepared::new(beam, world[index], blocker)
        });
        let mut record = |x: usize, y: usize, row: &[[u8; 4]]| {
            if remember {
                let (dx, dy) = (x - region.rect[0], y - region.rect[1]);
                memo.pixels[dy * width + dx..dy * width + dx + row.len()].copy_from_slice(row);
                memo.rows[dy] = true;
            }
            sink(x, y, row);
        };
        if beam.is_some() || !self.triangles.is_empty() {
            let request = request()?;
            let drawn = match if cpu_only { Request::Cpu } else { request } {
                Request::Cpu => false,
                Request::Cuda => Self::try_cuda(
                    #[cfg(all(feature = "cuda", unix))]
                    &mut self.cuda,
                    &view,
                    &self.triangles,
                    placement,
                    &region,
                    &mut record,
                ),
                Request::Gpu | Request::Auto => Self::try_gpu(
                    #[cfg(feature = "gpu")]
                    &mut self.gpu,
                    &view,
                    &self.triangles,
                    placement,
                    &region,
                    &mut record,
                ),
            };
            if !drawn && strict() {
                let reason = FALLBACK
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone()
                    .unwrap_or_else(|| "the GPU did not draw".into());
                bail!("--require-gpu: {reason}");
            }
            if !drawn {
                rasterize_with_beam(
                    &view,
                    &self.triangles,
                    placement,
                    &region,
                    &mut self.raster,
                    beam.as_ref(),
                    &mut record,
                );
            }
        }
        if remember {
            memo.key = Some(key);
            memo.region = Some(region);
        }
        Ok(Some(region))
    }

    #[cfg(all(feature = "cuda", unix))]
    fn try_cuda(
        target: &mut Option<super::cuda::Target>,
        view: &super::camera::View,
        triangles: &[Triangle],
        placement: &Placement,
        region: &Region,
        sink: &mut RowSink,
    ) -> bool {
        if FALLBACK.lock().unwrap_or_else(|e| e.into_inner()).is_some() {
            return false;
        }
        let Some(cuda) = super::cuda::shared() else {
            return false;
        };
        let target = target.get_or_insert_with(super::cuda::Target::default);
        match target.draw(cuda, view, triangles, placement, region, sink) {
            Ok(()) => {
                GPU_DRAWS.fetch_add(1, Ordering::Relaxed);
                true
            }
            Err(error) => {
                fall_back(format!("CUDA draw failed: {error:#}"));
                false
            }
        }
    }

    #[cfg(not(all(feature = "cuda", unix)))]
    fn try_cuda(
        _: &super::camera::View,
        _: &[Triangle],
        _: &Placement,
        _: &Region,
        _: &mut RowSink,
    ) -> bool {
        fall_back("this build has no CUDA backend (build with --features cuda on Linux)".into());
        false
    }

    #[cfg(feature = "gpu")]
    fn try_gpu(
        target: &mut Option<super::gpu::Target>,
        view: &super::camera::View,
        triangles: &[Triangle],
        placement: &Placement,
        region: &Region,
        sink: &mut RowSink,
    ) -> bool {
        if FALLBACK.lock().unwrap_or_else(|e| e.into_inner()).is_some() {
            return false;
        }
        let Some(gpu) = super::gpu::shared() else {
            return false;
        };
        let target = target.get_or_insert_with(super::gpu::Target::default);
        match target.draw(gpu, view, triangles, placement, region, sink) {
            Ok(()) => {
                GPU_DRAWS.fetch_add(1, Ordering::Relaxed);
                true
            }
            Err(error) => {
                fall_back(format!("GPU draw failed: {error:#}"));
                false
            }
        }
    }

    #[cfg(not(feature = "gpu"))]
    fn try_gpu(
        _: &super::camera::View,
        _: &[Triangle],
        _: &Placement,
        _: &Region,
        _: &mut RowSink,
    ) -> bool {
        fall_back(
            "this build has no GPU backend (reinstall with Cargo default features enabled)".into(),
        );
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Color;
    use crate::scene3d::mesh::Primitive;
    use crate::scene3d::{CameraSpec, Channel, Cull, Material, Node, NodeKind, Shading, Transform};
    use std::sync::Arc;

    fn spinning_box() -> Scene3d {
        let mut transform = Transform::default();
        transform.rotation[1] = Channel::keyed(&[
            (0.0, 0.0, crate::camera::curve::Interp::Smooth),
            (1.0, 90.0, crate::camera::curve::Interp::Smooth),
        ]);
        Scene3d {
            nodes: vec![Node {
                id: None,
                parent: None,
                kind: NodeKind::Mesh {
                    mesh: Arc::new(Primitive::Box { size: [60.0; 3] }.mesh()),
                    material: Material {
                        shading: Shading::Lambert,
                        color: Color::WHITE,
                        cull: Cull::Back,
                    },
                    shape: "box",
                },
                transform,
            }],
            camera: None,
            default_camera: CameraSpec::default_for(1000.0),
            antialias: 2,
            default_lights: true,
            physics: None,
        }
    }

    fn draw(workspace: &mut Workspace, scene: &Scene3d, t: f64) -> Vec<[u8; 4]> {
        let placement = Placement {
            origin: [3.25, -1.5],
            scale: 1.0,
            size: [96.0, 64.0],
        };
        let mut image = vec![[0; 4]; 100 * 64];
        workspace
            .draw(scene, t, &placement, [100, 64], &mut |x, y, row| {
                image[y * 100 + x..y * 100 + x + row.len()].copy_from_slice(row);
            })
            .unwrap();
        image
    }

    #[test]
    fn replayed_draws_equal_fresh_ones() {
        let scene = spinning_box();
        let mut warm = Workspace::default();
        let first = draw(&mut warm, &scene, 0.4);
        // The same draw again comes from the memo; a fresh workspace draws it from scratch.
        assert_eq!(draw(&mut warm, &scene, 0.4), first);
        assert_eq!(draw(&mut Workspace::default(), &scene, 0.4), first);
        assert!(first.iter().any(|p| p[3] == 255));
        // Another instant is drawn anew, never replayed.
        let later = draw(&mut warm, &scene, 0.6);
        assert_ne!(later, first);
        assert_eq!(later, draw(&mut Workspace::default(), &scene, 0.6));
    }
}
