use crate::PinturappUi;
use crate::renderer::{
    BrushBlendMode, BrushDispatch, BrushFalloff, BrushInput, draw_mesh_wireframe, enqueue_gpu_viewport,
    load_hdri_map,
    render_preview_frame,
    sample_surface_from_buffer,
};
use eframe::egui;

impl PinturappUi {
    fn target_preview_size(viewport_size: egui::Vec2) -> [usize; 2] {
        // Keep software rasterization cost bounded at fullscreen sizes.
        const MAX_DIM: f32 = 1280.0;
        let mut w = viewport_size.x.max(1.0);
        let mut h = viewport_size.y.max(1.0);
        let max_side = w.max(h);
        if max_side > MAX_DIM {
            let scale = MAX_DIM / max_side;
            w = (w * scale).max(1.0);
            h = (h * scale).max(1.0);
        }
        [w.round() as usize, h.round() as usize]
    }

    fn quantize_pressure(value: f32) -> f32 {
        ((value.clamp(0.0, 1.0) * 100.0).round() / 100.0).clamp(0.0, 1.0)
    }

    fn falloff_label(mode: BrushFalloff) -> &'static str {
        match mode {
            BrushFalloff::Smooth => "Smooth",
            BrushFalloff::Sphere => "Sphere",
            BrushFalloff::Root => "Root",
            BrushFalloff::Sharp => "Sharp",
            BrushFalloff::Linear => "Linear",
            BrushFalloff::Constant => "Constant",
        }
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

    fn current_hdri_label(&self) -> String {
        self.hdri_options
            .get(self.selected_hdri_index)
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
            .unwrap_or("None")
            .to_owned()
    }

    fn set_hdri_selection(&mut self, index: usize) {
        if index >= self.hdri_options.len() {
            return;
        }
        self.selected_hdri_index = index;
        match load_hdri_map(&self.hdri_options[index]) {
            Ok(map) => {
                self.hdri_map = Some(map);
                self.last_error = None;
                self.viewport_needs_refresh = true;
                self.is_dirty = true;
            }
            Err(err) => {
                self.last_error = Some(err);
            }
        }
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
                        "tablet/no-signal".to_string()
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
                self.show_brush_card(ui);
                ui.add_space(6.0);
                self.show_color_card(ui);
                ui.add_space(6.0);
                self.show_pipeline_card(ui);
            });
    }

    pub(crate) fn show_right_panel(&mut self, ui: &mut egui::Ui) {
        egui::Panel::right("material_panel")
            .resizable(true)
            .default_size(220.0)
            .frame(
                egui::Frame::default()
                    .fill(egui::Color32::from_rgb(39, 42, 49))
                    .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(72, 78, 90)))
                    .inner_margin(egui::Margin::same(8)),
            )
            .show_inside(ui, |ui| {
                Self::section_header(ui, "MATERIAL SETTINGS");
                self.show_material_card(ui);
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
                ui.separator();
                ui.label(if self.show_lighting { "Shading: HDRI" } else { "Shading: Flat" });
                if ui.checkbox(&mut self.show_lighting, "HDRI Lighting").changed() {
                    self.viewport_needs_refresh = true;
                }
                let mut changed_hdri = None;
                egui::ComboBox::from_id_salt("viewport_hdri_select")
                    .selected_text(self.current_hdri_label())
                    .show_ui(ui, |ui| {
                        for (idx, path) in self.hdri_options.iter().enumerate() {
                            let label = path
                                .file_name()
                                .and_then(|name| name.to_str())
                                .unwrap_or("Unnamed");
                            if ui.selectable_label(idx == self.selected_hdri_index, label).clicked() {
                                changed_hdri = Some(idx);
                            }
                        }
                    });
                if let Some(idx) = changed_hdri {
                    self.set_hdri_selection(idx);
                }
                if ui
                    .checkbox(&mut self.show_wireframe_overlay, "Wireframe Overlay")
                    .changed()
                {
                    self.is_dirty = true;
                }
                ui.separator();
                ui.small("Rotate: Ctrl+Shift+MouseMove");
                ui.small(format!("HDRI: {:.1} deg", self.hdri_rotation.to_degrees()));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.small(format!("U:{}  R:{}", self.undo_stack.len(), self.redo_stack.len()));
                    if ui.button("Redo").clicked() {
                        self.redo_paint();
                    }
                    if ui.button("Undo").clicked() {
                        self.undo_paint();
                    }
                });
            });
            ui.separator();
            if self.loaded_mesh.is_some() {
                let mesh = self
                    .loaded_mesh
                    .take()
                    .expect("loaded_mesh checked as Some above");
                self.show_mesh_details(ui, &mesh);
                ui.separator();
                let available = ui.available_size_before_wrap();
                let viewport_size = egui::vec2(available.x.max(240.0), available.y.max(240.0));
                let (response, painter) = ui.allocate_painter(viewport_size, egui::Sense::drag());
                let rect = response.rect;

                self.draw_viewport_backdrop(&painter, rect, response.hovered());
                self.handle_camera_input(ui, &response);
                let preview_size = Self::target_preview_size(rect.size());
                self.draw_viewport_texture_and_wireframe(ui, &painter, rect, &mesh, preview_size);
                self.handle_paint_input(ui, &response, rect, &mesh);
                self.draw_viewport_overlay(ui, &painter, rect);
                self.loaded_mesh = Some(mesh);
            } else {
                self.is_painting_stroke = false;
                self.preview_pick_buffer = None;
                self.viewport_frame_size = [0, 0];
                self.viewport_needs_refresh = true;
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

            if ui
                .add(
                    egui::Slider::new(&mut self.brush_sample_distance_px, 0.5..=64.0)
                        .text("Sample Distance"),
                )
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

            ui.horizontal_wrapped(|ui| {
                ui.small("Falloff");
                for mode in [
                    BrushFalloff::Smooth,
                    BrushFalloff::Sphere,
                    BrushFalloff::Root,
                    BrushFalloff::Sharp,
                    BrushFalloff::Linear,
                    BrushFalloff::Constant,
                ] {
                    if ui
                        .selectable_value(&mut self.brush_falloff, mode, Self::falloff_label(mode))
                        .changed()
                    {
                        self.is_dirty = true;
                    }
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
            if ui
                .checkbox(
                    &mut self.paint_pipeline_config.use_gpu_compute_experimental,
                    "GPU Compute (Experimental)",
                )
                .changed()
            {
                self.is_dirty = true;
            }
            ui.small("Higher values pad farther into UV gutter to reduce seam filtering artifacts.");
            ui.small("GPU compute path paints in UV-space on GPU and falls back to CPU if unavailable.");
        });
    }

    fn handle_camera_input(&mut self, ui: &egui::Ui, response: &egui::Response) {
        let hdri_drag = ui.ctx().input(|i| {
            response.hovered() && i.modifiers.ctrl && i.modifiers.shift && i.pointer.primary_down()
        });
        if hdri_drag {
            let delta = ui.ctx().input(|i| i.pointer.delta());
            self.hdri_rotation += delta.x * 0.01;
            self.is_dirty = true;
            self.viewport_needs_refresh = true;
            return;
        }

        let secondary_down = ui.ctx().input(|i| i.pointer.secondary_down());
        if response.hovered() && secondary_down {
            let delta = ui.ctx().input(|i| i.pointer.delta());
            self.orbit_yaw += delta.x * 0.01;
            self.orbit_pitch = (self.orbit_pitch + delta.y * 0.01).clamp(-1.4, 1.4);
            self.is_dirty = true;
            self.viewport_needs_refresh = true;
        }

        if response.hovered() {
            let scroll = ui.ctx().input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() > f32::EPSILON {
                let zoom_factor = (1.0_f32 - scroll * 0.0015_f32).clamp(0.80_f32, 1.25_f32);
                self.orbit_distance = (self.orbit_distance * zoom_factor).clamp(0.25, 50.0);
                self.is_dirty = true;
                self.viewport_needs_refresh = true;
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
            self.end_paint_stroke();
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
            self.end_paint_stroke();
        }
        self.display_brush_pressure = self.last_brush_pressure;

        if is_painting_now && self.is_painting_stroke {
            if let Some(pointer_pos) = ui.ctx().input(|i| i.pointer.interact_pos()) {
                if !rect.contains(pointer_pos) {
                    self.end_paint_stroke();
                    return;
                }
                let sx = (pointer_pos.x - rect.left()).clamp(0.0, rect.width() - 1.0);
                let sy = (pointer_pos.y - rect.top()).clamp(0.0, rect.height() - 1.0);
                let spacing = self.brush_sample_distance_px.max(0.5);
                let mut painted = false;

                if self.last_paint_sample_screen_pos.is_none() {
                    painted |= self.paint_stamp_at_screen(rect, mesh, sx, sy);
                    self.last_paint_sample_screen_pos = Some([sx, sy]);
                }

                if let Some(prev) = self.last_paint_sample_screen_pos {
                    let dx = sx - prev[0];
                    let dy = sy - prev[1];
                    let distance = (dx * dx + dy * dy).sqrt();
                    if distance >= spacing {
                        // Fill the entire path between frames with evenly spaced dabs.
                        let mut traveled = spacing;
                        let mut steps = 0_u32;
                        while traveled <= distance + 1e-4 && steps < 1024 {
                            let t = traveled / distance;
                            let px = prev[0] + dx * t;
                            let py = prev[1] + dy * t;
                            painted |= self.paint_stamp_at_screen(rect, mesh, px, py);
                            traveled += spacing;
                            steps += 1;
                        }
                    }
                    self.last_paint_sample_screen_pos = Some([sx, sy]);
                }

                if painted {
                    ui.ctx().request_repaint();
                }
            }
        }
    }

    fn paint_stamp_at_screen(
        &mut self,
        rect: egui::Rect,
        mesh: &crate::io::mesh_loader::MeshData,
        sx: f32,
        sy: f32,
    ) -> bool {
        let sample = if let Some(pick) = &self.preview_pick_buffer {
            let pick_w = pick.size[0].max(1) as f32;
            let pick_h = pick.size[1].max(1) as f32;
            let pick_x = ((sx / rect.width().max(1.0)) * pick_w).clamp(0.0, pick_w - 1.0);
            let pick_y = ((sy / rect.height().max(1.0)) * pick_h).clamp(0.0, pick_h - 1.0);
            sample_surface_from_buffer(mesh, pick, [pick_x, pick_y])
        } else {
            None
        };

        let Some(sample) = sample else {
            return false;
        };

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
                falloff: self.brush_falloff,
            },
        );
        true
    }

    fn draw_viewport_texture_and_wireframe(
        &mut self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        rect: egui::Rect,
        mesh: &crate::io::mesh_loader::MeshData,
        image_size: [usize; 2],
    ) {
        self.ensure_gpu_snapshot_ready_for_mesh(mesh);
        let can_draw_gpu_viewport = self.paint_pipeline_config.use_gpu_compute_experimental
            && self.gpu_albedo_snapshot.is_some()
            && self.wgpu_target_format.is_some()
            && self.hdri_map.is_some();
        if can_draw_gpu_viewport {
            let should_refresh_pick = self.preview_pick_buffer.is_none()
                || self.viewport_frame_size != image_size
                || self.viewport_needs_refresh;
            if should_refresh_pick {
                let frame = render_preview_frame(
                    mesh,
                    self.mesh_center,
                    self.mesh_fit_scale,
                    self.orbit_yaw,
                    self.orbit_pitch,
                    self.orbit_distance,
                    image_size,
                    None,
                );
                self.preview_pick_buffer = Some(frame.pick);
                self.viewport_frame_size = image_size;
                self.viewport_needs_refresh = false;
            }
            if let (Some(snapshot), Some(target_format), Some(hdri_map)) =
                (self.gpu_albedo_snapshot.as_ref(), self.wgpu_target_format, self.hdri_map.as_ref())
            {
                if enqueue_gpu_viewport(
                    painter,
                    rect,
                    mesh,
                    self.mesh_center,
                    self.mesh_fit_scale,
                    self.orbit_yaw,
                    self.orbit_pitch,
                    self.orbit_distance,
                    snapshot,
                    target_format,
                    self.show_lighting,
                    self.hdri_rotation,
                    hdri_map,
                ) {
                    ui.ctx().request_repaint();
                }
            }
        } else {
        let should_render = self.preview_texture.is_none()
            || self.preview_pick_buffer.is_none()
            || self.viewport_frame_size != image_size
            || self.viewport_needs_refresh;
        if should_render {
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
            self.viewport_frame_size = image_size;
            self.viewport_needs_refresh = false;
        }
        if let Some(texture) = &self.preview_texture {
            painter.image(
                texture.id(),
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        }

        if self.show_wireframe_overlay {
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
