use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};
use glam::{vec3, Mat4, Vec3, Vec4};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::collections::VecDeque;
use std::io::{Cursor, Read, Write};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use zip::write::FileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

mod io;
use io::mesh_loader::{load_obj, MeshData};
use image::RgbaImage;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProjectState {
    mesh_entry: Option<String>,
    texture_entry: Option<String>,
    orbit_yaw: f32,
    orbit_pitch: f32,
    orbit_distance: f32,
    brush_radius_px: f32,
    brush_color_rgba: [u8; 4],
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct RecentProjects {
    paths: Vec<String>,
}

#[derive(Debug, Clone)]
enum PendingLoadAction {
    NewProject,
    OpenProjectPicker,
    LoadProject(PathBuf),
    LoadAutosave,
}

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
        egui::Panel::top("toolbar").show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                ui.strong("Pinturapp");
                ui.separator();
                ui.menu_button("File", |ui| {
                    if ui.button("New Project    Ctrl+N").clicked() {
                        self.request_load_action(PendingLoadAction::NewProject);
                        ui.close();
                    }
                    if ui.button("Load OBJ...").clicked() {
                        self.pick_and_load_obj();
                        ui.close();
                    }
                    if ui.button("Load Texture...").clicked() {
                        self.pick_and_load_texture();
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Load Project... Ctrl+O").clicked() {
                        self.request_load_action(PendingLoadAction::OpenProjectPicker);
                        ui.close();
                    }
                    if ui.button("Load Autosave  Ctrl+Shift+O").clicked() {
                        self.request_load_action(PendingLoadAction::LoadAutosave);
                        ui.close();
                    }
                    ui.menu_button("Recent Projects", |ui| {
                        let recent = self.recent_projects.clone();
                        if recent.is_empty() {
                            ui.label("No recent projects");
                        } else {
                            for path in recent.iter().take(10) {
                                let label = path
                                    .file_name()
                                    .map(|n| n.to_string_lossy().to_string())
                                    .unwrap_or_else(|| path.display().to_string());
                                if ui.button(label).clicked() {
                                    self.request_load_action(PendingLoadAction::LoadProject(path.clone()));
                                    ui.close();
                                }
                            }
                        }
                    });
                    ui.separator();
                    if ui.button("Save Texture...").clicked() {
                        self.save_texture_to_file();
                        ui.close();
                    }
                    if ui.button("Save Project    Ctrl+S").clicked() {
                        self.save_project_to_file();
                        ui.close();
                    }
                    if ui.button("Save Project As... Ctrl+Shift+S").clicked() {
                        self.save_project_as_to_file();
                        ui.close();
                    }
                });
                ui.menu_button("Edit", |ui| {
                    if ui.button("Undo   Ctrl+Z").clicked() {
                        self.undo_paint();
                        ui.close();
                    }
                    if ui.button("Redo   Ctrl+Y").clicked() {
                        self.redo_paint();
                        ui.close();
                    }
                    if ui.button("Clear Recent Projects").clicked() {
                        self.clear_recent_projects();
                        ui.close();
                    }
                });
                ui.menu_button("View", |ui| {
                    if ui.button("Reset Camera").clicked() {
                        self.orbit_yaw = 0.5;
                        self.orbit_pitch = 0.25;
                        self.orbit_distance = 3.0;
                        self.is_dirty = true;
                        ui.close();
                    }
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.is_dirty {
                        ui.colored_label(egui::Color32::from_rgb(255, 200, 80), "Unsaved changes");
                    }
                    if let Some(path) = &self.current_project_path {
                        let name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| "Untitled".to_string());
                        ui.label(name);
                    } else {
                        ui.label("Untitled project");
                    }
                });
            });
        });
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

    fn panel_card() -> egui::Frame {
        egui::Frame::default()
            .fill(egui::Color32::from_rgb(24, 30, 40))
            .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(45, 58, 78)))
            .corner_radius(egui::CornerRadius::same(8))
            .inner_margin(egui::Margin::same(10))
    }

    fn welcome_surface() -> egui::Frame {
        egui::Frame::default()
            .fill(egui::Color32::from_rgb(20, 25, 35))
            .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(58, 74, 98)))
            .corner_radius(egui::CornerRadius::same(12))
            .inner_margin(egui::Margin::same(14))
    }

    fn apply_modern_theme(&mut self, ctx: &egui::Context) {
        if self.theme_applied {
            return;
        }
        ctx.set_visuals(egui::Visuals::dark());
        let mut style = (*ctx.global_style()).clone();
        style.spacing.item_spacing = egui::vec2(10.0, 10.0);
        style.spacing.button_padding = egui::vec2(12.0, 8.0);
        style.visuals.panel_fill = egui::Color32::from_rgb(18, 22, 30);
        style.visuals.faint_bg_color = egui::Color32::from_rgb(28, 34, 44);
        style.visuals.extreme_bg_color = egui::Color32::from_rgb(10, 13, 19);
        style.visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(34, 42, 54);
        style.visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(45, 56, 72);
        style.visuals.widgets.active.bg_fill = egui::Color32::from_rgb(58, 74, 98);
        style.visuals.selection.bg_fill = egui::Color32::from_rgb(66, 100, 160);
        ctx.set_global_style(style);
        self.theme_applied = true;
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let (new_project, load_project, load_autosave, save_project, save_project_as, undo, redo) = ctx.input(|i| {
            (
                i.modifiers.command && i.key_pressed(egui::Key::N),
                i.modifiers.command && i.key_pressed(egui::Key::O),
                i.modifiers.command && i.modifiers.shift && i.key_pressed(egui::Key::O),
                i.modifiers.command && i.key_pressed(egui::Key::S) && !i.modifiers.shift,
                i.modifiers.command && i.modifiers.shift && i.key_pressed(egui::Key::S),
                i.modifiers.command && i.key_pressed(egui::Key::Z) && !i.modifiers.shift,
                i.modifiers.command
                    && (i.key_pressed(egui::Key::Y)
                        || (i.modifiers.shift && i.key_pressed(egui::Key::Z))),
            )
        });
        if new_project {
            self.request_load_action(PendingLoadAction::NewProject);
        }
        if load_project {
            self.request_load_action(PendingLoadAction::OpenProjectPicker);
        }
        if load_autosave {
            self.request_load_action(PendingLoadAction::LoadAutosave);
        }
        if save_project {
            self.save_project_to_file();
        }
        if save_project_as {
            self.save_project_as_to_file();
        }
        if undo {
            self.undo_paint();
        }
        if redo {
            self.redo_paint();
        }
    }

    fn show_welcome_overlay_if_needed(&mut self, ctx: &egui::Context) {
        if !self.show_welcome_overlay || self.loaded_mesh.is_some() {
            return;
        }
        if self.show_discard_confirm || self.show_autosave_recovery_prompt {
            return;
        }

        let screen_rect = ctx.content_rect();
        let panel_size = egui::vec2(
            (screen_rect.width() * 0.62).clamp(460.0, 920.0),
            (screen_rect.height() * 0.72).clamp(340.0, 760.0),
        );
        let panel_rect = egui::Rect::from_center_size(screen_rect.center(), panel_size);
        let mut dismiss_overlay = false;

        egui::Area::new("welcome_overlay".into())
            .order(egui::Order::Foreground)
            .fixed_pos(screen_rect.min)
            .interactable(true)
            .show(ctx, |ui| {
                ui.set_min_size(screen_rect.size());
                let full_rect = ui.max_rect();
                let backdrop = ui.allocate_rect(full_rect, egui::Sense::click());
                ui.painter()
                    .rect_filled(full_rect, 0.0, egui::Color32::from_black_alpha(170));

                ui.scope_builder(egui::UiBuilder::new().max_rect(panel_rect), |ui| {
                    Self::welcome_surface().show(ui, |ui| {
                        ui.heading("Pinturapp");
                        ui.label("Start a new project or continue from recent work.");
                        ui.add_space(10.0);
                        ui.columns(2, |cols| {
                            cols[0].label(egui::RichText::new("Actions").strong());
                            cols[0].add_space(6.0);
                            if cols[0]
                                .add_sized(
                                    egui::vec2(cols[0].available_width(), 36.0),
                                    egui::Button::new("New Project  Ctrl+N"),
                                )
                                .clicked()
                            {
                                self.request_load_action(PendingLoadAction::NewProject);
                                dismiss_overlay = true;
                            }
                            cols[0].add_space(8.0);
                            if cols[0]
                                .add_sized(
                                    egui::vec2(cols[0].available_width(), 36.0),
                                    egui::Button::new("Load Project  Ctrl+O"),
                                )
                                .clicked()
                            {
                                self.request_load_action(PendingLoadAction::OpenProjectPicker);
                                dismiss_overlay = true;
                            }
                            if self.autosave_path.exists() {
                                cols[0].add_space(8.0);
                                if cols[0]
                                    .add_sized(
                                        egui::vec2(cols[0].available_width(), 36.0),
                                        egui::Button::new("Load Autosave  Ctrl+Shift+O"),
                                    )
                                    .clicked()
                                {
                                    self.request_load_action(PendingLoadAction::LoadAutosave);
                                    dismiss_overlay = true;
                                }
                            }

                            cols[1].label(egui::RichText::new("Recent Projects").strong());
                            cols[1].add_space(6.0);
                            let recent = self.recent_projects.clone();
                            if recent.is_empty() {
                                cols[1].small("No recent projects yet.");
                            } else {
                                for path in recent.iter().take(10) {
                                    let label = path
                                    .file_name()
                                    .map(|n| n.to_string_lossy().to_string())
                                    .unwrap_or_else(|| path.display().to_string());
                                    let row_rect = cols[1]
                                        .allocate_exact_size(
                                            egui::vec2(cols[1].available_width(), 24.0),
                                            egui::Sense::click(),
                                        )
                                        .0;
                                    let response = cols[1].interact(
                                        row_rect,
                                        cols[1].id().with(path),
                                        egui::Sense::click(),
                                    );

                                    if response.hovered() {
                                        cols[1].painter().rect_filled(
                                            row_rect,
                                            4.0,
                                            egui::Color32::from_rgb(38, 47, 62),
                                        );
                                    }
                                    cols[1].painter().text(
                                        row_rect.left_center() + egui::vec2(8.0, 0.0),
                                        egui::Align2::LEFT_CENTER,
                                        label,
                                        egui::FontId::proportional(14.0),
                                        if response.hovered() {
                                            egui::Color32::from_rgb(245, 250, 255)
                                        } else {
                                            egui::Color32::from_rgb(205, 216, 230)
                                        },
                                    );
                                    if response.clicked() {
                                        self.request_load_action(PendingLoadAction::LoadProject(path.clone()));
                                        dismiss_overlay = true;
                                    }
                                }
                            }
                        });
                    });
                });

                if backdrop.clicked() {
                    if let Some(pos) = backdrop.interact_pointer_pos() {
                        if !panel_rect.contains(pos) {
                            dismiss_overlay = true;
                        }
                    }
                }
            });

        if dismiss_overlay {
            self.show_welcome_overlay = false;
        }
    }

    fn update_preview_texture(&mut self, ctx: &egui::Context, image: ColorImage) {
        if let Some(texture) = &mut self.preview_texture {
            texture.set(image, TextureOptions::LINEAR);
        } else {
            self.preview_texture = Some(ctx.load_texture(
                "mesh_preview",
                image,
                TextureOptions::LINEAR,
            ));
        }
    }

    fn clear_history(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
    }

    fn ensure_albedo_texture(&mut self) {
        if self.albedo_texture.is_none() {
            self.albedo_texture = Some(RgbaImage::from_pixel(
                1024,
                1024,
                image::Rgba([200, 200, 200, 255]),
            ));
        }
    }

    fn begin_paint_stroke(&mut self) {
        self.ensure_albedo_texture();
        if let Some(texture) = &self.albedo_texture {
            self.undo_stack.push_back(texture.clone());
            if self.undo_stack.len() > Self::MAX_HISTORY {
                self.undo_stack.pop_front();
            }
            self.redo_stack.clear();
            self.is_dirty = true;
        }
    }

    fn undo_paint(&mut self) {
        let Some(current) = self.albedo_texture.take() else {
            return;
        };
        let Some(previous) = self.undo_stack.pop_back() else {
            self.albedo_texture = Some(current);
            return;
        };
        self.redo_stack.push_back(current);
        if self.redo_stack.len() > Self::MAX_HISTORY {
            self.redo_stack.pop_front();
        }
        self.albedo_texture = Some(previous);
        self.is_dirty = true;
    }

    fn redo_paint(&mut self) {
        let Some(current) = self.albedo_texture.take() else {
            return;
        };
        let Some(next) = self.redo_stack.pop_back() else {
            self.albedo_texture = Some(current);
            return;
        };
        self.undo_stack.push_back(current);
        if self.undo_stack.len() > Self::MAX_HISTORY {
            self.undo_stack.pop_front();
        }
        self.albedo_texture = Some(next);
        self.is_dirty = true;
    }

    fn save_texture_to_file(&mut self) {
        let Some(texture) = &self.albedo_texture else {
            self.last_error = Some("No texture to save. Paint or load a texture first.".to_string());
            return;
        };

        let Some(path) = rfd::FileDialog::new()
            .add_filter("PNG Image", &["png"])
            .set_file_name("pinturapp_texture.png")
            .save_file()
        else {
            return;
        };

        match texture.save(&path) {
            Ok(()) => {
                self.last_error = None;
                self.loaded_texture_path = Some(path);
            }
            Err(err) => {
                self.last_error = Some(format!("Failed to save texture: {err}"));
            }
        }
    }

    fn pick_and_load_obj(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Wavefront OBJ", &["obj"])
            .pick_file()
        else {
            return;
        };
        match load_obj(&path) {
            Ok(mesh) => {
                let (center, fit_scale) = compute_mesh_fit(&mesh);
                self.last_error = None;
                self.last_loaded_path = Some(path);
                self.loaded_mesh = Some(mesh);
                self.mesh_center = center;
                self.mesh_fit_scale = fit_scale;
                self.orbit_yaw = 0.5;
                self.orbit_pitch = 0.25;
                self.orbit_distance = 3.0;
                self.is_dirty = true;
                self.show_welcome_overlay = false;
            }
            Err(err) => {
                self.loaded_mesh = None;
                self.last_loaded_path = Some(path);
                self.last_error = Some(err);
            }
        }
    }

    fn pick_and_load_texture(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Texture", &["png", "jpg", "jpeg", "bmp", "tga"])
            .pick_file()
        else {
            return;
        };
        match image::open(&path) {
            Ok(dynamic) => {
                self.albedo_texture = Some(dynamic.to_rgba8());
                self.loaded_texture_path = Some(path);
                self.last_error = None;
                self.clear_history();
                self.is_dirty = true;
            }
            Err(err) => {
                self.last_error = Some(format!("Failed to load texture: {err}"));
                self.loaded_texture_path = Some(path);
            }
        }
    }

    fn project_state(&self) -> ProjectState {
        ProjectState {
            mesh_entry: None,
            texture_entry: None,
            orbit_yaw: self.orbit_yaw,
            orbit_pitch: self.orbit_pitch,
            orbit_distance: self.orbit_distance,
            brush_radius_px: self.brush_radius_px,
            brush_color_rgba: self.brush_color.to_array(),
        }
    }

    fn apply_project_state(&mut self, state: ProjectState, base_dir: &Path) -> Result<(), String> {
        self.orbit_yaw = state.orbit_yaw;
        self.orbit_pitch = state.orbit_pitch;
        self.orbit_distance = state.orbit_distance;
        self.brush_radius_px = state.brush_radius_px.clamp(1.0, 64.0);
        self.brush_color = egui::Color32::from_rgba_unmultiplied(
            state.brush_color_rgba[0],
            state.brush_color_rgba[1],
            state.brush_color_rgba[2],
            state.brush_color_rgba[3],
        );

        self.loaded_mesh = None;
        self.last_loaded_path = None;
        self.loaded_texture_path = None;
        self.albedo_texture = None;

        if let Some(mesh_entry) = state.mesh_entry {
            let path = base_dir.join(mesh_entry);
            let mesh =
                load_obj(&path).map_err(|e| format!("Failed to load mesh from project bundle: {e}"))?;
            let (center, fit_scale) = compute_mesh_fit(&mesh);
            self.loaded_mesh = Some(mesh);
            self.last_loaded_path = Some(path);
            self.mesh_center = center;
            self.mesh_fit_scale = fit_scale;
        }

        if let Some(texture_entry) = state.texture_entry {
            let path = base_dir.join(texture_entry);
            let tex = image::open(&path)
                .map_err(|e| format!("Failed to load texture from project bundle: {e}"))?
                .to_rgba8();
            self.albedo_texture = Some(tex);
            self.loaded_texture_path = Some(path);
            self.clear_history();
        }

        self.is_dirty = false;
        Ok(())
    }

    fn save_project_to_file(&mut self) {
        let target = self.current_project_path.clone().or_else(|| {
            rfd::FileDialog::new()
                .add_filter("Pinturapp Project Bundle", &["pinturaproj", "zip"])
                .set_file_name("pinturapp_project.pinturaproj")
                .save_file()
        });
        let Some(path) = target else {
            return;
        };
        match self.save_project_to_path(&path) {
            Ok(()) => {
                self.last_error = None;
                self.current_project_path = Some(path.clone());
                self.record_recent_project(path);
                self.is_dirty = false;
            }
            Err(err) => self.last_error = Some(err),
        }
    }

    fn save_project_as_to_file(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Pinturapp Project Bundle", &["pinturaproj", "zip"])
            .set_file_name("pinturapp_project.pinturaproj")
            .save_file()
        else {
            return;
        };
        match self.save_project_to_path(&path) {
            Ok(()) => {
                self.last_error = None;
                self.current_project_path = Some(path.clone());
                self.record_recent_project(path);
                self.is_dirty = false;
            }
            Err(err) => self.last_error = Some(err),
        }
    }

    fn request_load_action(&mut self, action: PendingLoadAction) {
        if self.is_dirty {
            self.pending_load_action = Some(action);
            self.show_discard_confirm = true;
            return;
        }
        self.execute_load_action(action);
    }

    fn execute_load_action(&mut self, action: PendingLoadAction) {
        match action {
            PendingLoadAction::NewProject => {
                self.clear_session();
                self.last_error = None;
                self.show_welcome_overlay = false;
            }
            PendingLoadAction::OpenProjectPicker => self.load_project_from_file(),
            PendingLoadAction::LoadProject(path) => match self.load_project_from_path(&path) {
                Ok(()) => {
                    self.last_error = None;
                    self.show_welcome_overlay = false;
                }
                Err(err) => self.last_error = Some(err),
            },
            PendingLoadAction::LoadAutosave => {
                self.load_autosave();
                if self.last_error.is_none() {
                    self.show_welcome_overlay = false;
                }
            }
        }
    }

    fn show_discard_confirm_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_discard_confirm {
            return;
        }
        let mut open = true;
        egui::Window::new("Unsaved Changes")
            .collapsible(false)
            .resizable(false)
            .default_width(380.0)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label("You have unsaved changes. Continue loading and discard current changes?");
                ui.separator();
                if ui.button("Save and Continue").clicked() {
                    self.save_project_to_file();
                    if !self.is_dirty {
                        self.show_discard_confirm = false;
                        if let Some(action) = self.pending_load_action.take() {
                            self.execute_load_action(action);
                        }
                    }
                }
                if ui.button("Discard and Continue").clicked() {
                    self.show_discard_confirm = false;
                    if let Some(action) = self.pending_load_action.take() {
                        self.execute_load_action(action);
                    }
                }
                if ui.button("Cancel").clicked() {
                    self.show_discard_confirm = false;
                    self.pending_load_action = None;
                }
            });
        if !open {
            self.show_discard_confirm = false;
            self.pending_load_action = None;
        }
    }

    fn show_autosave_recovery_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_autosave_recovery_prompt {
            return;
        }
        let mut open = true;
        egui::Window::new("Recover Autosave")
            .collapsible(false)
            .resizable(false)
            .default_width(380.0)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label("An autosave was found from a previous session.");
                ui.separator();
                if ui.button("Load Autosave").clicked() {
                    self.show_autosave_recovery_prompt = false;
                    self.request_load_action(PendingLoadAction::LoadAutosave);
                }
                if ui.button("Ignore").clicked() {
                    self.show_autosave_recovery_prompt = false;
                }
            });
        if !open {
            self.show_autosave_recovery_prompt = false;
        }
    }

    fn clear_session(&mut self) {
        self.loaded_mesh = None;
        self.last_loaded_path = None;
        self.loaded_texture_path = None;
        self.albedo_texture = None;
        self.preview_texture = None;
        self.orbit_yaw = 0.5;
        self.orbit_pitch = 0.25;
        self.orbit_distance = 3.0;
        self.mesh_center = Vec3::ZERO;
        self.mesh_fit_scale = 1.0;
        self.is_painting_stroke = false;
        self.current_project_path = None;
        self.clear_history();
        self.is_dirty = false;
    }

    fn load_project_from_file(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Pinturapp Project Bundle", &["pinturaproj", "zip"])
            .pick_file()
        else {
            return;
        };
        match self.load_project_from_path(&path) {
            Ok(()) => self.last_error = None,
            Err(err) => self.last_error = Some(err),
        }
    }

    fn save_project_to_path(&self, path: &Path) -> Result<(), String> {
        let file = fs::File::create(path)
            .map_err(|e| format!("Failed to create project bundle {}: {e}", path.display()))?;
        let mut zip = ZipWriter::new(file);
        let options = FileOptions::default().compression_method(CompressionMethod::Deflated);
        let mut state = self.project_state();

        if let Some(mesh_path) = &self.last_loaded_path {
            let mesh_bytes = fs::read(mesh_path)
                .map_err(|e| format!("Failed to read mesh file {}: {e}", mesh_path.display()))?;
            zip.start_file("assets/mesh.obj", options)
                .map_err(|e| format!("Failed to write mesh entry: {e}"))?;
            zip.write_all(&mesh_bytes)
                .map_err(|e| format!("Failed to write mesh bytes: {e}"))?;
            state.mesh_entry = Some("assets/mesh.obj".to_string());
        }

        if let Some(texture) = &self.albedo_texture {
            let mut cursor = Cursor::new(Vec::<u8>::new());
            image::DynamicImage::ImageRgba8(texture.clone())
                .write_to(&mut cursor, image::ImageOutputFormat::Png)
                .map_err(|e| format!("Failed to encode texture PNG: {e}"))?;
            zip.start_file("assets/texture.png", options)
                .map_err(|e| format!("Failed to write texture entry: {e}"))?;
            zip.write_all(&cursor.into_inner())
                .map_err(|e| format!("Failed to write texture bytes: {e}"))?;
            state.texture_entry = Some("assets/texture.png".to_string());
        }

        let state_json = serde_json::to_string_pretty(&state)
            .map_err(|e| format!("Failed to serialize project state: {e}"))?;
        zip.start_file("project.json", options)
            .map_err(|e| format!("Failed to write project state entry: {e}"))?;
        zip.write_all(state_json.as_bytes())
            .map_err(|e| format!("Failed to write project state: {e}"))?;
        zip.finish()
            .map_err(|e| format!("Failed to finalize project bundle: {e}"))?;
        Ok(())
    }

    fn load_project_from_path(&mut self, path: &Path) -> Result<(), String> {
        self.load_project_from_path_internal(path, true, true)
    }

    fn load_project_from_path_internal(
        &mut self,
        path: &Path,
        set_as_current: bool,
        add_to_recent: bool,
    ) -> Result<(), String> {
        let file = fs::File::open(path)
            .map_err(|e| format!("Failed to open project bundle {}: {e}", path.display()))?;
        let mut archive = ZipArchive::new(file)
            .map_err(|e| format!("Invalid project bundle (zip): {e}"))?;

        let unpack_dir = self.next_unpack_dir(path);
        fs::create_dir_all(&unpack_dir)
            .map_err(|e| format!("Failed to prepare unpack directory {}: {e}", unpack_dir.display()))?;

        for i in 0..archive.len() {
            let mut entry = archive
                .by_index(i)
                .map_err(|e| format!("Failed to read bundle entry {i}: {e}"))?;
            let Some(rel_path) = entry.enclosed_name().map(Path::to_path_buf) else {
                continue;
            };
            let out_path = unpack_dir.join(rel_path);
            if entry.name().ends_with('/') {
                fs::create_dir_all(&out_path).map_err(|e| {
                    format!("Failed to create bundle directory {}: {e}", out_path.display())
                })?;
                continue;
            }
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent).map_err(|e| {
                    format!(
                        "Failed to create bundle file parent directory {}: {e}",
                        parent.display()
                    )
                })?;
            }
            let mut out_file = fs::File::create(&out_path)
                .map_err(|e| format!("Failed to create unpacked file {}: {e}", out_path.display()))?;
            std::io::copy(&mut entry, &mut out_file)
                .map_err(|e| format!("Failed to unpack entry to {}: {e}", out_path.display()))?;
        }

        let mut state_text = String::new();
        let mut state_file = fs::File::open(unpack_dir.join("project.json")).map_err(|e| {
            format!(
                "Bundle is missing project state file {}: {e}",
                unpack_dir.join("project.json").display()
            )
        })?;
        state_file
            .read_to_string(&mut state_text)
            .map_err(|e| format!("Failed to read bundle project state: {e}"))?;
        let state: ProjectState = serde_json::from_str(&state_text)
            .map_err(|e| format!("Failed to parse bundle project state: {e}"))?;
        self.apply_project_state(state, &unpack_dir)?;
        if set_as_current {
            self.current_project_path = Some(path.to_path_buf());
        }
        if add_to_recent {
            self.record_recent_project(path.to_path_buf());
        }
        Ok(())
    }

    fn load_autosave(&mut self) {
        if !self.autosave_path.exists() {
            self.last_error = Some("No autosave project found yet.".to_string());
            return;
        }
        let autosave_path = self.autosave_path.clone();
        match self.load_project_from_path_internal(&autosave_path, false, false) {
            Ok(()) => self.last_error = None,
            Err(err) => self.last_error = Some(format!("Failed to load autosave: {err}")),
        }
    }

    fn next_unpack_dir(&self, source_path: &Path) -> PathBuf {
        let stamp = unix_timestamp_secs();
        let stem = source_path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "project".to_string());
        self.storage_dir
            .join("unpacked")
            .join(format!("{stem}_{stamp}"))
    }

    fn maybe_autosave(&mut self) {
        if !self.is_dirty {
            return;
        }
        if self.last_autosave_at.elapsed() < Duration::from_secs(20) {
            return;
        }
        let autosave_path = self.autosave_path.clone();
        if let Err(err) = self.save_project_to_path(&autosave_path) {
            self.last_error = Some(format!("Autosave failed: {err}"));
            return;
        }
        self.last_autosave_at = Instant::now();
        self.last_autosave_ok_at = Some(self.last_autosave_at);
    }

    fn autosave_status_text(&self) -> String {
        if let Some(last_ok) = self.last_autosave_ok_at {
            let secs = last_ok.elapsed().as_secs();
            return format!("{secs}s ago");
        }
        if self.autosave_path.exists() {
            return "available".to_string();
        }
        "pending".to_string()
    }

    fn record_recent_project(&mut self, path: PathBuf) {
        self.recent_projects.retain(|p| p != &path);
        self.recent_projects.insert(0, path);
        if self.recent_projects.len() > 10 {
            self.recent_projects.truncate(10);
        }
        let _ = save_recent_projects(&self.storage_dir, &self.recent_projects);
    }

    fn clear_recent_projects(&mut self) {
        self.recent_projects.clear();
        let _ = save_recent_projects(&self.storage_dir, &self.recent_projects);
    }

    fn paint_at_uv(&mut self, uv: [f32; 2]) {
        self.ensure_albedo_texture();

        let Some(texture) = self.albedo_texture.as_mut() else {
            return;
        };
        let w = texture.width().max(1);
        let h = texture.height().max(1);
        let cx = uv[0].clamp(0.0, 1.0) * (w.saturating_sub(1) as f32);
        let cy = uv[1].clamp(0.0, 1.0) * (h.saturating_sub(1) as f32);
        let radius = self.brush_radius_px.max(1.0);
        let r2 = radius * radius;

        let min_x = (cx - radius).floor().max(0.0) as i32;
        let max_x = (cx + radius).ceil().min((w - 1) as f32) as i32;
        let min_y = (cy - radius).floor().max(0.0) as i32;
        let max_y = (cy + radius).ceil().min((h - 1) as f32) as i32;

        let src = self.brush_color.to_array();
        let src_alpha = src[3] as f32 / 255.0;
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let dx = x as f32 - cx;
                let dy = y as f32 - cy;
                if dx * dx + dy * dy > r2 {
                    continue;
                }

                let px = texture.get_pixel_mut(x as u32, y as u32);
                let dst = px.0;
                let out_r = src[0] as f32 * src_alpha + dst[0] as f32 * (1.0 - src_alpha);
                let out_g = src[1] as f32 * src_alpha + dst[1] as f32 * (1.0 - src_alpha);
                let out_b = src[2] as f32 * src_alpha + dst[2] as f32 * (1.0 - src_alpha);
                *px = image::Rgba([out_r as u8, out_g as u8, out_b as u8, 255]);
            }
        }
        self.is_dirty = true;
    }
}

fn default_storage_dir() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".pinturapp")
}

fn recent_projects_path(storage_dir: &Path) -> PathBuf {
    storage_dir.join("recent_projects.json")
}

fn load_recent_projects(storage_dir: &Path) -> Vec<PathBuf> {
    let path = recent_projects_path(storage_dir);
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(recent) = serde_json::from_str::<RecentProjects>(&text) else {
        return Vec::new();
    };
    recent.paths.into_iter().map(PathBuf::from).collect()
}

fn save_recent_projects(storage_dir: &Path, paths: &[PathBuf]) -> Result<(), String> {
    let recent = RecentProjects {
        paths: paths
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect(),
    };
    let text = serde_json::to_string_pretty(&recent)
        .map_err(|e| format!("Failed to serialize recent projects: {e}"))?;
    fs::write(recent_projects_path(storage_dir), text)
        .map_err(|e| format!("Failed to save recent projects: {e}"))
}

fn unix_timestamp_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn compute_mesh_fit(mesh: &MeshData) -> (Vec3, f32) {
    let mut min = vec3(f32::MAX, f32::MAX, f32::MAX);
    let mut max = vec3(f32::MIN, f32::MIN, f32::MIN);

    for v in &mesh.vertices {
        let p = vec3(v.position[0], v.position[1], v.position[2]);
        min = min.min(p);
        max = max.max(p);
    }

    let center = (min + max) * 0.5;
    let extent = (max - min).max(vec3(1e-4, 1e-4, 1e-4));
    let largest = extent.x.max(extent.y).max(extent.z);
    let fit_scale = 1.8 / largest;
    (center, fit_scale)
}

fn render_textured_preview(
    mesh: &MeshData,
    center: Vec3,
    fit_scale: f32,
    yaw: f32,
    pitch: f32,
    distance: f32,
    size: [usize; 2],
    albedo: Option<&RgbaImage>,
) -> ColorImage {
    let width = size[0].max(1);
    let height = size[1].max(1);
    let mut pixels = vec![0_u8; width * height * 4];
    let mut depth = vec![f32::INFINITY; width * height];

    // Background
    for i in 0..(width * height) {
        let p = i * 4;
        pixels[p] = 26;
        pixels[p + 1] = 29;
        pixels[p + 2] = 35;
        pixels[p + 3] = 255;
    }

    let target = vec3(0.0, 0.0, 0.0);
    let eye = target
        + vec3(
            distance * yaw.cos() * pitch.cos(),
            distance * pitch.sin(),
            distance * yaw.sin() * pitch.cos(),
        );

    let model = Mat4::from_scale(Vec3::splat(fit_scale)) * Mat4::from_translation(-center);
    let view = Mat4::look_at_rh(eye, target, Vec3::Y);
    let aspect = (width as f32 / height as f32).max(0.01);
    let proj = Mat4::perspective_rh_gl(45.0_f32.to_radians(), aspect, 0.01, 200.0);
    let mvp = proj * view * model;

    for tri in mesh.indices.chunks_exact(3) {
        let i0 = tri[0] as usize;
        let i1 = tri[1] as usize;
        let i2 = tri[2] as usize;
        let (Some(v0), Some(v1), Some(v2)) = (
            mesh.vertices.get(i0),
            mesh.vertices.get(i1),
            mesh.vertices.get(i2),
        ) else {
            continue;
        };

        let Some(p0) = project_vertex(mvp, v0.position, width, height) else {
            continue;
        };
        let Some(p1) = project_vertex(mvp, v1.position, width, height) else {
            continue;
        };
        let Some(p2) = project_vertex(mvp, v2.position, width, height) else {
            continue;
        };

        rasterize_triangle(
            &mut pixels,
            &mut depth,
            [width, height],
            [(p0.0, p0.1, p0.2, v0.uv), (p1.0, p1.1, p1.2, v1.uv), (p2.0, p2.1, p2.2, v2.uv)],
            albedo,
        );
    }

    ColorImage::from_rgba_unmultiplied([width, height], &pixels)
}

fn pick_uv_at_screen(
    mesh: &MeshData,
    center: Vec3,
    fit_scale: f32,
    yaw: f32,
    pitch: f32,
    distance: f32,
    size: [usize; 2],
    screen: [f32; 2],
) -> Option<[f32; 2]> {
    let width = size[0].max(1);
    let height = size[1].max(1);

    let target = vec3(0.0, 0.0, 0.0);
    let eye = target
        + vec3(
            distance * yaw.cos() * pitch.cos(),
            distance * pitch.sin(),
            distance * yaw.sin() * pitch.cos(),
        );
    let model = Mat4::from_scale(Vec3::splat(fit_scale)) * Mat4::from_translation(-center);
    let view = Mat4::look_at_rh(eye, target, Vec3::Y);
    let aspect = (width as f32 / height as f32).max(0.01);
    let proj = Mat4::perspective_rh_gl(45.0_f32.to_radians(), aspect, 0.01, 200.0);
    let mvp = proj * view * model;

    let mut best_z = f32::INFINITY;
    let mut best_uv: Option<[f32; 2]> = None;

    for tri in mesh.indices.chunks_exact(3) {
        let (Some(v0), Some(v1), Some(v2)) = (
            mesh.vertices.get(tri[0] as usize),
            mesh.vertices.get(tri[1] as usize),
            mesh.vertices.get(tri[2] as usize),
        ) else {
            continue;
        };
        let Some(p0) = project_vertex(mvp, v0.position, width, height) else {
            continue;
        };
        let Some(p1) = project_vertex(mvp, v1.position, width, height) else {
            continue;
        };
        let Some(p2) = project_vertex(mvp, v2.position, width, height) else {
            continue;
        };

        let area = edge_fn(p0.0, p0.1, p1.0, p1.1, p2.0, p2.1);
        if area.abs() < 1e-6 {
            continue;
        }

        let px = screen[0] + 0.5;
        let py = screen[1] + 0.5;
        let w0 = edge_fn(p1.0, p1.1, p2.0, p2.1, px, py) / area;
        let w1 = edge_fn(p2.0, p2.1, p0.0, p0.1, px, py) / area;
        let w2 = edge_fn(p0.0, p0.1, p1.0, p1.1, px, py) / area;
        if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
            continue;
        }

        let z = w0 * p0.2 + w1 * p1.2 + w2 * p2.2;
        if z < best_z {
            best_z = z;
            let u = w0 * v0.uv[0] + w1 * v1.uv[0] + w2 * v2.uv[0];
            let v = w0 * v0.uv[1] + w1 * v1.uv[1] + w2 * v2.uv[1];
            best_uv = Some([u, v]);
        }
    }

    best_uv
}

fn project_vertex(
    mvp: Mat4,
    pos: [f32; 3],
    width: usize,
    height: usize,
) -> Option<(f32, f32, f32)> {
    let clip = mvp * Vec4::new(pos[0], pos[1], pos[2], 1.0);
    if clip.w <= 0.0 {
        return None;
    }

    let ndc = clip.truncate() / clip.w;
    if ndc.x.abs() > 1.3 || ndc.y.abs() > 1.3 || ndc.z.abs() > 1.3 {
        return None;
    }

    let sx = (ndc.x * 0.5 + 0.5) * width as f32;
    let sy = (-ndc.y * 0.5 + 0.5) * height as f32;
    Some((sx, sy, ndc.z))
}

fn rasterize_triangle(
    pixels: &mut [u8],
    depth: &mut [f32],
    size: [usize; 2],
    tri: [(f32, f32, f32, [f32; 2]); 3],
    albedo: Option<&RgbaImage>,
) {
    let width = size[0];
    let height = size[1];
    let (x0, y0, z0, uv0) = tri[0];
    let (x1, y1, z1, uv1) = tri[1];
    let (x2, y2, z2, uv2) = tri[2];

    let area = edge_fn(x0, y0, x1, y1, x2, y2);
    if area.abs() < 1e-6 {
        return;
    }

    let min_x = x0.min(x1).min(x2).floor().max(0.0) as i32;
    let max_x = x0.max(x1).max(x2).ceil().min((width - 1) as f32) as i32;
    let min_y = y0.min(y1).min(y2).floor().max(0.0) as i32;
    let max_y = y0.max(y1).max(y2).ceil().min((height - 1) as f32) as i32;

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;

            let w0 = edge_fn(x1, y1, x2, y2, px, py) / area;
            let w1 = edge_fn(x2, y2, x0, y0, px, py) / area;
            let w2 = edge_fn(x0, y0, x1, y1, px, py) / area;
            if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                continue;
            }

            let z = w0 * z0 + w1 * z1 + w2 * z2;
            let idx = y as usize * width + x as usize;
            if z >= depth[idx] {
                continue;
            }
            depth[idx] = z;

            let u = w0 * uv0[0] + w1 * uv1[0] + w2 * uv2[0];
            let v = w0 * uv0[1] + w1 * uv1[1] + w2 * uv2[1];
            let mut color = if let Some(tex) = albedo {
                sample_texture(tex, u, v)
            } else {
                let ru = (u.fract().abs() * 255.0) as u8;
                let gv = (v.fract().abs() * 255.0) as u8;
                [ru, gv, 220, 255]
            };

            let shade = (1.2 - z * 0.35).clamp(0.35, 1.15);
            color[0] = ((color[0] as f32) * shade).clamp(0.0, 255.0) as u8;
            color[1] = ((color[1] as f32) * shade).clamp(0.0, 255.0) as u8;
            color[2] = ((color[2] as f32) * shade).clamp(0.0, 255.0) as u8;

            let p = idx * 4;
            pixels[p] = color[0];
            pixels[p + 1] = color[1];
            pixels[p + 2] = color[2];
            pixels[p + 3] = 255;
        }
    }
}

fn sample_texture(tex: &RgbaImage, u: f32, v: f32) -> [u8; 4] {
    let w = tex.width().max(1);
    let h = tex.height().max(1);
    let uu = u.rem_euclid(1.0);
    let vv = v.rem_euclid(1.0);
    let x = (uu * (w.saturating_sub(1) as f32)).round() as u32;
    let y = (vv * (h.saturating_sub(1) as f32)).round() as u32;
    tex.get_pixel(x, y).0
}

fn edge_fn(ax: f32, ay: f32, bx: f32, by: f32, px: f32, py: f32) -> f32 {
    (px - ax) * (by - ay) - (py - ay) * (bx - ax)
}

fn draw_mesh_wireframe(
    painter: &egui::Painter,
    rect: egui::Rect,
    mesh: &MeshData,
    center: Vec3,
    fit_scale: f32,
    yaw: f32,
    pitch: f32,
    distance: f32,
) {
    let target = vec3(0.0, 0.0, 0.0);
    let eye = target
        + vec3(
            distance * yaw.cos() * pitch.cos(),
            distance * pitch.sin(),
            distance * yaw.sin() * pitch.cos(),
        );

    let model = Mat4::from_scale(Vec3::splat(fit_scale)) * Mat4::from_translation(-center);
    let view = Mat4::look_at_rh(eye, target, Vec3::Y);
    let aspect = (rect.width() / rect.height()).max(0.01);
    let proj = Mat4::perspective_rh_gl(45.0_f32.to_radians(), aspect, 0.01, 200.0);
    let mvp = proj * view * model;

    let mut unique_edges: HashSet<(u32, u32)> = HashSet::new();

    for tri in mesh.indices.chunks_exact(3) {
        let a = tri[0];
        let b = tri[1];
        let c = tri[2];
        add_edge(&mut unique_edges, a, b);
        add_edge(&mut unique_edges, b, c);
        add_edge(&mut unique_edges, c, a);
    }

    let stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(150, 200, 255));
    for (a, b) in unique_edges {
        let p0 = mesh.vertices.get(a as usize).map(|v| v.position);
        let p1 = mesh.vertices.get(b as usize).map(|v| v.position);
        let (Some(p0), Some(p1)) = (p0, p1) else {
            continue;
        };

        let Some(s0) = project_to_screen(rect, mvp, p0) else {
            continue;
        };
        let Some(s1) = project_to_screen(rect, mvp, p1) else {
            continue;
        };

        painter.line_segment([s0, s1], stroke);
    }
}

fn add_edge(edges: &mut HashSet<(u32, u32)>, a: u32, b: u32) {
    if a < b {
        edges.insert((a, b));
    } else {
        edges.insert((b, a));
    }
}

fn project_to_screen(rect: egui::Rect, mvp: Mat4, pos: [f32; 3]) -> Option<egui::Pos2> {
    let clip: Vec4 = mvp * Vec4::new(pos[0], pos[1], pos[2], 1.0);
    if clip.w <= 0.0 {
        return None;
    }

    let ndc = clip.truncate() / clip.w;
    if ndc.x.abs() > 1.5 || ndc.y.abs() > 1.5 || ndc.z.abs() > 1.5 {
        return None;
    }

    let sx = rect.left() + (ndc.x * 0.5 + 0.5) * rect.width();
    let sy = rect.top() + (-ndc.y * 0.5 + 0.5) * rect.height();
    Some(egui::pos2(sx, sy))
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
