use crate::PinturappUi;
use crate::io::mesh_loader::load_obj;
use crate::renderer::compute_mesh_fit;

impl PinturappUi {
    pub(crate) fn save_texture_to_file(&mut self) {
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

    pub(crate) fn pick_and_load_obj(&mut self) {
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

    pub(crate) fn pick_and_load_texture(&mut self) {
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
}
