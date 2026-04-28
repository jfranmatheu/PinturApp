use crate::PinturappUi;
use crate::app::PendingLoadAction;
use eframe::egui::{self, ColorImage, TextureOptions};

impl PinturappUi {
    pub(crate) fn panel_card() -> egui::Frame {
        egui::Frame::default()
            .fill(egui::Color32::from_rgb(24, 30, 40))
            .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(45, 58, 78)))
            .corner_radius(egui::CornerRadius::same(8))
            .inner_margin(egui::Margin::same(10))
    }

    pub(crate) fn apply_modern_theme(&mut self, ctx: &egui::Context) {
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

    pub(crate) fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let (new_project, load_project, load_autosave, save_project, save_project_as, undo, redo) =
            ctx.input(|i| {
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

    pub(crate) fn update_preview_texture(&mut self, ctx: &egui::Context, image: ColorImage) {
        if let Some(texture) = &mut self.preview_texture {
            texture.set(image, TextureOptions::LINEAR);
        } else {
            self.preview_texture = Some(ctx.load_texture("mesh_preview", image, TextureOptions::LINEAR));
        }
    }
}
