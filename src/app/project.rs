use crate::PinturappUi;
use crate::app::{PendingLoadAction, ProjectState, save_recent_projects, unix_timestamp_secs};
use crate::io::mesh_loader::load_obj;
use crate::renderer::compute_mesh_fit;
use eframe::egui;
use glam::Vec3;
use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use zip::write::FileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

impl PinturappUi {
    fn project_state(&self) -> ProjectState {
        ProjectState {
            mesh_entry: None,
            texture_entry: None,
            orbit_yaw: self.orbit_yaw,
            orbit_pitch: self.orbit_pitch,
            orbit_distance: self.orbit_distance,
            brush_radius_px: self.brush_radius_px,
            brush_color_rgba: self.brush_color.to_array(),
            brush_blend_mode: self.brush_blend_mode,
            use_tablet_pressure: self.use_tablet_pressure,
            pressure_smoothing: self.pressure_smoothing,
            seam_padding_iterations: self.paint_pipeline_config.padding_iterations,
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
        self.brush_blend_mode = state.brush_blend_mode;
        self.use_tablet_pressure = state.use_tablet_pressure;
        self.pressure_smoothing = state.pressure_smoothing.clamp(0.0, 1.0);
        self.paint_pipeline_config.padding_iterations = state.seam_padding_iterations.clamp(0, 8);

        self.loaded_mesh = None;
        self.last_loaded_path = None;
        self.loaded_texture_path = None;
        self.albedo_texture = None;
        self.uv_coverage_cache = None;

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

    pub(crate) fn save_project_to_file(&mut self) {
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

    pub(crate) fn save_project_as_to_file(&mut self) {
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

    pub(crate) fn request_load_action(&mut self, action: PendingLoadAction) {
        if self.is_dirty {
            self.pending_load_action = Some(action);
            self.show_discard_confirm = true;
            return;
        }
        self.execute_load_action(action);
    }

    pub(crate) fn execute_load_action(&mut self, action: PendingLoadAction) {
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

    fn clear_session(&mut self) {
        self.loaded_mesh = None;
        self.last_loaded_path = None;
        self.loaded_texture_path = None;
        self.albedo_texture = None;
        self.uv_coverage_cache = None;
        self.preview_texture = None;
        self.orbit_yaw = 0.5;
        self.orbit_pitch = 0.25;
        self.orbit_distance = 3.0;
        self.mesh_center = Vec3::ZERO;
        self.mesh_fit_scale = 1.0;
        self.paint_pipeline_config = Default::default();
        self.brush_blend_mode = Default::default();
        self.use_tablet_pressure = true;
        self.tablet_pressure_detected = false;
        self.pressure_smoothing = 0.25;
        self.last_brush_pressure = 1.0;
        self.display_brush_pressure = 1.0;
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
        let mut archive = ZipArchive::new(file).map_err(|e| format!("Invalid project bundle (zip): {e}"))?;

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
                fs::create_dir_all(&out_path)
                    .map_err(|e| format!("Failed to create bundle directory {}: {e}", out_path.display()))?;
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
        self.storage_dir.join("unpacked").join(format!("{stem}_{stamp}"))
    }

    pub(crate) fn maybe_autosave(&mut self) {
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

    pub(crate) fn autosave_status_text(&self) -> String {
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

    pub(crate) fn clear_recent_projects(&mut self) {
        self.recent_projects.clear();
        let _ = save_recent_projects(&self.storage_dir, &self.recent_projects);
    }
}
