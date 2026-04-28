use eframe::egui::{self, TextureHandle};
use glam::Vec3;
use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

mod app;
mod io;
mod renderer;
mod ui;

use app::{PendingLoadAction, default_storage_dir, load_recent_projects};
use image::RgbaImage;
use io::mesh_loader::MeshData;
use renderer::{draw_mesh_wireframe, pick_uv_at_screen, render_textured_preview};

struct PinturappUi {
    loaded_mesh: Option<MeshData>,
    last_loaded_path: Option<PathBuf>,
    loaded_texture_path: Option<PathBuf>,
    last_error: Option<String>,
    orbit_yaw: f32,
    orbit_pitch: f32,
    orbit_distance: f32,
    mesh_center: Vec3,
    mesh_fit_scale: f32,
    albedo_texture: Option<RgbaImage>,
    preview_texture: Option<TextureHandle>,
    brush_radius_px: f32,
    brush_color: egui::Color32,
    undo_stack: VecDeque<RgbaImage>,
    redo_stack: VecDeque<RgbaImage>,
    is_painting_stroke: bool,
    current_project_path: Option<PathBuf>,
    recent_projects: Vec<PathBuf>,
    storage_dir: PathBuf,
    autosave_path: PathBuf,
    last_autosave_at: Instant,
    last_autosave_ok_at: Option<Instant>,
    is_dirty: bool,
    pending_load_action: Option<PendingLoadAction>,
    show_discard_confirm: bool,
    show_autosave_recovery_prompt: bool,
    show_welcome_overlay: bool,
    theme_applied: bool,
}

impl Default for PinturappUi {
    fn default() -> Self {
        let storage_dir = default_storage_dir();
        let _ = fs::create_dir_all(storage_dir.join("unpacked"));
        let recent_projects = load_recent_projects(&storage_dir);
        let autosave_path = storage_dir.join("autosave.pinturaproj");
        let show_autosave_recovery_prompt = autosave_path.exists();
        Self {
            loaded_mesh: None,
            last_loaded_path: None,
            loaded_texture_path: None,
            last_error: None,
            orbit_yaw: 0.5,
            orbit_pitch: 0.25,
            orbit_distance: 3.0,
            mesh_center: Vec3::ZERO,
            mesh_fit_scale: 1.0,
            albedo_texture: None,
            preview_texture: None,
            brush_radius_px: 12.0,
            brush_color: egui::Color32::from_rgba_unmultiplied(255, 90, 90, 255),
            undo_stack: VecDeque::new(),
            redo_stack: VecDeque::new(),
            is_painting_stroke: false,
            current_project_path: None,
            recent_projects,
            storage_dir,
            autosave_path,
            last_autosave_at: Instant::now(),
            last_autosave_ok_at: None,
            is_dirty: false,
            pending_load_action: None,
            show_discard_confirm: false,
            show_autosave_recovery_prompt,
            show_welcome_overlay: true,
            theme_applied: false,
        }
    }
}

impl eframe::App for PinturappUi {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.apply_modern_theme(ui.ctx());
        self.handle_shortcuts(ui.ctx());
        self.show_toolbar_panel(ui);
        egui::Panel::bottom("status_bar").show_inside(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                let mesh_status = self
                    .loaded_mesh
                    .as_ref()
                    .map(|m| {
                        m.source_path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| "Loaded mesh".to_string())
                    })
                    .unwrap_or_else(|| "No mesh".to_string());
                let texture_status = self
                    .loaded_texture_path
                    .as_ref()
                    .map(|p| {
                        p.file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| "Texture loaded".to_string())
                    })
                    .unwrap_or_else(|| "No texture".to_string());
                ui.label(format!(
                    "Project: {}",
                    self.current_project_path
                        .as_ref()
                        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
                        .unwrap_or_else(|| "Untitled".to_string())
                ));
                ui.separator();
                ui.label(format!("Mesh: {mesh_status}"));
                ui.separator();
                ui.label(format!("Texture: {texture_status}"));
                ui.separator();
                let dirty_label = if self.is_dirty {
                    "State: Unsaved changes"
                } else {
                    "State: Saved"
                };
                ui.label(dirty_label);
                ui.separator();
                ui.label(format!("Autosave: {}", self.autosave_status_text()));
            });
        });

        egui::Panel::left("layers")
            .resizable(true)
            .default_size(220.0)
            .show_inside(ui, |ui| {
                ui.heading("Layers");
                ui.separator();
                Self::panel_card().show(ui, |ui| {
                    ui.strong("Viewport Controls");
                    ui.small("LMB Drag: Paint");
                    ui.small("RMB Drag: Orbit");
                    ui.small("Scroll: Zoom");
                });
                ui.add_space(8.0);
                Self::panel_card().show(ui, |ui| {
                    ui.strong("Material");
                    if let Some(path) = &self.loaded_texture_path {
                        ui.small(format!("Texture: {}", path.display()));
                    } else {
                        ui.small("Texture: UV gradient fallback");
                    }
                });
                ui.add_space(8.0);
                Self::panel_card().show(ui, |ui| {
                    ui.strong("Brush");
                    if ui
                        .add(egui::Slider::new(&mut self.brush_radius_px, 1.0..=64.0).text("Radius"))
                        .changed()
                    {
                        self.is_dirty = true;
                    }
                    if ui.color_edit_button_srgba(&mut self.brush_color).changed() {
                        self.is_dirty = true;
                    }
                    ui.small(format!("Undo: {}", self.undo_stack.len()));
                    ui.small(format!("Redo: {}", self.redo_stack.len()));
                });
                if let Some(path) = &self.current_project_path {
                    ui.add_space(8.0);
                    Self::panel_card().show(ui, |ui| {
                        ui.strong("Project");
                        ui.small(path.display().to_string());
                    });
                }
            });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            ui.heading("3D Viewport");
            if let Some(mesh) = self.loaded_mesh.clone() {
                ui.label(format!("Loaded: {}", mesh.source_path.display()));
                ui.label(format!("Vertices: {}", mesh.vertices.len()));
                ui.label(format!("Triangles: {}", mesh.indices.len() / 3));
                if let Some(v0) = mesh.vertices.first() {
                    ui.label(format!(
                        "Sample Vertex: pos=({:.3}, {:.3}, {:.3}) uv=({:.3}, {:.3})",
                        v0.position[0], v0.position[1], v0.position[2], v0.uv[0], v0.uv[1]
                    ));
                }

                ui.separator();
                let available = ui.available_size_before_wrap();
                let viewport_size = egui::vec2(available.x.max(240.0), available.y.max(240.0));
                let (response, painter) = ui.allocate_painter(viewport_size, egui::Sense::drag());
                let rect = response.rect;

                painter.rect_filled(rect, 4.0, egui::Color32::from_rgb(26, 29, 35));

                if response.dragged_by(egui::PointerButton::Secondary) {
                    let delta = ui.ctx().input(|i| i.pointer.delta());
                    self.orbit_yaw -= delta.x * 0.01;
                    self.orbit_pitch = (self.orbit_pitch + delta.y * 0.01).clamp(-1.4, 1.4);
                    self.is_dirty = true;
                }

                if response.hovered() {
                    let scroll = ui.ctx().input(|i| i.smooth_scroll_delta.y);
                    if scroll.abs() > f32::EPSILON {
                        let zoom_factor = (1.0_f32 - scroll * 0.0015_f32).clamp(0.80_f32, 1.25_f32);
                        self.orbit_distance = (self.orbit_distance * zoom_factor).clamp(0.25, 50.0);
                        self.is_dirty = true;
                    }
                }

                let img_w = rect.width().max(1.0).round() as usize;
                let img_h = rect.height().max(1.0).round() as usize;
                let is_painting_now = response.hovered()
                    && ui
                        .ctx()
                        .input(|i| i.pointer.button_down(egui::PointerButton::Primary));
                if is_painting_now && !self.is_painting_stroke {
                    self.begin_paint_stroke();
                    self.is_painting_stroke = true;
                } else if !is_painting_now {
                    self.is_painting_stroke = false;
                }

                if is_painting_now {
                    if let Some(pointer_pos) = ui.ctx().input(|i| i.pointer.interact_pos()) {
                        if !rect.contains(pointer_pos) {
                            self.is_painting_stroke = false;
                        }
                        let sx = (pointer_pos.x - rect.left()).clamp(0.0, rect.width() - 1.0);
                        let sy = (pointer_pos.y - rect.top()).clamp(0.0, rect.height() - 1.0);
                        if let Some(uv) = pick_uv_at_screen(
                            &mesh,
                            self.mesh_center,
                            self.mesh_fit_scale,
                            self.orbit_yaw,
                            self.orbit_pitch,
                            self.orbit_distance,
                            [img_w, img_h],
                            [sx, sy],
                        ) {
                            self.paint_at_uv(uv);
                        }
                    }
                }

                let image = render_textured_preview(
                    &mesh,
                    self.mesh_center,
                    self.mesh_fit_scale,
                    self.orbit_yaw,
                    self.orbit_pitch,
                    self.orbit_distance,
                    [img_w, img_h],
                    self.albedo_texture.as_ref(),
                );
                self.update_preview_texture(ui.ctx(), image);
                if let Some(texture) = &self.preview_texture {
                    painter.image(
                        texture.id(),
                        rect,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                }

                draw_mesh_wireframe(
                    &painter,
                    rect,
                    &mesh,
                    self.mesh_center,
                    self.mesh_fit_scale,
                    self.orbit_yaw,
                    self.orbit_pitch,
                    self.orbit_distance,
                );
            } else {
                self.is_painting_stroke = false;
                ui.label("No mesh loaded yet. Use 'Load OBJ' to import a model with UVs.");
            }

            if let Some(path) = &self.last_loaded_path {
                ui.label(format!("Last file: {}", path.display()));
            }

            if let Some(err) = &self.last_error {
                ui.colored_label(egui::Color32::RED, format!("Load error: {err}"));
            }
        });
        self.show_welcome_overlay_if_needed(ui.ctx());
        self.show_discard_confirm_dialog(ui.ctx());
        self.show_autosave_recovery_dialog(ui.ctx());
        self.maybe_autosave();
    }
}

impl PinturappUi {
    const MAX_HISTORY: usize = 20;
}

fn main() -> eframe::Result<()> {
    env_logger::init();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Pinturapp - 3D Texture Painter")
            .with_inner_size([1200.0, 800.0]),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };

    eframe::run_native(
        "Pinturapp - 3D Texture Painter",
        options,
        Box::new(|_cc| Ok(Box::<PinturappUi>::default())),
    )
}
