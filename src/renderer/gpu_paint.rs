use crate::renderer::{BrushBlendMode, BrushDispatch, BrushFalloff, BrushInput, UvCoverageCache};
use bytemuck::{Pod, Zeroable};
use eframe::wgpu;
use image::RgbaImage;
use std::sync::{Arc, OnceLock};

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct StampGpu {
    center_uv: [f32; 2],
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
            let instance =
                wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
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
}

struct Stamp {
    center_uv: vec2<f32>,
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
    if mode == 0u { // smooth
        return t * t * (3.0 - 2.0 * t);
    }
    if mode == 1u { // sphere
        return sqrt(max(0.0, 1.0 - (1.0 - t) * (1.0 - t)));
    }
    if mode == 2u { // root
        return sqrt(t);
    }
    if mode == 3u { // sharp
        return t * t;
    }
    if mode == 5u { // constant
        return select(0.0, 1.0, t > 0.0);
    }
    return t; // linear
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
    let idx = gid.x;
    let pixel_count = params.width * params.height;
    if idx >= pixel_count {
        return;
    }
    if coverage[idx] == 0u {
        return;
    }

    let x = idx % params.width;
    let y = idx / params.width;
    let denom_x = max(f32(params.width - 1u), 1.0);
    let denom_y = max(f32(params.height - 1u), 1.0);
    let uv = vec2<f32>(f32(x) / denom_x, f32(y) / denom_y);
    var dst = unpack_rgba8(pixels[idx]);

    for (var i: u32 = 0u; i < params.stamp_count; i = i + 1u) {
        let s = stamps[i];
        let dx = (uv.x - s.center_uv.x) * denom_x;
        let dy = (uv.y - s.center_uv.y) * denom_y;
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

pub fn try_paint_stamps_gpu(
    texture: &mut RgbaImage,
    stamps_input: &[(BrushInput, BrushDispatch)],
    uv_cache: &mut UvCoverageCache,
    mesh: &crate::io::mesh_loader::MeshData,
) -> bool {
    if stamps_input.is_empty() {
        return false;
    }
    let Some(runtime) = gpu_runtime() else {
        return false;
    };
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

    let mut stamps = Vec::<StampGpu>::with_capacity(stamps_input.len());
    for (input, dispatch) in stamps_input {
        let mut color = dispatch.color;
        color[3] = ((color[3] as f32)
            * dispatch.strength.clamp(0.0, 1.0)
            * dispatch.pressure.clamp(0.0, 1.0))
        .round() as u8;
        stamps.push(StampGpu {
            center_uv: input.center_uv,
            radius_texels: dispatch.radius_px.max(0.5),
            strength: 1.0,
            color_rgba: pack_color_rgba(color),
            blend_mode: blend_to_u32(dispatch.blend_mode),
            falloff_mode: falloff_to_u32(dispatch.falloff),
            _pad0: 0,
        });
    }

    let params = ParamsGpu {
        width: width as u32,
        height: height as u32,
        stamp_count: stamps.len() as u32,
        _pad0: 0,
    };

    let pixel_bytes = bytemuck::cast_slice(&pixels_u32);
    let coverage_bytes = bytemuck::cast_slice(&coverage_u32);
    let stamps_bytes = bytemuck::cast_slice(&stamps);
    let params_bytes = bytemuck::bytes_of(&params);

    let device = runtime.device.clone();
    let queue = runtime.queue.clone();

    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("pinturapp-gpu-paint-shader"),
        source: wgpu::ShaderSource::Wgsl(GPU_PAINT_WGSL.into()),
    });

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

    let stamps_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("pinturapp-gpu-stamps"),
        size: stamps_bytes.len().max(16) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&stamps_buffer, 0, stamps_bytes);

    let params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("pinturapp-gpu-params"),
        size: params_bytes.len() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&params_buffer, 0, params_bytes);

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

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("pinturapp-gpu-paint-encoder"),
    });
    {
        let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("pinturapp-gpu-paint-pass"),
            timestamp_writes: None,
        });
        cpass.set_pipeline(&pipeline);
        cpass.set_bind_group(0, &bind_group, &[]);
        let workgroups = (pixel_count as u32).div_ceil(64);
        cpass.dispatch_workgroups(workgroups, 1, 1);
    }

    let readback_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("pinturapp-gpu-paint-readback"),
        size: pixel_bytes.len() as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    encoder.copy_buffer_to_buffer(
        &pixels_buffer,
        0,
        &readback_buffer,
        0,
        pixel_bytes.len() as u64,
    );
    queue.submit(std::iter::once(encoder.finish()));

    let slice = readback_buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
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
    if out_u32.len() != pixel_count {
        drop(mapped);
        readback_buffer.unmap();
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
    readback_buffer.unmap();
    true
}
