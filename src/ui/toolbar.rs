use crate::PinturappUi;
use crate::app::PendingLoadAction;
use eframe::egui;

impl PinturappUi {
    pub(crate) fn show_toolbar_panel(&mut self, ui: &mut egui::Ui) {
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
                                    self.request_load_action(PendingLoadAction::LoadProject(
                                        path.clone(),
                                    ));
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
    }
}
