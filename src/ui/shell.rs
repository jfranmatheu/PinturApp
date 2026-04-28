use crate::PinturappUi;
use crate::app::PendingLoadAction;
use eframe::egui::{self, ColorImage, TextureOptions};

impl PinturappUi {
    pub(crate) fn panel_card() -> egui::Frame {
        egui::Frame::default()
            .fill(egui::Color32::from_rgb(32, 35, 41))
            .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(58, 62, 74)))
            .corner_radius(egui::CornerRadius::same(6))
            .inner_margin(egui::Margin::same(10))
    }

    pub(crate) fn apply_modern_theme(&mut self, ctx: &egui::Context) {
        if self.theme_applied {
            return;
        }
        ctx.set_visuals(egui::Visuals::dark());
        let mut style = (*ctx.global_style()).clone();
        style.spacing.item_spacing = egui::vec2(7.0, 7.0);
        style.spacing.button_padding = egui::vec2(9.0, 5.0);
        style.spacing.menu_margin = egui::Margin::same(6);
        style.spacing.indent = 14.0;
        style.spacing.slider_width = 150.0;
        style.visuals.panel_fill = egui::Color32::from_rgb(36, 39, 46);
        style.visuals.window_fill = egui::Color32::from_rgb(42, 45, 52);
        style.visuals.window_stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(68, 72, 84));
        style.visuals.faint_bg_color = egui::Color32::from_rgb(47, 51, 60);
        style.visuals.extreme_bg_color = egui::Color32::from_rgb(24, 26, 31);
        style.visuals.code_bg_color = egui::Color32::from_rgb(30, 33, 38);
        style.visuals.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(39, 42, 49);
        style.visuals.widgets.noninteractive.fg_stroke =
            egui::Stroke::new(1.0, egui::Color32::from_rgb(176, 183, 198));
        style.visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(54, 58, 68);
        style.visuals.widgets.inactive.fg_stroke =
            egui::Stroke::new(1.0, egui::Color32::from_rgb(194, 199, 210));
        style.visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(67, 76, 96);
        style.visuals.widgets.hovered.bg_stroke =
            egui::Stroke::new(1.0, egui::Color32::from_rgb(118, 150, 214));
        style.visuals.widgets.hovered.fg_stroke =
            egui::Stroke::new(1.0, egui::Color32::from_rgb(228, 232, 242));
        style.visuals.widgets.active.bg_fill = egui::Color32::from_rgb(80, 95, 128);
        style.visuals.widgets.active.bg_stroke =
            egui::Stroke::new(1.0, egui::Color32::from_rgb(131, 169, 238));
        style.visuals.selection.bg_fill = egui::Color32::from_rgb(79, 124, 205);
        style.visuals.selection.stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(222, 230, 244));
        style.text_styles.insert(
            egui::TextStyle::Small,
            egui::FontId::new(11.0, egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Body,
            egui::FontId::new(12.5, egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Button,
            egui::FontId::new(12.0, egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Heading,
            egui::FontId::new(16.0, egui::FontFamily::Proportional),
        );
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
