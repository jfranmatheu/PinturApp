use crate::PinturappUi;
use crate::app::PendingLoadAction;
use eframe::egui;

impl PinturappUi {
    pub(crate) fn show_discard_confirm_dialog(&mut self, ctx: &egui::Context) {
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

    pub(crate) fn show_autosave_recovery_dialog(&mut self, ctx: &egui::Context) {
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
}
