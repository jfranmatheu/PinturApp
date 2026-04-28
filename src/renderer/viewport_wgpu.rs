use crate::io::mesh_loader::MeshData;
use crate::renderer::gpu_paint::GpuAlbedoSnapshot;
use bytemuck::{Pod, Zeroable};
use eframe::egui;
use eframe::wgpu;
use glam::{Mat4, Vec3, vec3};
use std::sync::Mutex;

const SCENE_WGSL: &str = r#"
struct Camera {
    mvp: mat4x4<f32>,
};

struct AlbedoParams {
    tex_size: vec2<f32>,
    lighting_enabled: f32,
    _pad: f32,
};

struct VSIn {
    @location(0) pos: vec3<f32>,
    @location(1) uv: vec2<f32>,
};

struct VSOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) world_pos: vec3<f32>,
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
    out.world_pos = v.pos;
    return out;
}

@fragment
fn fs_main(in_f: VSOut, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    let w = max(u32(albedo.tex_size.x), 1u);
    let h = max(u32(albedo.tex_size.y), 1u);
    let u = clamp(in_f.uv.x, 0.0, 1.0);
    let v = clamp(in_f.uv.y, 0.0, 1.0);
    let x = min(u32(u * f32(w - 1u)), w - 1u);
    let y = min(u32(v * f32(h - 1u)), h - 1u);
    let idx = y * w + x;
    let base = unpack_rgba8(pixels[idx]);
    if albedo.lighting_enabled < 0.5 {
        return base;
    }
    var n = normalize(cross(dpdx(in_f.world_pos), dpdy(in_f.world_pos)));
    if !front_facing {
        n = -n;
    }
    let light_dir = normalize(vec3<f32>(0.55, 0.75, 0.35));
    let lambert = max(dot(n, light_dir), 0.0);
    let shade = 0.22 + 0.78 * lambert;
    return vec4<f32>(base.rgb * shade, base.a);
}
"#;

const COMPOSITE_WGSL: &str = r#"
struct VSOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@group(0) @binding(0) var scene_tex: texture_2d<f32>;
@group(0) @binding(1) var scene_sampler: sampler;

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VSOut {
    var out: VSOut;
    let p = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(-1.0,  1.0),
        vec2<f32>( 3.0,  1.0)
    );
    let xy = p[vid];
    out.pos = vec4<f32>(xy, 0.0, 1.0);
    out.uv = vec2<f32>(xy.x * 0.5 + 0.5, 1.0 - (xy.y * 0.5 + 0.5));
    return out;
}

@fragment
fn fs_main(in_f: VSOut) -> @location(0) vec4<f32> {
    return textureSample(scene_tex, scene_sampler, in_f.uv);
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
    lighting_enabled: f32,
    _pad: f32,
}

struct Prepared {
    _scene_color_texture: wgpu::Texture,
    _scene_depth_texture: wgpu::Texture,
    composite_pipeline: wgpu::RenderPipeline,
    composite_bind_group: wgpu::BindGroup,
}

struct ViewportCallback {
    vertices: Vec<GpuVertex>,
    indices: Vec<u32>,
    snapshot: GpuAlbedoSnapshot,
    camera: CameraUniform,
    target_format: wgpu::TextureFormat,
    lighting_enabled: bool,
    viewport_points: [f32; 2],
    prepared: Mutex<Option<Prepared>>,
}

impl egui_wgpu::CallbackTrait for ViewportCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        _callback_resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        use wgpu::util::DeviceExt as _;

        let viewport_w = ((self.viewport_points[0].max(1.0) * screen_descriptor.pixels_per_point).round()
            as u32)
            .max(1);
        let viewport_h = ((self.viewport_points[1].max(1.0) * screen_descriptor.pixels_per_point).round()
            as u32)
            .max(1);

        let scene_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pinturapp-viewport-scene-shader"),
            source: wgpu::ShaderSource::Wgsl(SCENE_WGSL.into()),
        });
        let scene_bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pinturapp-viewport-scene-bind-layout"),
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
        let scene_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pinturapp-viewport-scene-pipeline-layout"),
            bind_group_layouts: &[Some(&scene_bind_layout)],
            immediate_size: 0,
        });
        let scene_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("pinturapp-viewport-scene-pipeline"),
            layout: Some(&scene_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &scene_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GpuVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2],
                }],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &scene_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: self.target_format,
                    blend: Some(wgpu::BlendState::REPLACE),
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
            lighting_enabled: if self.lighting_enabled { 1.0 } else { 0.0 },
            _pad: 0.0,
        };
        let albedo_params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("pinturapp-viewport-albedo-params-buffer"),
            contents: bytemuck::bytes_of(&albedo_params),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let scene_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pinturapp-viewport-scene-bind-group"),
            layout: &scene_bind_layout,
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

        let scene_color_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("pinturapp-viewport-scene-color"),
            size: wgpu::Extent3d {
                width: viewport_w,
                height: viewport_h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.target_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let scene_color_view = scene_color_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let scene_depth_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("pinturapp-viewport-scene-depth"),
            size: wgpu::Extent3d {
                width: viewport_w,
                height: viewport_h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let scene_depth_view = scene_depth_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("pinturapp-viewport-scene-encoder"),
        });
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("pinturapp-viewport-scene-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &scene_color_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.1,
                            g: 0.12,
                            b: 0.15,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &scene_depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rpass.set_pipeline(&scene_pipeline);
            rpass.set_bind_group(0, &scene_bind_group, &[]);
            rpass.set_vertex_buffer(0, vertex_buffer.slice(..));
            rpass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            rpass.draw_indexed(0..self.indices.len() as u32, 0, 0..1);
        }

        let composite_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pinturapp-viewport-composite-shader"),
            source: wgpu::ShaderSource::Wgsl(COMPOSITE_WGSL.into()),
        });
        let composite_bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pinturapp-viewport-composite-bind-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let composite_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pinturapp-viewport-composite-pipeline-layout"),
            bind_group_layouts: &[Some(&composite_bind_layout)],
            immediate_size: 0,
        });
        let composite_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("pinturapp-viewport-composite-pipeline"),
            layout: Some(&composite_layout),
            vertex: wgpu::VertexState {
                module: &composite_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &composite_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: self.target_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let composite_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("pinturapp-viewport-composite-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let composite_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pinturapp-viewport-composite-bind-group"),
            layout: &composite_bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&scene_color_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&composite_sampler),
                },
            ],
        });

        if let Ok(mut prepared) = self.prepared.lock() {
            *prepared = Some(Prepared {
                _scene_color_texture: scene_color_texture,
                _scene_depth_texture: scene_depth_texture,
                composite_pipeline,
                composite_bind_group,
            });
        }
        vec![encoder.finish()]
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
        render_pass.set_pipeline(&prepared.composite_pipeline);
        render_pass.set_bind_group(0, &prepared.composite_bind_group, &[]);
        render_pass.draw(0..3, 0..1);
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
    lighting_enabled: bool,
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
        lighting_enabled,
        viewport_points: [rect.width(), rect.height()],
        prepared: Mutex::new(None),
    };
    painter.add(egui_wgpu::Callback::new_paint_callback(rect, callback));
    true
}
