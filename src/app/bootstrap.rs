use crate::PinturappUi;
use crate::app::{default_storage_dir, load_recent_projects};
use crate::renderer::{BrushBlendMode, BrushFalloff};
use eframe::CreationContext;
use eframe::egui;
use glam::Vec3;
use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

fn discover_hdri_options() -> Vec<PathBuf> {
    let hdri_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src").join("resources").join("hdri");
    let mut files = Vec::new();
    if let Ok(entries) = fs::read_dir(hdri_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let is_exr = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case("exr"))
                .unwrap_or(false);
            if is_exr {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

impl Default for PinturappUi {
    fn default() -> Self {
        let storage_dir = default_storage_dir();
        let _ = fs::create_dir_all(storage_dir.join("unpacked"));
        let recent_projects = load_recent_projects(&storage_dir);
        let autosave_path = storage_dir.join("autosave.pinturaproj");
        let show_autosave_recovery_prompt = autosave_path.exists();
        let hdri_options = discover_hdri_options();
        let selected_hdri_index = 0;
        let hdri_map = hdri_options
            .get(selected_hdri_index)
            .and_then(|path| crate::renderer::load_hdri_map(path).ok());
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
            gpu_albedo_snapshot: None,
            viewport_frame_size: [0, 0],
            viewport_needs_refresh: true,
            show_wireframe_overlay: false,
            show_lighting: true,
            hdri_rotation: 0.0,
            hdri_options,
            selected_hdri_index,
            hdri_map,
            paint_pipeline_config: Default::default(),
            uv_coverage_cache: None,
            brush_size_px: 24.0,
            brush_strength: 1.0,
            brush_color: egui::Color32::from_rgba_unmultiplied(255, 90, 90, 255),
            brush_blend_mode: BrushBlendMode::Normal,
            brush_falloff: BrushFalloff::Smooth,
            brush_sample_distance_px: 2.0,
            use_tablet_pressure: true,
            use_pressure_for_size: true,
            use_pressure_for_strength: true,
            tablet_pressure_detected: false,
            last_brush_pressure: 1.0,
            display_brush_pressure: 1.0,
            undo_stack: VecDeque::new(),
            redo_stack: VecDeque::new(),
            is_painting_stroke: false,
            last_paint_sample_screen_pos: None,
            paint_worker_tx: None,
            paint_worker_rx: None,
            paint_worker_join: None,
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
            wgpu_target_format: None,
        }
    }
}

impl PinturappUi {
    pub(crate) fn new(cc: &CreationContext<'_>) -> Self {
        if let Some(state) = cc.wgpu_render_state.as_ref() {
            crate::renderer::gpu_paint::install_shared_runtime(state.device.clone(), state.queue.clone());
        }
        Self::default()
    }
}
