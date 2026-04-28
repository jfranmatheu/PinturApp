use crate::PinturappUi;
use crate::app::PendingLoadAction;
use eframe::egui;

impl PinturappUi {
    fn welcome_surface(opacity: f32) -> egui::Frame {
        let alpha = (255.0 * opacity.clamp(0.0, 1.0)).round() as u8;
        egui::Frame::default()
            .fill(egui::Color32::from_rgba_unmultiplied(20, 25, 35, alpha))
            .stroke(egui::Stroke::new(
                1.0,
                egui::Color32::from_rgba_unmultiplied(58, 74, 98, alpha),
            ))
            .corner_radius(egui::CornerRadius::same(12))
            .inner_margin(egui::Margin::same(14))
    }

    pub(crate) fn show_welcome_overlay_if_needed(&mut self, ctx: &egui::Context) {
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
        let fade_t = ctx.animate_bool(egui::Id::new("welcome_overlay_fade"), true);
        if fade_t < 1.0 {
            ctx.request_repaint();
        }
        let panel_rect =
            egui::Rect::from_center_size(screen_rect.center(), panel_size * (0.98 + fade_t * 0.02));
        let mut dismiss_overlay = false;

        egui::Area::new("welcome_overlay".into())
            .order(egui::Order::Foreground)
            .fixed_pos(screen_rect.min)
            .interactable(true)
            .show(ctx, |ui| {
                ui.set_min_size(screen_rect.size());
                let full_rect = ui.max_rect();
                let backdrop = ui.allocate_rect(full_rect, egui::Sense::click());
                ui.painter().rect_filled(
                    full_rect,
                    0.0,
                    egui::Color32::from_black_alpha((170.0 * fade_t).round() as u8),
                );

                ui.scope_builder(egui::UiBuilder::new().max_rect(panel_rect), |ui| {
                    Self::welcome_surface(fade_t).show(ui, |ui| {
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
                                for (row_idx, path) in recent.iter().take(10).enumerate() {
                                    let label = path
                                        .file_name()
                                        .map(|n| n.to_string_lossy().to_string())
                                        .unwrap_or_else(|| path.display().to_string());
                                    let row_rect = cols[1]
                                        .allocate_exact_size(
                                            egui::vec2(cols[1].available_width(), 26.0),
                                            egui::Sense::click(),
                                        )
                                        .0;
                                    let response = cols[1].interact(
                                        row_rect,
                                        cols[1].id().with(path),
                                        egui::Sense::click(),
                                    );
                                    let hover_t = ctx.animate_bool(response.id, response.hovered());

                                    if hover_t > 0.0 {
                                        cols[1].painter().rect_filled(
                                            row_rect,
                                            4.0,
                                            egui::Color32::from_rgba_unmultiplied(
                                                84,
                                                112,
                                                146,
                                                (48.0 * hover_t).round() as u8,
                                            ),
                                        );
                                    }
                                    cols[1].painter().text(
                                        row_rect.left_center() + egui::vec2(8.0, 0.0),
                                        egui::Align2::LEFT_CENTER,
                                        label,
                                        egui::FontId::proportional(14.0),
                                        egui::Color32::from_rgb(
                                            egui::lerp(205.0..=236.0, hover_t).round() as u8,
                                            egui::lerp(216.0..=244.0, hover_t).round() as u8,
                                            egui::lerp(230.0..=252.0, hover_t).round() as u8,
                                        ),
                                    );
                                    if row_idx + 1 < recent.len().min(10) {
                                        let y = row_rect.bottom() - 0.5;
                                        cols[1].painter().line_segment(
                                            [
                                                egui::pos2(row_rect.left() + 8.0, y),
                                                egui::pos2(row_rect.right() - 8.0, y),
                                            ],
                                            egui::Stroke::new(
                                                1.0,
                                                egui::Color32::from_rgba_unmultiplied(
                                                    140, 156, 178, 26,
                                                ),
                                            ),
                                        );
                                    }
                                    if response.clicked() {
                                        self.request_load_action(PendingLoadAction::LoadProject(
                                            path.clone(),
                                        ));
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
}
