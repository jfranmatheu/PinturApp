use crate::io::mesh_loader::MeshData;
use crate::renderer::gpu_paint::GpuAlbedoSnapshot;
use bytemuck::{Pod, Zeroable};
use eframe::egui;
use eframe::wgpu;
use glam::{Mat4, Vec3, vec3};
use std::sync::Mutex;

const VIEWPORT_WGSL: &str = r#"
struct Camera {
    mvp: mat4x4<f32>,
};

struct AlbedoParams {
    tex_size: vec2<f32>,
    _pad: vec2<f32>,
};

struct VSIn {
    @location(0) pos: vec3<f32>,
    @location(1) uv: vec2<f32>,
};

struct VSOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var<storage, read> pixels: array<u32>;
@group(0) @binding(2) var<uniform> albedo: AlbedoParams;

fn unpack_rgba8(v: u32) -> vec4<f32> {
    let r: f32 = f32(v & 0xffu) / 255.0;
    let g: f32 = f32((v >> 8u) & 0xffu) / 255.0;
    let b: f32 = f32((v >> 16u) & 0xffu) / 255.0;
    let a: f32 = f32((v >> 24u) & 0xffu) / 255.0;
    return vec4<f32>(r, g, b, a);
}

@vertex
fn vs_main(v: VSIn) -> VSOut {
    var out: VSOut;
    out.pos = camera.mvp * vec4<f32>(v.pos, 1.0);
    out.uv = v.uv;
    return out;
}

@fragment
fn fs_main(in_f: VSOut) -> @location(0) vec4<f32> {
    let w = max(u32(albedo.tex_size.x), 1u);
    let h = max(u32(albedo.tex_size.y), 1u);
    let u = clamp(in_f.uv.x, 0.0, 1.0);
    let v = clamp(in_f.uv.y, 0.0, 1.0);
    let x = min(u32(u * f32(w - 1u)), w - 1u);
    let y = min(u32(v * f32(h - 1u)), h - 1u);
    let idx = y * w + x;
    return unpack_rgba8(pixels[idx]);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuVertex {
    pos: [f32; 3],
    uv: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CameraUniform {
    mvp: [[f32; 4]; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct AlbedoParams {
    tex_size: [f32; 2],
    _pad: [f32; 2],
}

struct Prepared {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
}

struct ViewportCallback {
    vertices: Vec<GpuVertex>,
    indices: Vec<u32>,
    snapshot: GpuAlbedoSnapshot,
    camera: CameraUniform,
    target_format: wgpu::TextureFormat,
    prepared: Mutex<Option<Prepared>>,
}

impl egui_wgpu::CallbackTrait for ViewportCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        _callback_resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        use wgpu::util::DeviceExt as _;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pinturapp-viewport-wgpu-shader"),
            source: wgpu::ShaderSource::Wgsl(VIEWPORT_WGSL.into()),
        });
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pinturapp-viewport-wgpu-bind-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
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
            label: Some("pinturapp-viewport-wgpu-pipeline-layout"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("pinturapp-viewport-wgpu-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GpuVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2],
                }],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: self.target_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("pinturapp-viewport-camera-buffer"),
            contents: bytemuck::bytes_of(&self.camera),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let albedo_params = AlbedoParams {
            tex_size: [self.snapshot.width as f32, self.snapshot.height as f32],
            _pad: [0.0, 0.0],
        };
        let albedo_params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("pinturapp-viewport-albedo-params-buffer"),
            contents: bytemuck::bytes_of(&albedo_params),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pinturapp-viewport-wgpu-bind-group"),
            layout: &bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: camera_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.snapshot.pixels_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: albedo_params_buffer.as_entire_binding(),
                },
            ],
        });
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("pinturapp-viewport-vertex-buffer"),
            contents: bytemuck::cast_slice(&self.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("pinturapp-viewport-index-buffer"),
            contents: bytemuck::cast_slice(&self.indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        if let Ok(mut prepared) = self.prepared.lock() {
            *prepared = Some(Prepared {
                pipeline,
                bind_group,
                vertex_buffer,
                index_buffer,
                index_count: self.indices.len() as u32,
            });
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        _callback_resources: &egui_wgpu::CallbackResources,
    ) {
        let Ok(prepared) = self.prepared.lock() else {
            return;
        };
        let Some(prepared) = prepared.as_ref() else {
            return;
        };
        render_pass.set_pipeline(&prepared.pipeline);
        render_pass.set_bind_group(0, &prepared.bind_group, &[]);
        render_pass.set_vertex_buffer(0, prepared.vertex_buffer.slice(..));
        render_pass.set_index_buffer(prepared.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        render_pass.draw_indexed(0..prepared.index_count, 0, 0..1);
    }
}

fn viewport_mvp(center: Vec3, fit_scale: f32, yaw: f32, pitch: f32, distance: f32, aspect: f32) -> Mat4 {
    let target = vec3(0.0, 0.0, 0.0);
    let eye = target
        + vec3(
            distance * yaw.cos() * pitch.cos(),
            distance * pitch.sin(),
            distance * yaw.sin() * pitch.cos(),
        );
    let model = Mat4::from_scale(Vec3::splat(fit_scale)) * Mat4::from_translation(-center);
    let view = Mat4::look_at_rh(eye, target, Vec3::Y);
    let proj = Mat4::perspective_rh_gl(45.0_f32.to_radians(), aspect.max(0.01), 0.01, 200.0);
    proj * view * model
}

pub fn enqueue_gpu_viewport(
    painter: &egui::Painter,
    rect: egui::Rect,
    mesh: &MeshData,
    center: Vec3,
    fit_scale: f32,
    yaw: f32,
    pitch: f32,
    distance: f32,
    snapshot: &GpuAlbedoSnapshot,
    target_format: wgpu::TextureFormat,
) -> bool {
    if mesh.indices.is_empty() || mesh.vertices.is_empty() {
        return false;
    }
    let vertices = mesh
        .vertices
        .iter()
        .map(|v| GpuVertex {
            pos: v.position,
            uv: v.uv,
        })
        .collect::<Vec<_>>();
    let indices = mesh.indices.clone();
    let aspect = (rect.width() / rect.height().max(1.0)).max(0.01);
    let mvp = viewport_mvp(center, fit_scale, yaw, pitch, distance, aspect).to_cols_array_2d();
    let callback = ViewportCallback {
        vertices,
        indices,
        snapshot: snapshot.clone(),
        camera: CameraUniform { mvp },
        target_format,
        prepared: Mutex::new(None),
    };
    painter.add(egui_wgpu::Callback::new_paint_callback(rect, callback));
    true
}
