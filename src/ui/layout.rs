use crate::PinturappUi;
use crate::renderer::{
    BrushBlendMode, BrushDispatch, BrushInput, draw_mesh_wireframe, render_preview_frame,
    sample_surface_from_buffer,
};
use eframe::egui;

impl PinturappUi {
    fn quantize_pressure(value: f32) -> f32 {
        ((value.clamp(0.0, 1.0) * 100.0).round() / 100.0).clamp(0.0, 1.0)
    }

    fn current_brush_pressure(ui: &egui::Ui) -> Option<f32> {
        let event_pressure = ui.ctx().input(|i| {
            i.events
                .iter()
                .rev()
                .find_map(|event| match event {
                    egui::Event::Touch { force, .. } => force.map(|value| value.clamp(0.0, 1.0)),
                    _ => None,
                })
        });
        if event_pressure.is_some() {
            return event_pressure;
        }
        #[cfg(target_os = "windows")]
        {
            if let Some(value) = crate::platform::windows_pen::latest_pressure(120) {
                return Some(value);
            }
            return crate::platform::windows_wintab::latest_pressure(80);
        }
        #[cfg(not(target_os = "windows"))]
        {
            None
        }
    }

    fn section_header(ui: &mut egui::Ui, label: &str) {
        ui.label(
            egui::RichText::new(label)
                .monospace()
                .size(10.5)
                .color(egui::Color32::from_rgb(162, 173, 194)),
        );
        ui.add_space(1.0);
        ui.separator();
    }

    fn status_chip(ui: &mut egui::Ui, tag: &str, value: impl Into<String>) {
        ui.label(
            egui::RichText::new(format!("[{tag}] {}", value.into()))
                .monospace()
                .size(10.5)
                .color(egui::Color32::from_rgb(198, 206, 222)),
        );
    }

    fn pressure_toggle_button(ui: &mut egui::Ui, enabled: &mut bool, label: &str) -> bool {
        let text = if *enabled {
            format!("[P] {label}")
        } else {
            format!("[ ] {label}")
        };
        ui.selectable_label(*enabled, text).clicked()
    }

    pub(crate) fn show_status_bar_panel(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("status_bar").show_inside(ui, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(6.0, 2.0);
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
                Self::status_chip(
                    ui,
                    "PRJ",
                    self.current_project_path
                        .as_ref()
                        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
                        .unwrap_or_else(|| "Untitled".to_string()),
                );
                ui.separator();
                Self::status_chip(ui, "MSH", mesh_status);
                ui.separator();
                Self::status_chip(ui, "TEX", texture_status);
                ui.separator();
                let dirty_label = if self.is_dirty {
                    "Dirty"
                } else {
                    "Saved"
                };
                Self::status_chip(ui, "STS", dirty_label);
                ui.separator();
                Self::status_chip(ui, "ASV", self.autosave_status_text());
                ui.separator();
                let pressure_mode = if self.use_tablet_pressure {
                    #[cfg(target_os = "windows")]
                    let has_pressure_signal = self.tablet_pressure_detected
                        || crate::platform::windows_pen::pressure_signal_detected()
                        || crate::platform::windows_wintab::pressure_signal_detected();
                    #[cfg(not(target_os = "windows"))]
                    let has_pressure_signal = self.tablet_pressure_detected;

                    if has_pressure_signal {
                        "tablet".to_string()
                    } else {
                        #[cfg(target_os = "windows")]
                        {
                            let (
                                hook_attempts,
                                hook_successes,
                                any_msgs,
                                pointer_msgs,
                                pressure_samples,
                                pointer_type,
                            ) =
                                crate::platform::windows_pen::debug_snapshot();
                            let (wt_attempts, wt_successes, wt_polls, wt_packets, wt_contact_packets, _) =
                                crate::platform::windows_wintab::debug_snapshot();
                            format!(
                                "tablet/no-signal (ink hook:{hook_successes}/{hook_attempts} any:{any_msgs} wm_ptr:{pointer_msgs} samples:{pressure_samples} type:{pointer_type} | wintab init:{wt_successes}/{wt_attempts} polls:{wt_polls} packets:{wt_packets} contact:{wt_contact_packets})"
                            )
                        }
                        #[cfg(not(target_os = "windows"))]
                        {
                            "tablet/no-signal".to_string()
                        }
                    }
                } else {
                    "fixed".to_string()
                };
                Self::status_chip(
                    ui,
                    "PRS",
                    format!("{:.2} ({pressure_mode})", self.display_brush_pressure),
                );
            });
        });
    }

    pub(crate) fn show_left_panel(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("layers")
            .resizable(true)
            .default_size(220.0)
            .frame(
                egui::Frame::default()
                    .fill(egui::Color32::from_rgb(39, 42, 49))
                    .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(72, 78, 90)))
                    .inner_margin(egui::Margin::same(8)),
            )
            .show_inside(ui, |ui| {
                Self::section_header(ui, "TOOL SETTINGS");
                self.show_material_card(ui);
                ui.add_space(6.0);
                self.show_brush_card(ui);
                ui.add_space(6.0);
                self.show_color_card(ui);
                ui.add_space(6.0);
                self.show_pipeline_card(ui);
                if let Some(path) = &self.current_project_path {
                    ui.add_space(6.0);
                    self.show_project_card(ui, path);
                }
            });
    }

    pub(crate) fn show_viewport_panel(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default()
            .frame(
                egui::Frame::default()
                    .fill(egui::Color32::from_rgb(31, 34, 40))
                    .inner_margin(egui::Margin::same(8)),
            )
            .show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("VIEWPORT")
                        .monospace()
                        .size(10.5)
                        .color(egui::Color32::from_rgb(167, 178, 198)),
                );
                ui.separator();
                ui.small("[LMB] Paint");
                ui.small("[RMB] Orbit");
                ui.small("[WHEEL] Zoom");
            });
            ui.separator();
            if let Some(mesh) = self.loaded_mesh.clone() {
                self.show_mesh_details(ui, &mesh);
                ui.separator();
                let available = ui.available_size_before_wrap();
                let viewport_size = egui::vec2(available.x.max(240.0), available.y.max(240.0));
                let (response, painter) = ui.allocate_painter(viewport_size, egui::Sense::drag());
                let rect = response.rect;

                self.draw_viewport_backdrop(&painter, rect, response.hovered());
                self.handle_camera_input(ui, &response);
                let img_w = rect.width().max(1.0).round() as usize;
                let img_h = rect.height().max(1.0).round() as usize;
                self.draw_viewport_texture_and_wireframe(ui, &painter, rect, &mesh, [img_w, img_h]);
                self.handle_paint_input(ui, &response, rect, &mesh);
                self.draw_viewport_overlay(ui, &painter, rect);
            } else {
                self.is_painting_stroke = false;
                self.preview_pick_buffer = None;
                Self::panel_card().show(ui, |ui| {
                    ui.label("No mesh loaded yet.");
                    ui.small("Use 'Load OBJ' to import a model with UVs.");
                });
            }
            self.show_viewport_footer(ui);
        });
    }

    fn show_mesh_details(&self, ui: &mut egui::Ui, mesh: &crate::io::mesh_loader::MeshData) {
        ui.horizontal_wrapped(|ui| {
            ui.small(format!("Mesh: {}", mesh.source_path.display()));
            ui.separator();
            ui.small(format!("Vertices: {}", mesh.vertices.len()));
            ui.separator();
            ui.small(format!("Triangles: {}", mesh.indices.len() / 3));
        });
    }

    fn show_material_card(&self, ui: &mut egui::Ui) {
        Self::panel_card().show(ui, |ui| {
            Self::section_header(ui, "MATERIAL");
            if let Some(path) = &self.loaded_texture_path {
                ui.small(format!("Texture: {}", path.display()));
            } else {
                ui.small("Texture: UV gradient fallback");
            }
        });
    }

    fn show_brush_card(&mut self, ui: &mut egui::Ui) {
        Self::panel_card().show(ui, |ui| {
            Self::section_header(ui, "BRUSH");
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("S {:.1}px", self.brush_size_px))
                        .monospace()
                        .color(egui::Color32::from_rgb(202, 210, 226)),
                );
                ui.separator();
                ui.label(
                    egui::RichText::new(format!("T {:.2}", self.brush_strength))
                        .monospace()
                        .color(egui::Color32::from_rgb(202, 210, 226)),
                );
                ui.separator();
                ui.label(
                    egui::RichText::new(format!("P {:.2}", self.display_brush_pressure))
                        .monospace()
                        .color(egui::Color32::from_rgb(202, 210, 226)),
                );
            });
            ui.add_space(4.0);

            if ui
                .add(egui::Slider::new(&mut self.brush_size_px, 1.0..=128.0).text("Size (px)"))
                .changed()
            {
                self.is_dirty = true;
            }

            ui.horizontal_wrapped(|ui| {
                ui.small("Presets");
                for preset in [2.0_f32, 4.0, 8.0, 16.0, 32.0, 64.0] {
                    if ui
                        .selectable_label((self.brush_size_px - preset).abs() < 0.5, format!("{preset:.0}"))
                        .clicked()
                    {
                        self.brush_size_px = preset;
                        self.is_dirty = true;
                    }
                }
            });

            if ui
                .add(egui::Slider::new(&mut self.brush_strength, 0.0..=1.0).text("Strength"))
                .changed()
            {
                self.is_dirty = true;
            }

            ui.horizontal_wrapped(|ui| {
                ui.small("Blend");
                if ui
                    .selectable_value(&mut self.brush_blend_mode, BrushBlendMode::Normal, "Normal")
                    .changed()
                {
                    self.is_dirty = true;
                }
                if ui
                    .selectable_value(&mut self.brush_blend_mode, BrushBlendMode::Multiply, "Multiply")
                    .changed()
                {
                    self.is_dirty = true;
                }
                if ui
                    .selectable_value(&mut self.brush_blend_mode, BrushBlendMode::Screen, "Screen")
                    .changed()
                {
                    self.is_dirty = true;
                }
            });

            ui.separator();
            ui.group(|ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.small("Pressure");
                    let pressure_mode = if self.use_tablet_pressure { "tablet" } else { "fixed" };
                    let signal = if self.tablet_pressure_detected { "signal" } else { "no-signal" };
                    ui.label(
                        egui::RichText::new(format!(
                            "[{pressure_mode}] {:.2} [{signal}]",
                            self.display_brush_pressure
                        ))
                        .monospace()
                        .color(egui::Color32::from_rgb(186, 196, 216)),
                    );
                });
                if ui
                    .checkbox(&mut self.use_tablet_pressure, "Use tablet pressure")
                    .changed()
                {
                    if !self.use_tablet_pressure {
                        self.tablet_pressure_detected = false;
                    }
                    self.is_dirty = true;
                }
                ui.horizontal_wrapped(|ui| {
                    if Self::pressure_toggle_button(ui, &mut self.use_pressure_for_size, "Affect Size") {
                        self.use_pressure_for_size = !self.use_pressure_for_size;
                        self.is_dirty = true;
                    }
                    if Self::pressure_toggle_button(
                        ui,
                        &mut self.use_pressure_for_strength,
                        "Affect Strength",
                    ) {
                        self.use_pressure_for_strength = !self.use_pressure_for_strength;
                        self.is_dirty = true;
                    }
                });
                ui.small("Input is quantized to 2 decimals. No temporal smoothing.");
            });

            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Undo").clicked() {
                    self.undo_paint();
                }
                if ui.button("Redo").clicked() {
                    self.redo_paint();
                }
                ui.small(format!(
                    "U:{}  R:{}",
                    self.undo_stack.len(),
                    self.redo_stack.len()
                ));
            });
        });
    }

    fn show_color_card(&mut self, ui: &mut egui::Ui) {
        Self::panel_card().show(ui, |ui| {
            Self::section_header(ui, "COLOR");
            if egui::color_picker::color_picker_color32(
                ui,
                &mut self.brush_color,
                egui::color_picker::Alpha::Opaque,
            ) {
                self.is_dirty = true;
            }
        });
    }

    fn show_pipeline_card(&mut self, ui: &mut egui::Ui) {
        Self::panel_card().show(ui, |ui| {
            Self::section_header(ui, "PIPELINE");
            if ui
                .add(
                    egui::Slider::new(&mut self.paint_pipeline_config.padding_iterations, 0..=8)
                        .text("Seam Padding"),
                )
                .changed()
            {
                self.is_dirty = true;
            }
            ui.small("Higher values pad farther into UV gutter to reduce seam filtering artifacts.");
        });
    }

    fn show_project_card(&self, ui: &mut egui::Ui, path: &std::path::Path) {
        Self::panel_card().show(ui, |ui| {
            Self::section_header(ui, "PROJECT");
            ui.small(path.display().to_string());
        });
    }

    fn handle_camera_input(&mut self, ui: &egui::Ui, response: &egui::Response) {
        let secondary_down = ui.ctx().input(|i| i.pointer.secondary_down());
        if response.hovered() && secondary_down {
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
    ) {
        let (is_secondary_down, secondary_pressed, secondary_released, is_primary_down) = ui.ctx().input(|i| {
            (
                i.pointer.button_down(egui::PointerButton::Secondary),
                i.pointer.button_pressed(egui::PointerButton::Secondary),
                i.pointer.button_released(egui::PointerButton::Secondary),
                i.pointer.button_down(egui::PointerButton::Primary),
            )
        });
        let is_navigation_drag = response.dragged_by(egui::PointerButton::Secondary);
        let secondary_activity = is_navigation_drag || is_secondary_down || secondary_pressed || secondary_released;
        let is_painting_now = response.hovered()
            && !secondary_activity
            && is_primary_down;
        if secondary_activity {
            self.is_painting_stroke = false;
        }
        let sampled_pressure = Self::current_brush_pressure(ui).map(Self::quantize_pressure);
        if self.use_tablet_pressure {
            if let Some(pressure) = sampled_pressure {
                self.last_brush_pressure = pressure;
                self.tablet_pressure_detected = true;
            }
        } else {
            self.last_brush_pressure = 1.0;
        }
        if is_painting_now && !self.is_painting_stroke {
            self.begin_paint_stroke();
            self.is_painting_stroke = true;
        } else if !is_painting_now {
            self.is_painting_stroke = false;
        }
        self.display_brush_pressure = self.last_brush_pressure;

        if is_painting_now && self.is_painting_stroke {
            if let Some(pointer_pos) = ui.ctx().input(|i| i.pointer.interact_pos()) {
                if !rect.contains(pointer_pos) {
                    self.is_painting_stroke = false;
                }
                let sx = (pointer_pos.x - rect.left()).clamp(0.0, rect.width() - 1.0);
                let sy = (pointer_pos.y - rect.top()).clamp(0.0, rect.height() - 1.0);
                if let Some(pick) = &self.preview_pick_buffer
                    && let Some(sample) = sample_surface_from_buffer(mesh, pick, [sx, sy])
                {
                    let pressure_for_size = if self.use_pressure_for_size {
                        self.last_brush_pressure
                    } else {
                        1.0
                    };
                    let pressure_for_strength = if self.use_pressure_for_strength {
                        self.last_brush_pressure
                    } else {
                        1.0
                    };
                    let brush_size_px = (self.brush_size_px * pressure_for_size).clamp(1.0, 128.0);
                    self.paint_projected_brush(
                        mesh,
                        BrushInput {
                            hit: sample.hit,
                            center_world: glam::vec3(sample.world_pos[0], sample.world_pos[1], sample.world_pos[2]),
                            center_uv: sample.uv,
                        },
                        BrushDispatch {
                            screen_pos: [sx, sy],
                            radius_px: (brush_size_px * 0.5).max(0.5),
                            strength: self.brush_strength,
                            color: self.brush_color.to_array(),
                            pressure: pressure_for_strength,
                            blend_mode: self.brush_blend_mode,
                        },
                    );
                    ui.ctx().request_repaint();
                }
            }
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
        let frame = render_preview_frame(
            mesh,
            self.mesh_center,
            self.mesh_fit_scale,
            self.orbit_yaw,
            self.orbit_pitch,
            self.orbit_distance,
            image_size,
            self.albedo_texture.as_ref(),
        );
        self.preview_pick_buffer = Some(frame.pick);
        self.update_preview_texture(ui.ctx(), frame.image);
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
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            if let Some(path) = &self.last_loaded_path {
                ui.small(format!("Last file: {}", path.display()));
            }
            if let Some(err) = &self.last_error {
                ui.colored_label(egui::Color32::RED, format!("Load error: {err}"));
            }
        });
    }

    fn draw_viewport_backdrop(&self, painter: &egui::Painter, rect: egui::Rect, hovered: bool) {
        let base = egui::Color32::from_rgb(32, 34, 40);
        let tint = if hovered {
            egui::Color32::from_rgba_unmultiplied(119, 156, 224, 24)
        } else {
            egui::Color32::from_rgba_unmultiplied(0, 0, 0, 0)
        };
        painter.rect_filled(rect, 8.0, base);
        painter.rect_filled(rect, 8.0, tint);
        let border_color = if hovered {
            egui::Color32::from_rgb(106, 141, 206)
        } else {
            egui::Color32::from_rgb(66, 72, 86)
        };
        painter.rect_stroke(
            rect,
            8.0,
            egui::Stroke::new(1.0, border_color),
            egui::StrokeKind::Outside,
        );
        painter.rect_stroke(
            rect.shrink(1.0),
            7.0,
            egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 20)),
            egui::StrokeKind::Outside,
        );
    }

    fn draw_viewport_overlay(&self, ui: &egui::Ui, painter: &egui::Painter, rect: egui::Rect) {
        let center = rect.center();
        let crosshair = 6.0;
        painter.line_segment(
            [
                egui::pos2(center.x - crosshair, center.y),
                egui::pos2(center.x + crosshair, center.y),
            ],
            egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(230, 235, 245, 120)),
        );
        painter.line_segment(
            [
                egui::pos2(center.x, center.y - crosshair),
                egui::pos2(center.x, center.y + crosshair),
            ],
            egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(230, 235, 245, 120)),
        );

        let corner = 16.0;
        let corner_stroke = egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(210, 220, 240, 70));
        painter.line_segment(
            [rect.left_top(), egui::pos2(rect.left() + corner, rect.top())],
            corner_stroke,
        );
        painter.line_segment(
            [rect.left_top(), egui::pos2(rect.left(), rect.top() + corner)],
            corner_stroke,
        );
        painter.line_segment(
            [rect.right_top(), egui::pos2(rect.right() - corner, rect.top())],
            corner_stroke,
        );
        painter.line_segment(
            [rect.right_top(), egui::pos2(rect.right(), rect.top() + corner)],
            corner_stroke,
        );
        painter.line_segment(
            [rect.left_bottom(), egui::pos2(rect.left() + corner, rect.bottom())],
            corner_stroke,
        );
        painter.line_segment(
            [rect.left_bottom(), egui::pos2(rect.left(), rect.bottom() - corner)],
            corner_stroke,
        );
        painter.line_segment(
            [rect.right_bottom(), egui::pos2(rect.right() - corner, rect.bottom())],
            corner_stroke,
        );
        painter.line_segment(
            [rect.right_bottom(), egui::pos2(rect.right(), rect.bottom() - corner)],
            corner_stroke,
        );

        let pressure_mode = if self.use_tablet_pressure { "tablet" } else { "fixed" };
        let overlay = format!(
            "S {:.0}px  T {:.2}  P {:.2} ({})  Z {:.2}",
            self.brush_size_px,
            self.brush_strength,
            self.display_brush_pressure,
            pressure_mode,
            self.orbit_distance
        );
        painter.text(
            rect.left_top() + egui::vec2(10.0, 10.0),
            egui::Align2::LEFT_TOP,
            overlay,
            egui::TextStyle::Small.resolve(ui.style()),
            egui::Color32::from_rgb(230, 236, 245),
        );
    }
}
