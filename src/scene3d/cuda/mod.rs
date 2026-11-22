//! Optional CUDA backend for 3D scenes (the `cuda` build feature, Linux, NVIDIA).
//!
//! It draws the same frames as the CPU rasterizer and does the same arithmetic. The host evaluates
//! the scene, clips, and snaps every triangle exactly as [`super::raster`] does ([`setups`]); one
//! kernel (`raster.cu`) then covers samples, depth-tests them in draw order, and resolves each
//! pixel with the CPU's integer mean. The RGBA rows come back to the host and go to the same sink as
//! CPU rows. Only triangle rasterization, depth, and resolve run on the GPU; 2D composition,
//! effects, audio, and encoding stay where they are.
//!
//! The driver is loaded at run time (`driver.rs`), so the binary has no CUDA link dependency. The
//! kernel is precompiled by `tools/build_cuda_kernel.py` into a cubin for the device's exact
//! compute capability, or portable PTX the driver JIT-compiles for newer GPUs. [`kernel_dir`] says
//! where they are looked up. One context is shared by the process ([`shared`]); each renderer
//! keeps a [`Target`] with its device buffers. Any failure is reported to the caller, which falls
//! back to the CPU or, in strict mode, aborts the render.

mod driver;

use super::backend::fall_back;
use super::camera::{Placement, View};
use super::frame::Triangle;
use super::raster::{setups, Region, RowSink, Setup};
use anyhow::{bail, Context, Result};
use driver::{CUdeviceptr, Driver, Handle};
use std::ffi::{c_int, c_void, CString};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

/// PCI vendor id of NVIDIA (reported in the proof next to the Vulkan backend's).
pub const NVIDIA: u32 = 0x10DE;

/// Threads per block edge.
const BLOCK: u32 = 16;

/// The process-wide driver, context, and kernel.
pub struct Cuda {
    driver: Driver,
    context: Handle,
    function: Handle,
    pub name: String,
    pub uuid: String,
    pub capability: (i32, i32),
    pub driver_version: i32,
    /// The kernel artifact loaded, and whether the driver compiled it at load (PTX).
    pub module: String,
    pub jit: bool,
}

// SAFETY: the handles are process-global CUDA objects; every use binds the context first.
unsafe impl Send for Cuda {}
unsafe impl Sync for Cuda {}

static CUDA: OnceLock<Option<Cuda>> = OnceLock::new();
static LAUNCHES: AtomicU64 = AtomicU64::new(0);
static KERNEL_NS: AtomicU64 = AtomicU64::new(0);
static UPLOAD_NS: AtomicU64 = AtomicU64::new(0);
static READBACK_NS: AtomicU64 = AtomicU64::new(0);
static READBACK_BYTES: AtomicU64 = AtomicU64::new(0);

/// Where the kernel artifacts live: `AETHER_CUDA_KERNEL_DIR`, else `cuda/` next to the executable,
/// else the executable's folder.
pub fn kernel_dir() -> Vec<PathBuf> {
    if let Ok(dir) = std::env::var("AETHER_CUDA_KERNEL_DIR") {
        if !dir.trim().is_empty() {
            return vec![PathBuf::from(dir)];
        }
    }
    let exe = std::env::current_exe().ok();
    let dir = exe.as_deref().and_then(Path::parent).map(Path::to_path_buf);
    dir.map(|d| vec![d.join("cuda"), d]).unwrap_or_default()
}

/// The shared CUDA device, initialized on first use; `None` (with the reason recorded) when it
/// cannot be.
pub fn shared() -> Option<&'static Cuda> {
    CUDA.get_or_init(|| match Cuda::new() {
        Ok(cuda) => Some(cuda),
        Err(error) => {
            fall_back(format!("no usable CUDA device: {error:#}"));
            None
        }
    })
    .as_ref()
}

/// Whether CUDA has been initialized successfully (never initializes it).
pub fn available() -> bool {
    CUDA.get().is_some_and(Option::is_some)
}

/// The adapter name of the initialized device, if any.
pub fn adapter_name() -> Option<String> {
    CUDA.get().and_then(|c| c.as_ref()).map(|c| c.name.clone())
}

/// The strict-mode proof fields as JSON (`"cuda":{...}`), once a device is initialized.
pub fn proof_fields() -> Option<String> {
    let c = CUDA.get().and_then(|c| c.as_ref())?;
    let ms = |ns: &AtomicU64| ns.load(Ordering::Relaxed) as f64 / 1e6;
    Some(format!(
        "{{\"driver_version\":{},\"compute_capability\":\"{}.{}\",\"uuid\":{:?},\"module\":{:?},\"jit\":{},\
\"launches\":{},\"upload_ms\":{:.3},\"kernel_ms\":{:.3},\"readback_ms\":{:.3},\"readback_bytes\":{}}}",
        c.driver_version,
        c.capability.0,
        c.capability.1,
        c.uuid,
        c.module,
        c.jit,
        LAUNCHES.load(Ordering::Relaxed),
        ms(&UPLOAD_NS),
        ms(&KERNEL_NS),
        ms(&READBACK_NS),
        READBACK_BYTES.load(Ordering::Relaxed),
    ))
}

impl Cuda {
    fn new() -> Result<Self> {
        let d = Driver::load()?;
        // SAFETY (all calls below): valid out pointers, handles from earlier calls, one thread.
        unsafe {
            d.check("cuInit", (d.cuInit)(0))?;
            let (mut version, mut count) = (0, 0);
            d.check("cuDriverGetVersion", (d.cuDriverGetVersion)(&mut version))?;
            d.check("cuDeviceGetCount", (d.cuDeviceGetCount)(&mut count))?;
            if count < 1 {
                bail!("the driver reports no CUDA devices");
            }
            let mut device = 0;
            d.check("cuDeviceGet", (d.cuDeviceGet)(&mut device, 0))?;
            let mut name = [0 as std::ffi::c_char; 256];
            d.check(
                "cuDeviceGetName",
                (d.cuDeviceGetName)(name.as_mut_ptr(), 256, device),
            )?;
            let name = std::ffi::CStr::from_ptr(name.as_ptr())
                .to_string_lossy()
                .into_owned();
            let (mut major, mut minor) = (0, 0);
            d.check(
                "cuDeviceGetAttribute",
                (d.cuDeviceGetAttribute)(&mut major, driver::CC_MAJOR, device),
            )?;
            d.check(
                "cuDeviceGetAttribute",
                (d.cuDeviceGetAttribute)(&mut minor, driver::CC_MINOR, device),
            )?;
            let mut uuid = [0_u8; 16];
            d.check(
                "cuDeviceGetUuid",
                (d.cuDeviceGetUuid)(uuid.as_mut_ptr(), device),
            )?;
            let uuid = format!(
                "GPU-{}",
                uuid.iter().map(|b| format!("{b:02x}")).collect::<String>()
            );
            let mut context: Handle = std::ptr::null_mut();
            d.check(
                "cuDevicePrimaryCtxRetain",
                (d.cuDevicePrimaryCtxRetain)(&mut context, device),
            )?;
            d.check("cuCtxSetCurrent", (d.cuCtxSetCurrent)(context))?;
            let (module_name, mut image) = load_kernel(major, minor)?;
            let jit = module_name.ends_with(".ptx");
            image.push(0); // PTX must be NUL-terminated; harmless after a cubin
            let mut module: Handle = std::ptr::null_mut();
            d.check(
                "cuModuleLoadData",
                (d.cuModuleLoadData)(&mut module, image.as_ptr().cast()),
            )?;
            let mut function: Handle = std::ptr::null_mut();
            let entry = CString::new("rasterize")?;
            d.check(
                "cuModuleGetFunction",
                (d.cuModuleGetFunction)(&mut function, module, entry.as_ptr()),
            )?;
            Ok(Self {
                driver: d,
                context,
                function,
                name,
                uuid,
                capability: (major, minor),
                driver_version: version,
                module: module_name,
                jit,
            })
        }
    }
}

/// Reads the kernel artifact for this device: a cubin for its exact capability, else PTX.
fn load_kernel(major: c_int, minor: c_int) -> Result<(String, Vec<u8>)> {
    let dirs = kernel_dir();
    let names = [
        format!("raster_sm{major}{minor}.cubin"),
        "raster_compute75.ptx".to_string(),
    ];
    for dir in &dirs {
        for name in &names {
            if let Ok(bytes) = std::fs::read(dir.join(name)) {
                return Ok((name.clone(), bytes));
            }
        }
    }
    bail!(
        "no CUDA kernel found ({} or {}) in {:?}; build it with tools/build_cuda_kernel.py or set AETHER_CUDA_KERNEL_DIR",
        names[0],
        names[1],
        dirs
    )
}

/// A renderer's device buffers, sized to the largest draw so far.
#[derive(Default)]
pub struct Target {
    setups: Vec<Setup>,
    host: Vec<u32>,
    device_setups: Option<(CUdeviceptr, usize)>,
    device_out: Option<(CUdeviceptr, usize)>,
}

impl Target {
    /// Draws `triangles` into `region` and hands the resolved rows to `sink`. Nothing reaches the
    /// sink unless the whole draw succeeds, so a failure can fall back to the CPU cleanly.
    pub fn draw(
        &mut self,
        cuda: &Cuda,
        view: &View,
        triangles: &[Triangle],
        placement: &Placement,
        region: &Region,
        sink: &mut RowSink,
    ) -> Result<()> {
        setups(view, triangles, placement, region, &mut self.setups);
        if self.setups.is_empty() {
            return Ok(());
        }
        let d = &cuda.driver;
        let (w, h) = (region.width(), region.height());
        let setup_bytes = self.setups.len() * std::mem::size_of::<Setup>();
        let out_bytes = w * h * 4;
        // SAFETY: the context is bound; every pointer passed is a live host buffer or a device
        // allocation made below, with the byte counts they were sized for.
        unsafe {
            d.check("cuCtxSetCurrent", (d.cuCtxSetCurrent)(cuda.context))?;
            let t0 = Instant::now();
            let in_ptr = grow(d, &mut self.device_setups, setup_bytes)?;
            let out_ptr = grow(d, &mut self.device_out, out_bytes)?;
            d.check(
                "cuMemcpyHtoD",
                (d.cuMemcpyHtoD_v2)(in_ptr, self.setups.as_ptr().cast(), setup_bytes),
            )?;
            let t1 = Instant::now();
            let (count, width, height, ss) = (
                self.setups.len() as c_int,
                w as c_int,
                h as c_int,
                region.antialias as c_int,
            );
            let mut params: [*mut c_void; 6] = [
                (&in_ptr as *const CUdeviceptr).cast_mut().cast(),
                (&count as *const c_int).cast_mut().cast(),
                (&out_ptr as *const CUdeviceptr).cast_mut().cast(),
                (&width as *const c_int).cast_mut().cast(),
                (&height as *const c_int).cast_mut().cast(),
                (&ss as *const c_int).cast_mut().cast(),
            ];
            let grid = |n: usize| (n as u32).div_ceil(BLOCK);
            d.check(
                "cuLaunchKernel",
                (d.cuLaunchKernel)(
                    cuda.function,
                    grid(w),
                    grid(h),
                    1,
                    BLOCK,
                    BLOCK,
                    1,
                    0,
                    std::ptr::null_mut(),
                    params.as_mut_ptr(),
                    std::ptr::null_mut(),
                ),
            )?;
            d.check("cuCtxSynchronize", (d.cuCtxSynchronize)())?;
            let t2 = Instant::now();
            self.host.resize(w * h, 0);
            d.check(
                "cuMemcpyDtoH",
                (d.cuMemcpyDtoH_v2)(self.host.as_mut_ptr().cast(), out_ptr, out_bytes),
            )?;
            let t3 = Instant::now();
            UPLOAD_NS.fetch_add((t1 - t0).as_nanos() as u64, Ordering::Relaxed);
            KERNEL_NS.fetch_add((t2 - t1).as_nanos() as u64, Ordering::Relaxed);
            READBACK_NS.fetch_add((t3 - t2).as_nanos() as u64, Ordering::Relaxed);
            READBACK_BYTES.fetch_add(out_bytes as u64, Ordering::Relaxed);
        }
        LAUNCHES.fetch_add(1, Ordering::Relaxed);
        let mut row = vec![[0_u8; 4]; w];
        for y in 0..h {
            let pixels = &self.host[y * w..(y + 1) * w];
            if pixels.iter().any(|p| p >> 24 != 0) {
                for (out, p) in row.iter_mut().zip(pixels) {
                    *out = p.to_le_bytes();
                }
                sink(region.rect[0], region.rect[1] + y, &row);
            }
        }
        Ok(())
    }
}

/// The device pointer of an allocation of at least `bytes`, replacing a smaller one.
///
/// # Safety
/// The CUDA context must be current on this thread.
unsafe fn grow(
    d: &Driver,
    slot: &mut Option<(CUdeviceptr, usize)>,
    bytes: usize,
) -> Result<CUdeviceptr> {
    if let Some((ptr, cap)) = *slot {
        if cap >= bytes {
            return Ok(ptr);
        }
        d.check("cuMemFree", (d.cuMemFree_v2)(ptr))?;
        *slot = None;
    }
    let cap = bytes.next_power_of_two().max(4096);
    let mut ptr: CUdeviceptr = 0;
    d.check("cuMemAlloc", (d.cuMemAlloc_v2)(&mut ptr, cap))
        .context("device allocation")?;
    *slot = Some((ptr, cap));
    Ok(ptr)
}

impl Drop for Target {
    fn drop(&mut self) {
        let Some(cuda) = CUDA.get().and_then(|c| c.as_ref()) else {
            return;
        };
        let d = &cuda.driver;
        // SAFETY: the pointers were allocated by this target in the shared context; failures while
        // releasing memory at teardown have nowhere useful to go.
        unsafe {
            let _ = (d.cuCtxSetCurrent)(cuda.context);
            for (ptr, _) in [self.device_setups.take(), self.device_out.take()]
                .into_iter()
                .flatten()
            {
                let _ = (d.cuMemFree_v2)(ptr);
            }
        }
    }
}

// `Setup` is uploaded as is; the kernel declares the same 112-byte layout.
const _: () = assert!(std::mem::size_of::<Setup>() == 112);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene3d::camera::Placement;
    use crate::scene3d::raster::{rasterize, Scratch};
    use crate::scene3d::Projection;

    fn view() -> View {
        View {
            eye: [0.0, 0.0, 1000.0],
            axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            projection: Projection::Perspective { focal: 1000.0 },
            zoom: 1.0,
            near: 10.0,
            far: 50_000.0,
        }
    }

    fn tri(a: [f64; 3], b: [f64; 3], c: [f64; 3], color: [u8; 3]) -> Triangle {
        Triangle {
            corners: [a, b, c],
            color,
        }
    }

    /// Overlapping triangles, a depth tie, geometry clipped by the near plane and the guard band,
    /// and slanted edges at 4x4 supersampling: the CUDA image must equal the CPU image exactly.
    /// Skipped (with a note) on machines without an NVIDIA driver and the built kernel.
    #[test]
    fn cuda_matches_the_cpu_rasterizer() {
        let cuda = match Cuda::new() {
            Ok(cuda) => cuda,
            Err(error) => {
                eprintln!("skipped: no CUDA device or kernel ({error:#})");
                return;
            }
        };
        let triangles = [
            tri(
                [-60.0, -50.0, 0.0],
                [70.0, -40.0, 0.0],
                [10.0, 60.0, 0.0],
                [220, 40, 40],
            ),
            // Same plane and corners: a depth tie the first triangle keeps.
            tri(
                [-60.0, -50.0, 0.0],
                [70.0, -40.0, 0.0],
                [10.0, 60.0, 0.0],
                [40, 220, 40],
            ),
            tri(
                [-30.0, -70.0, 50.0],
                [90.0, 10.0, -30.0],
                [-70.0, 30.0, 20.0],
                [40, 40, 220],
            ),
            // Runs from the far distance to behind the eye.
            tri(
                [-500.0, 50.0, -2000.0],
                [500.0, 50.0, -2000.0],
                [0.0, 50.0, 3000.0],
                [90, 90, 90],
            ),
        ];
        let placement = Placement {
            origin: [0.0, 0.0],
            scale: 1.0,
            size: [97.0, 61.0],
        };
        for ss in [1_u32, 2, 4] {
            let region = Region::of(&placement, [97, 61], ss).unwrap();
            let mut expected = vec![[0_u8; 4]; 97 * 61];
            rasterize(
                &view(),
                &triangles,
                &placement,
                &region,
                &mut Scratch::default(),
                &mut |x, y, row| {
                    expected[y * 97 + x..y * 97 + x + row.len()].copy_from_slice(row);
                },
            );
            let mut actual = vec![[0_u8; 4]; 97 * 61];
            Target::default()
                .draw(
                    &cuda,
                    &view(),
                    &triangles,
                    &placement,
                    &region,
                    &mut |x, y, row| {
                        actual[y * 97 + x..y * 97 + x + row.len()].copy_from_slice(row);
                    },
                )
                .unwrap();
            let differing = expected.iter().zip(&actual).filter(|(a, b)| a != b).count();
            assert_eq!(differing, 0, "{differing} pixels differ at antialias {ss}");
            assert!(actual.iter().any(|p| p[3] == 255));
        }
    }
}
