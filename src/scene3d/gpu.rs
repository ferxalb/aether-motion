//! Optional wgpu backend for 3D scenes (the `gpu` build feature).
//!
//! It draws the same [`Triangle`]s as the CPU rasterizer with the same [`ClipMatrix`] (converted
//! to `f32`, around the eye for precision), into a target of `antialias^2` samples per pixel with a
//! reversed `Depth32Float` buffer (`Greater` test, so ties keep the first triangle, as on the CPU)
//! and a scissor on the viewport's sample box. A second pass resolves each pixel with the CPU's
//! integer arithmetic (mean of the covered samples, alpha = covered share), so the backends differ
//! only where the hardware covers or depth-tests a sample differently from the fixed-point
//! rasterizer: along triangle edges and in depth ties. The resolved rows are read back and handed to
//! the same sink as CPU rows. See `docs/3d.md` for the measured tolerance.
//!
//! One device is shared by the whole process ([`shared`]); each renderer keeps a [`Target`] sized
//! to the last region it drew. Initialization failures leave the process on the CPU.

use super::backend::fall_back;
use super::camera::{Placement, View};
use super::frame::Triangle;
use super::raster::{Region, RowSink};
use anyhow::{anyhow, bail, Context, Result};
use std::sync::OnceLock;

const SHADER: &str = r#"
struct Uniforms { clip: mat4x4<f32> };
@group(0) @binding(0) var<uniform> u: Uniforms;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) color: vec4<f32>,
};

@vertex
fn vs(@location(0) p: vec3<f32>, @location(1) c: vec4<f32>) -> VertexOut {
    var out: VertexOut;
    out.position = u.clip * vec4<f32>(p, 1.0);
    out.color = vec4<f32>(c.rgb, 1.0);
    return out;
}

@fragment
fn fs(v: VertexOut) -> @location(0) vec4<f32> {
    return v.color;
}

@group(0) @binding(0) var samples: texture_2d<f32>;
@group(0) @binding(1) var<uniform> factor: vec4<u32>;

@vertex
fn resolve_vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

@fragment
fn resolve_fs(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let s = factor.x;
    let base = vec2<u32>(u32(p.x), u32(p.y)) * s;
    var n = 0u;
    var sum = vec3<u32>(0u, 0u, 0u);
    for (var y = 0u; y < s; y = y + 1u) {
        for (var x = 0u; x < s; x = x + 1u) {
            let t = textureLoad(samples, vec2<i32>(base + vec2<u32>(x, y)), 0);
            if (t.a > 0.5) {
                n = n + 1u;
                sum = sum + vec3<u32>(round(t.rgb * 255.0));
            }
        }
    }
    if (n == 0u) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    let total = s * s;
    let mean = (sum + vec3<u32>(n / 2u)) / vec3<u32>(n);
    let alpha = (n * 255u + total / 2u) / total;
    return vec4<f32>(vec3<f32>(mean), f32(alpha)) / 255.0;
}
"#;

/// The process-wide device and pipelines.
pub struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    scene: wgpu::RenderPipeline,
    scene_layout: wgpu::BindGroupLayout,
    resolve: wgpu::RenderPipeline,
    resolve_layout: wgpu::BindGroupLayout,
    /// Adapter name and backend, for reports.
    pub name: String,
}

static GPU: OnceLock<Option<Gpu>> = OnceLock::new();

/// The shared GPU, initialized on first use; `None` (with the reason recorded) when it cannot be.
pub fn shared() -> Option<&'static Gpu> {
    GPU.get_or_init(|| match pollster::block_on(Gpu::new()) {
        Ok(gpu) => Some(gpu),
        Err(error) => {
            fall_back(format!("no usable GPU: {error:#}"));
            None
        }
    })
    .as_ref()
}

/// Whether the GPU has been initialized successfully (never initializes it).
pub fn available() -> bool {
    GPU.get().is_some_and(Option::is_some)
}

/// Adapter description of the initialized GPU, if any.
pub fn adapter_name() -> Option<String> {
    GPU.get().and_then(|g| g.as_ref()).map(|g| g.name.clone())
}

const COLOR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

impl Gpu {
    async fn new() -> Result<Self> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                ..Default::default()
            })
            .await
            .map_err(|e| anyhow!("no adapter: {e}"))?;
        let info = adapter.get_info();
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("aether-3d"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_defaults()
                    .using_resolution(adapter.limits()),
                ..Default::default()
            })
            .await
            .context("could not open the GPU device")?;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("aether-3d"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let scene_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("aether-3d-scene"),
            entries: &[uniform_entry(0, wgpu::ShaderStages::VERTEX)],
        });
        let resolve_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("aether-3d-resolve"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                uniform_entry(1, wgpu::ShaderStages::FRAGMENT),
            ],
        });
        let layout = |bind: &wgpu::BindGroupLayout| {
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &[Some(bind)],
                ..Default::default()
            })
        };
        let scene = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("aether-3d-scene"),
            layout: Some(&layout(&scene_layout)),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 16,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Unorm8x4],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(COLOR.into())],
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let resolve = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("aether-3d-resolve"),
            layout: Some(&layout(&resolve_layout)),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("resolve_vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("resolve_fs"),
                compilation_options: Default::default(),
                targets: &[Some(COLOR.into())],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        Ok(Self {
            device,
            queue,
            scene,
            scene_layout,
            resolve,
            resolve_layout,
            name: format!("{} ({:?})", info.name, info.backend),
        })
    }
}

fn uniform_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// GPU resources of one renderer: sample targets, the resolved picture, and buffers, kept while
/// the region size stays the same.
#[derive(Default)]
pub struct Target {
    size: [u32; 2],
    antialias: u32,
    textures: Option<Textures>,
    vertices: Option<wgpu::Buffer>,
    staging: Vec<u8>,
}

struct Textures {
    color: wgpu::TextureView,
    depth: wgpu::TextureView,
    resolved: wgpu::Texture,
    readback: wgpu::Buffer,
    uniforms: wgpu::Buffer,
    _factor: wgpu::Buffer,
    scene_group: wgpu::BindGroup,
    resolve_group: wgpu::BindGroup,
    row_bytes: u32,
}

impl Target {
    /// Draws `triangles` into `region` and hands the resolved rows to `sink`. Nothing reaches the
    /// sink unless the whole draw succeeds, so a failure can fall back to the CPU cleanly.
    pub fn draw(
        &mut self,
        gpu: &Gpu,
        view: &View,
        triangles: &[Triangle],
        placement: &Placement,
        region: &Region,
        sink: &mut RowSink,
    ) -> Result<()> {
        let size = [region.width() as u32, region.height() as u32];
        let ss = region.antialias;
        let limit = gpu.device.limits().max_texture_dimension_2d;
        if size[0] * ss > limit || size[1] * ss > limit {
            bail!(
                "a {}x{} region at {ss}x supersampling exceeds the GPU's {limit} px textures",
                size[0],
                size[1]
            );
        }
        if self.textures.is_none() || self.size != size || self.antialias != ss {
            self.textures = Some(Textures::new(gpu, size, ss));
            self.size = size;
            self.antialias = ss;
        }
        let textures = self.textures.as_ref().expect("allocated");
        // The clip matrix around the eye, so vertices keep f32 precision.
        let m = region.matrix(view, placement).0;
        let eye = view.eye;
        let mut columns = [0.0_f32; 16];
        for c in 0..4 {
            for r in 0..4 {
                let value = if c == 3 {
                    m[r][3] + m[r][0] * eye[0] + m[r][1] * eye[1] + m[r][2] * eye[2]
                } else {
                    m[r][c]
                };
                columns[c * 4 + r] = value as f32;
            }
        }
        gpu.queue
            .write_buffer(&textures.uniforms, 0, &f32_bytes(&columns));
        self.staging.clear();
        for triangle in triangles {
            for corner in &triangle.corners {
                for (axis, value) in corner.iter().enumerate() {
                    self.staging
                        .extend_from_slice(&((value - eye[axis]) as f32).to_le_bytes());
                }
                self.staging.extend_from_slice(&[
                    triangle.color[0],
                    triangle.color[1],
                    triangle.color[2],
                    255,
                ]);
            }
        }
        let bytes = self.staging.len() as u64;
        if self.vertices.as_ref().is_none_or(|b| b.size() < bytes) {
            self.vertices = Some(gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("aether-3d-vertices"),
                size: bytes.next_power_of_two().max(4096),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        let vertices = self.vertices.as_ref().expect("allocated");
        gpu.queue.write_buffer(vertices, 0, &self.staging);
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("aether-3d-scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &textures.color,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &textures.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            let s = region.samples;
            pass.set_scissor_rect(
                s[0] as u32,
                s[1] as u32,
                (s[2] - s[0]) as u32,
                (s[3] - s[1]) as u32,
            );
            pass.set_pipeline(&gpu.scene);
            pass.set_bind_group(0, &textures.scene_group, &[]);
            pass.set_vertex_buffer(0, vertices.slice(..bytes));
            pass.draw(0..triangles.len() as u32 * 3, 0..1);
        }
        let resolved = textures
            .resolved
            .create_view(&wgpu::TextureViewDescriptor::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("aether-3d-resolve"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &resolved,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&gpu.resolve);
            pass.set_bind_group(0, &textures.resolve_group, &[]);
            pass.draw(0..3, 0..1);
        }
        encoder.copy_texture_to_buffer(
            textures.resolved.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &textures.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(textures.row_bytes),
                    rows_per_image: Some(size[1]),
                },
            },
            wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
        );
        gpu.queue.submit([encoder.finish()]);
        let slice = textures.readback.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| anyhow!("GPU poll failed: {e}"))?;
        receiver
            .recv()
            .context("GPU readback was dropped")?
            .map_err(|e| anyhow!("GPU readback failed: {e}"))?;
        {
            let data = slice
                .get_mapped_range()
                .map_err(|e| anyhow!("GPU readback could not be mapped: {e:?}"))?;
            let width = size[0] as usize;
            for y in 0..size[1] as usize {
                let start = y * textures.row_bytes as usize;
                let row = &data[start..start + width * 4];
                let (pixels, _) = row.as_chunks::<4>();
                if pixels.iter().any(|p| p[3] != 0) {
                    sink(region.rect[0], region.rect[1] + y, pixels);
                }
            }
        }
        textures.readback.unmap();
        Ok(())
    }
}

fn f32_bytes(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

impl Textures {
    fn new(gpu: &Gpu, size: [u32; 2], ss: u32) -> Self {
        let device = &gpu.device;
        let texture = |label, [w, h]: [u32; 2], format, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let samples = [size[0] * ss, size[1] * ss];
        let color = texture(
            "aether-3d-samples",
            samples,
            COLOR,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let depth = texture(
            "aether-3d-depth",
            samples,
            DEPTH,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let resolved = texture(
            "aether-3d-resolved",
            size,
            COLOR,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let row_bytes = (size[0] * 4).div_ceil(256) * 256;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("aether-3d-readback"),
            size: u64::from(row_bytes) * u64::from(size[1]),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("aether-3d-clip"),
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let factor = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("aether-3d-factor"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        gpu.queue.write_buffer(
            &factor,
            0,
            &[ss, 0, 0, 0]
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect::<Vec<u8>>(),
        );
        let color_view = color.create_view(&wgpu::TextureViewDescriptor::default());
        let scene_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &gpu.scene_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        let resolve_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &gpu.resolve_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&color_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: factor.as_entire_binding(),
                },
            ],
        });
        Self {
            color: color_view,
            depth: depth.create_view(&wgpu::TextureViewDescriptor::default()),
            resolved,
            readback,
            uniforms,
            _factor: factor,
            scene_group,
            resolve_group,
            row_bytes,
        }
    }
}
