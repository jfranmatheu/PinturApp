use crate::io::mesh_loader::MeshData;
use crate::renderer::{BrushBlendMode, BrushDispatch, BrushFalloff, BrushInput, UvCoverageCache};
use bytemuck::{Pod, Zeroable};
use eframe::wgpu;
use image::RgbaImage;
use std::sync::mpsc;
use std::sync::{Arc, OnceLock};

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct StampGpu {
    center_texel: [f32; 2],
    radius_texels: f32,
    strength: f32,
    color_rgba: u32,
    blend_mode: u32,
    falloff_mode: u32,
    _pad0: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct ParamsGpu {
    width: u32,
    height: u32,
    stamp_count: u32,
    _pad0: u32,
    min_x: u32,
    min_y: u32,
    max_x: u32,
    max_y: u32,
}

#[derive(Clone)]
struct GpuRuntime {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
}

static GPU_RUNTIME: OnceLock<Option<GpuRuntime>> = OnceLock::new();

fn gpu_runtime() -> Option<GpuRuntime> {
    GPU_RUNTIME
        .get_or_init(|| {
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
            let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            }))
            .ok()?;
            let req = adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("pinturapp-gpu-paint-device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_defaults(),
                memory_hints: wgpu::MemoryHints::Performance,
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                trace: wgpu::Trace::Off,
            });
            let (device, queue) = pollster::block_on(req).ok()?;
            Some(GpuRuntime {
                device: Arc::new(device),
                queue: Arc::new(queue),
            })
        })
        .clone()
}

fn pack_color_rgba(color: [u8; 4]) -> u32 {
    (color[0] as u32)
        | ((color[1] as u32) << 8)
        | ((color[2] as u32) << 16)
        | ((color[3] as u32) << 24)
}

fn blend_to_u32(mode: BrushBlendMode) -> u32 {
    match mode {
        BrushBlendMode::Normal => 0,
        BrushBlendMode::Multiply => 1,
        BrushBlendMode::Screen => 2,
    }
}

fn falloff_to_u32(mode: BrushFalloff) -> u32 {
    match mode {
        BrushFalloff::Smooth => 0,
        BrushFalloff::Sphere => 1,
        BrushFalloff::Root => 2,
        BrushFalloff::Sharp => 3,
        BrushFalloff::Linear => 4,
        BrushFalloff::Constant => 5,
    }
}

const GPU_PAINT_WGSL: &str = r#"
struct Params {
    width: u32,
    height: u32,
    stamp_count: u32,
    _pad0: u32,
    min_x: u32,
    min_y: u32,
    max_x: u32,
    max_y: u32,
}

struct Stamp {
    center_texel: vec2<f32>,
    radius_texels: f32,
    strength: f32,
    color_rgba: u32,
    blend_mode: u32,
    falloff_mode: u32,
    _pad0: u32,
}

@group(0) @binding(0) var<storage, read_write> pixels: array<u32>;
@group(0) @binding(1) var<storage, read> coverage: array<u32>;
@group(0) @binding(2) var<storage, read> stamps: array<Stamp>;
@group(0) @binding(3) var<uniform> params: Params;

fn unpack_rgba8(v: u32) -> vec4<f32> {
    let r: f32 = f32(v & 0xffu) / 255.0;
    let g: f32 = f32((v >> 8u) & 0xffu) / 255.0;
    let b: f32 = f32((v >> 16u) & 0xffu) / 255.0;
    let a: f32 = f32((v >> 24u) & 0xffu) / 255.0;
    return vec4<f32>(r, g, b, a);
}

fn pack_rgba8(v: vec4<f32>) -> u32 {
    let c = clamp(v, vec4<f32>(0.0), vec4<f32>(1.0));
    let r = u32(round(c.x * 255.0));
    let g = u32(round(c.y * 255.0));
    let b = u32(round(c.z * 255.0));
    let a = u32(round(c.w * 255.0));
    return r | (g << 8u) | (b << 16u) | (a << 24u);
}

fn apply_falloff_curve(t_raw: f32, mode: u32) -> f32 {
    let t = clamp(t_raw, 0.0, 1.0);
    if mode == 0u {
        return t * t * (3.0 - 2.0 * t);
    }
    if mode == 1u {
        return sqrt(max(0.0, 1.0 - (1.0 - t) * (1.0 - t)));
    }
    if mode == 2u {
        return sqrt(t);
    }
    if mode == 3u {
        return t * t;
    }
    if mode == 5u {
        return select(0.0, 1.0, t > 0.0);
    }
    return t;
}

fn blend_channel(src: f32, dst: f32, alpha: f32, mode: u32) -> f32 {
    var blended = src;
    if mode == 1u {
        blended = src * dst;
    } else if mode == 2u {
        blended = 1.0 - (1.0 - src) * (1.0 - dst);
    }
    return blended * alpha + dst * (1.0 - alpha);
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let region_w = (params.max_x - params.min_x) + 1u;
    let region_h = (params.max_y - params.min_y) + 1u;
    let region_count = region_w * region_h;
    let local_idx = gid.x;
    if local_idx >= region_count {
        return;
    }
    let x = params.min_x + (local_idx % region_w);
    let y = params.min_y + (local_idx / region_w);
    let idx = y * params.width + x;
    if idx >= params.width * params.height {
        return;
    }
    if coverage[idx] == 0u {
        return;
    }

    let xf = f32(x);
    let yf = f32(y);
    var dst = unpack_rgba8(pixels[idx]);
    for (var i: u32 = 0u; i < params.stamp_count; i = i + 1u) {
        let s = stamps[i];
        let dx = xf - s.center_texel.x;
        let dy = yf - s.center_texel.y;
        let dist = sqrt(dx * dx + dy * dy);
        if dist > s.radius_texels || s.radius_texels <= 0.0 {
            continue;
        }
        let t = 1.0 - (dist / s.radius_texels);
        let falloff = apply_falloff_curve(t, s.falloff_mode);
        if falloff <= 0.0 {
            continue;
        }
        let src = unpack_rgba8(s.color_rgba);
        let alpha = clamp(src.a * s.strength * falloff, 0.0, 1.0);
        if alpha <= 0.0 {
            continue;
        }
        dst.r = blend_channel(src.r, dst.r, alpha, s.blend_mode);
        dst.g = blend_channel(src.g, dst.g, alpha, s.blend_mode);
        dst.b = blend_channel(src.b, dst.b, alpha, s.blend_mode);
        dst.a = 1.0;
    }
    pixels[idx] = pack_rgba8(dst);
}
"#;

pub struct GpuPaintSession {
    runtime: GpuRuntime,
    width: usize,
    height: usize,
    pixel_count: usize,
    pixels_buffer: wgpu::Buffer,
    coverage_buffer: wgpu::Buffer,
    stamps_buffer: wgpu::Buffer,
    params_buffer: wgpu::Buffer,
    readback_buffer: wgpu::Buffer,
    stamp_capacity: usize,
    bind_layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
    bind_group: wgpu::BindGroup,
    dirty_gpu: bool,
}

impl GpuPaintSession {
    pub fn new(texture: &RgbaImage, uv_cache: &mut UvCoverageCache, mesh: &MeshData) -> Option<Self> {
        let runtime = gpu_runtime()?;
        let device = runtime.device.clone();
        let queue = runtime.queue.clone();
        let width = texture.width().max(1) as usize;
        let height = texture.height().max(1) as usize;
        let pixel_count = width * height;
        uv_cache.ensure_for(mesh, width, height);

        let mut pixels_u32 = Vec::with_capacity(pixel_count);
        for p in texture.pixels() {
            pixels_u32.push(pack_color_rgba(p.0));
        }
        let mut coverage_u32 = Vec::with_capacity(pixel_count);
        for covered in uv_cache.coverage() {
            coverage_u32.push(if *covered { 1_u32 } else { 0_u32 });
        }
        let pixel_bytes = bytemuck::cast_slice(&pixels_u32);
        let coverage_bytes = bytemuck::cast_slice(&coverage_u32);

        let pixels_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pinturapp-gpu-pixels"),
            size: pixel_bytes.len() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&pixels_buffer, 0, pixel_bytes);

        let coverage_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pinturapp-gpu-coverage"),
            size: coverage_bytes.len() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&coverage_buffer, 0, coverage_bytes);

        let params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pinturapp-gpu-params"),
            size: std::mem::size_of::<ParamsGpu>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let readback_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pinturapp-gpu-readback"),
            size: pixel_bytes.len() as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let stamp_capacity = 256;
        let stamps_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pinturapp-gpu-stamps"),
            size: (stamp_capacity * std::mem::size_of::<StampGpu>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pinturapp-gpu-paint-bind-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pinturapp-gpu-paint-pipeline-layout"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pinturapp-gpu-paint-shader"),
            source: wgpu::ShaderSource::Wgsl(GPU_PAINT_WGSL.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("pinturapp-gpu-paint-pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            cache: None,
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pinturapp-gpu-paint-bind-group"),
            layout: &bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: pixels_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: coverage_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: stamps_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: params_buffer.as_entire_binding(),
                },
            ],
        });

        Some(Self {
            runtime,
            width,
            height,
            pixel_count,
            pixels_buffer,
            coverage_buffer,
            stamps_buffer,
            params_buffer,
            readback_buffer,
            stamp_capacity,
            bind_layout,
            pipeline,
            bind_group,
            dirty_gpu: false,
        })
    }

    fn ensure_stamp_capacity(&mut self, needed: usize) {
        if needed <= self.stamp_capacity {
            return;
        }
        let device = self.runtime.device.clone();
        self.stamp_capacity = needed.next_power_of_two().max(256);
        self.stamps_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pinturapp-gpu-stamps-resized"),
            size: (self.stamp_capacity * std::mem::size_of::<StampGpu>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pinturapp-gpu-paint-bind-group-resized"),
            layout: &self.bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.pixels_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.coverage_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.stamps_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.params_buffer.as_entire_binding(),
                },
            ],
        });
    }

    pub fn apply_stamps(&mut self, stamps_input: &[(BrushInput, BrushDispatch)]) -> bool {
        if stamps_input.is_empty() {
            return false;
        }
        self.ensure_stamp_capacity(stamps_input.len());

        let tex_w = self.width.max(1) as f32;
        let tex_h = self.height.max(1) as f32;
        let mut stamps = Vec::<StampGpu>::with_capacity(stamps_input.len());
        let mut min_x = self.width as i32 - 1;
        let mut min_y = self.height as i32 - 1;
        let mut max_x = 0_i32;
        let mut max_y = 0_i32;

        for (input, dispatch) in stamps_input {
            let cx = input.center_uv[0].clamp(0.0, 1.0) * (tex_w - 1.0);
            let cy = input.center_uv[1].clamp(0.0, 1.0) * (tex_h - 1.0);
            let radius = dispatch.radius_px.max(0.5);
            let mut color = dispatch.color;
            color[3] = ((color[3] as f32)
                * dispatch.strength.clamp(0.0, 1.0)
                * dispatch.pressure.clamp(0.0, 1.0))
            .round() as u8;

            min_x = min_x.min((cx - radius).floor().max(0.0) as i32);
            min_y = min_y.min((cy - radius).floor().max(0.0) as i32);
            max_x = max_x.max((cx + radius).ceil().min((self.width - 1) as f32) as i32);
            max_y = max_y.max((cy + radius).ceil().min((self.height - 1) as f32) as i32);

            stamps.push(StampGpu {
                center_texel: [cx, cy],
                radius_texels: radius,
                strength: 1.0,
                color_rgba: pack_color_rgba(color),
                blend_mode: blend_to_u32(dispatch.blend_mode),
                falloff_mode: falloff_to_u32(dispatch.falloff),
                _pad0: 0,
            });
        }

        if min_x > max_x || min_y > max_y {
            return false;
        }
        let params = ParamsGpu {
            width: self.width as u32,
            height: self.height as u32,
            stamp_count: stamps.len() as u32,
            _pad0: 0,
            min_x: min_x as u32,
            min_y: min_y as u32,
            max_x: max_x as u32,
            max_y: max_y as u32,
        };

        let queue = self.runtime.queue.clone();
        let device = self.runtime.device.clone();
        queue.write_buffer(&self.stamps_buffer, 0, bytemuck::cast_slice(&stamps));
        queue.write_buffer(&self.params_buffer, 0, bytemuck::bytes_of(&params));

        let region_w = (params.max_x - params.min_x + 1) as u64;
        let region_h = (params.max_y - params.min_y + 1) as u64;
        let region_count = (region_w * region_h) as u32;
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("pinturapp-gpu-paint-encoder"),
        });
        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("pinturapp-gpu-paint-pass"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(&self.pipeline);
            cpass.set_bind_group(0, &self.bind_group, &[]);
            cpass.dispatch_workgroups(region_count.div_ceil(64), 1, 1);
        }
        queue.submit(std::iter::once(encoder.finish()));
        self.dirty_gpu = true;
        true
    }

    pub fn readback_if_dirty(&mut self, texture: &mut RgbaImage) -> bool {
        if !self.dirty_gpu {
            return false;
        }
        let device = self.runtime.device.clone();
        let queue = self.runtime.queue.clone();
        let total_bytes = self.pixel_count * std::mem::size_of::<u32>();

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("pinturapp-gpu-readback-encoder"),
        });
        encoder.copy_buffer_to_buffer(
            &self.pixels_buffer,
            0,
            &self.readback_buffer,
            0,
            total_bytes as u64,
        );
        queue.submit(std::iter::once(encoder.finish()));

        let slice = self.readback_buffer.slice(..);
        let (tx, rx) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |res| {
            let _ = tx.send(res.is_ok());
        });
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        let ok = rx.recv().unwrap_or(false);
        if !ok {
            return false;
        }

        let mapped = slice.get_mapped_range();
        let out_u32: &[u32] = bytemuck::cast_slice(&mapped);
        if out_u32.len() != self.pixel_count {
            drop(mapped);
            self.readback_buffer.unmap();
            return false;
        }
        for (i, pixel) in texture.pixels_mut().enumerate() {
            let v = out_u32[i];
            pixel.0 = [
                (v & 0xff) as u8,
                ((v >> 8) & 0xff) as u8,
                ((v >> 16) & 0xff) as u8,
                ((v >> 24) & 0xff) as u8,
            ];
        }
        drop(mapped);
        self.readback_buffer.unmap();
        self.dirty_gpu = false;
        true
    }
}
