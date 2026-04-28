use eframe::egui::{self, TextureHandle};
use glam::Vec3;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::thread::JoinHandle;
use std::time::Instant;

mod app;
mod io;
mod platform;
mod renderer;
mod ui;

use app::PendingLoadAction;
use image::RgbaImage;
use io::mesh_loader::MeshData;
use renderer::{
    BrushBlendMode, BrushDispatch, BrushFalloff, BrushInput, PaintPipelineConfig, ScreenPickBuffer,
    UvCoverageCache,
};

enum PaintWorkerCommand {
    Stamp { input: BrushInput, dispatch: BrushDispatch },
    Finish,
    Abort,
}

enum PaintWorkerEvent {
    Preview(RgbaImage),
    Finished(RgbaImage),
}

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
    viewport_frame_size: [usize; 2],
    viewport_needs_refresh: bool,
    show_wireframe_overlay: bool,
    paint_pipeline_config: PaintPipelineConfig,
    uv_coverage_cache: Option<UvCoverageCache>,
    brush_size_px: f32,
    brush_strength: f32,
    brush_color: egui::Color32,
    brush_blend_mode: BrushBlendMode,
    brush_falloff: BrushFalloff,
    brush_sample_distance_px: f32,
    use_tablet_pressure: bool,
    use_pressure_for_size: bool,
    use_pressure_for_strength: bool,
    tablet_pressure_detected: bool,
    last_brush_pressure: f32,
    display_brush_pressure: f32,
    undo_stack: VecDeque<RgbaImage>,
    redo_stack: VecDeque<RgbaImage>,
    is_painting_stroke: bool,
    last_paint_sample_screen_pos: Option<[f32; 2]>,
    paint_worker_tx: Option<Sender<PaintWorkerCommand>>,
    paint_worker_rx: Option<Receiver<PaintWorkerEvent>>,
    paint_worker_join: Option<JoinHandle<()>>,
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
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        if let Some(state) = frame.wgpu_render_state() {
            crate::renderer::gpu_paint::install_shared_runtime(state.device.clone(), state.queue.clone());
        }
        #[cfg(target_os = "windows")]
        crate::platform::windows_pen::install(frame);
        #[cfg(target_os = "windows")]
        crate::platform::windows_wintab::install(frame);
        self.poll_paint_worker();
        if self.paint_worker_rx.is_some() {
            ui.ctx().request_repaint();
        }
        self.apply_modern_theme(ui.ctx());
        self.handle_shortcuts(ui.ctx());
        self.show_toolbar_panel(ui);
        self.show_status_bar_panel(ui);
        self.show_left_panel(ui);
        self.show_right_panel(ui);
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
