use crate::PinturappUi;
use crate::renderer::{draw_mesh_wireframe, pick_paint_uv_targets_at_screen, render_textured_preview};
use eframe::egui;

impl PinturappUi {
    pub(crate) fn show_status_bar_panel(&mut self, ui: &mut egui::Ui) {
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
    }

    pub(crate) fn show_left_panel(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("layers")
            .resizable(true)
            .default_size(220.0)
            .show_inside(ui, |ui| {
                ui.heading("Layers");
                ui.separator();
                self.show_viewport_controls_card(ui);
                ui.add_space(8.0);
                self.show_material_card(ui);
                ui.add_space(8.0);
                self.show_brush_card(ui);
                if let Some(path) = &self.current_project_path {
                    ui.add_space(8.0);
                    self.show_project_card(ui, path);
                }
            });
    }

    pub(crate) fn show_viewport_panel(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default().show_inside(ui, |ui| {
            ui.heading("3D Viewport");
            if let Some(mesh) = self.loaded_mesh.clone() {
                self.show_mesh_details(ui, &mesh);
                ui.separator();
                let available = ui.available_size_before_wrap();
                let viewport_size = egui::vec2(available.x.max(240.0), available.y.max(240.0));
                let (response, painter) = ui.allocate_painter(viewport_size, egui::Sense::drag());
                let rect = response.rect;

                painter.rect_filled(rect, 4.0, egui::Color32::from_rgb(26, 29, 35));
                self.handle_camera_input(ui, &response);
                let img_w = rect.width().max(1.0).round() as usize;
                let img_h = rect.height().max(1.0).round() as usize;
                self.handle_paint_input(ui, &response, rect, &mesh, [img_w, img_h]);
                self.draw_viewport_texture_and_wireframe(ui, &painter, rect, &mesh, [img_w, img_h]);
            } else {
                self.is_painting_stroke = false;
                ui.label("No mesh loaded yet. Use 'Load OBJ' to import a model with UVs.");
            }
            self.show_viewport_footer(ui);
        });
    }

    fn show_mesh_details(&self, ui: &mut egui::Ui, mesh: &crate::io::mesh_loader::MeshData) {
        ui.label(format!("Loaded: {}", mesh.source_path.display()));
        ui.label(format!("Vertices: {}", mesh.vertices.len()));
        ui.label(format!("Triangles: {}", mesh.indices.len() / 3));
        if let Some(v0) = mesh.vertices.first() {
            ui.label(format!(
                "Sample Vertex: pos=({:.3}, {:.3}, {:.3}) uv=({:.3}, {:.3})",
                v0.position[0], v0.position[1], v0.position[2], v0.uv[0], v0.uv[1]
            ));
        }
    }

    fn show_viewport_controls_card(&self, ui: &mut egui::Ui) {
        Self::panel_card().show(ui, |ui| {
            ui.strong("Viewport Controls");
            ui.small("LMB Drag: Paint");
            ui.small("RMB Drag: Orbit");
            ui.small("Scroll: Zoom");
        });
    }

    fn show_material_card(&self, ui: &mut egui::Ui) {
        Self::panel_card().show(ui, |ui| {
            ui.strong("Material");
            if let Some(path) = &self.loaded_texture_path {
                ui.small(format!("Texture: {}", path.display()));
            } else {
                ui.small("Texture: UV gradient fallback");
            }
        });
    }

    fn show_brush_card(&mut self, ui: &mut egui::Ui) {
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
    }

    fn show_project_card(&self, ui: &mut egui::Ui, path: &std::path::Path) {
        Self::panel_card().show(ui, |ui| {
            ui.strong("Project");
            ui.small(path.display().to_string());
        });
    }

    fn handle_camera_input(&mut self, ui: &egui::Ui, response: &egui::Response) {
        if response.dragged_by(egui::PointerButton::Secondary) {
            let delta = ui.ctx().input(|i| i.pointer.delta());
            self.orbit_yaw += delta.x * 0.01;
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
    }

    fn handle_paint_input(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        rect: egui::Rect,
        mesh: &crate::io::mesh_loader::MeshData,
        image_size: [usize; 2],
    ) {
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
                let brush_screen_radius = self.estimate_brush_screen_radius(image_size);
                let paint_targets = pick_paint_uv_targets_at_screen(
                    mesh,
                    self.mesh_center,
                    self.mesh_fit_scale,
                    self.orbit_yaw,
                    self.orbit_pitch,
                    self.orbit_distance,
                    image_size,
                    [sx, sy],
                    brush_screen_radius,
                );
                for uv in paint_targets {
                    self.paint_at_uv(uv);
                }
            }
        }
    }

    fn estimate_brush_screen_radius(&self, image_size: [usize; 2]) -> f32 {
        let viewport_w = image_size[0].max(1) as f32;
        let viewport_h = image_size[1].max(1) as f32;
        if let Some(tex) = &self.albedo_texture {
            let tex_w = tex.width().max(1) as f32;
            let tex_h = tex.height().max(1) as f32;
            let sx = viewport_w / tex_w;
            let sy = viewport_h / tex_h;
            (self.brush_radius_px * sx.min(sy)).clamp(1.5, 64.0)
        } else {
            (self.brush_radius_px * 0.2).clamp(1.5, 32.0)
        }
    }

    fn draw_viewport_texture_and_wireframe(
        &mut self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        rect: egui::Rect,
        mesh: &crate::io::mesh_loader::MeshData,
        image_size: [usize; 2],
    ) {
        let image = render_textured_preview(
            mesh,
            self.mesh_center,
            self.mesh_fit_scale,
            self.orbit_yaw,
            self.orbit_pitch,
            self.orbit_distance,
            image_size,
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
            painter,
            rect,
            mesh,
            self.mesh_center,
            self.mesh_fit_scale,
            self.orbit_yaw,
            self.orbit_pitch,
            self.orbit_distance,
        );
    }

    fn show_viewport_footer(&self, ui: &mut egui::Ui) {
        if let Some(path) = &self.last_loaded_path {
            ui.label(format!("Last file: {}", path.display()));
        }

        if let Some(err) = &self.last_error {
            ui.colored_label(egui::Color32::RED, format!("Load error: {err}"));
        }
    }
}
