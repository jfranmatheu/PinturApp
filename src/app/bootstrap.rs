use crate::PinturappUi;
use crate::app::{default_storage_dir, load_recent_projects};
use crate::renderer::BrushBlendMode;
use eframe::egui;
use glam::Vec3;
use std::collections::VecDeque;
use std::fs;
use std::time::Instant;

impl Default for PinturappUi {
    fn default() -> Self {
        let storage_dir = default_storage_dir();
        let _ = fs::create_dir_all(storage_dir.join("unpacked"));
        let recent_projects = load_recent_projects(&storage_dir);
        let autosave_path = storage_dir.join("autosave.pinturaproj");
        let show_autosave_recovery_prompt = autosave_path.exists();
        Self {
            loaded_mesh: None,
            last_loaded_path: None,
            loaded_texture_path: None,
            last_error: None,
            orbit_yaw: 0.5,
            orbit_pitch: 0.25,
            orbit_distance: 3.0,
            mesh_center: Vec3::ZERO,
            mesh_fit_scale: 1.0,
            albedo_texture: None,
            preview_texture: None,
            preview_pick_buffer: None,
            paint_pipeline_config: Default::default(),
            uv_coverage_cache: None,
            brush_radius_px: 12.0,
            brush_color: egui::Color32::from_rgba_unmultiplied(255, 90, 90, 255),
            brush_blend_mode: BrushBlendMode::Normal,
            use_tablet_pressure: true,
            tablet_pressure_detected: false,
            last_brush_pressure: 1.0,
            display_brush_pressure: 1.0,
            undo_stack: VecDeque::new(),
            redo_stack: VecDeque::new(),
            is_painting_stroke: false,
            current_project_path: None,
            recent_projects,
            storage_dir,
            autosave_path,
            last_autosave_at: Instant::now(),
            last_autosave_ok_at: None,
            is_dirty: false,
            pending_load_action: None,
            show_discard_confirm: false,
            show_autosave_recovery_prompt,
            show_welcome_overlay: true,
            theme_applied: false,
        }
    }
}
