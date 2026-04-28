use eframe::egui::{self, TextureHandle};
use glam::Vec3;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Instant;

mod app;
mod io;
mod renderer;
mod ui;

use app::PendingLoadAction;
use image::RgbaImage;
use io::mesh_loader::MeshData;
use renderer::{BrushBlendMode, PaintPipelineConfig, ScreenPickBuffer, UvCoverageCache};

struct PinturappUi {
    loaded_mesh: Option<MeshData>,
    last_loaded_path: Option<PathBuf>,
    loaded_texture_path: Option<PathBuf>,
    last_error: Option<String>,
    orbit_yaw: f32,
    orbit_pitch: f32,
    orbit_distance: f32,
    mesh_center: Vec3,
    mesh_fit_scale: f32,
    albedo_texture: Option<RgbaImage>,
    preview_texture: Option<TextureHandle>,
    preview_pick_buffer: Option<ScreenPickBuffer>,
    paint_pipeline_config: PaintPipelineConfig,
    uv_coverage_cache: Option<UvCoverageCache>,
    brush_radius_px: f32,
    brush_color: egui::Color32,
    brush_blend_mode: BrushBlendMode,
    use_tablet_pressure: bool,
    pressure_smoothing: f32,
    last_brush_pressure: f32,
    display_brush_pressure: f32,
    undo_stack: VecDeque<RgbaImage>,
    redo_stack: VecDeque<RgbaImage>,
    is_painting_stroke: bool,
    current_project_path: Option<PathBuf>,
    recent_projects: Vec<PathBuf>,
    storage_dir: PathBuf,
    autosave_path: PathBuf,
    last_autosave_at: Instant,
    last_autosave_ok_at: Option<Instant>,
    is_dirty: bool,
    pending_load_action: Option<PendingLoadAction>,
    show_discard_confirm: bool,
    show_autosave_recovery_prompt: bool,
    show_welcome_overlay: bool,
    theme_applied: bool,
}

impl eframe::App for PinturappUi {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.apply_modern_theme(ui.ctx());
        self.handle_shortcuts(ui.ctx());
        self.show_toolbar_panel(ui);
        self.show_status_bar_panel(ui);
        self.show_left_panel(ui);
        self.show_viewport_panel(ui);
        self.show_welcome_overlay_if_needed(ui.ctx());
        self.show_discard_confirm_dialog(ui.ctx());
        self.show_autosave_recovery_dialog(ui.ctx());
        self.maybe_autosave();
    }
}

impl PinturappUi {
    const MAX_HISTORY: usize = 20;
}

fn main() -> eframe::Result<()> {
    app::run()
}
