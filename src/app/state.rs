use crate::renderer::BrushBlendMode;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectState {
    pub mesh_entry: Option<String>,
    pub texture_entry: Option<String>,
    pub orbit_yaw: f32,
    pub orbit_pitch: f32,
    pub orbit_distance: f32,
    #[serde(default = "default_brush_size_px", alias = "brush_radius_px")]
    pub brush_size_px: f32,
    #[serde(default = "default_brush_strength")]
    pub brush_strength: f32,
    pub brush_color_rgba: [u8; 4],
    #[serde(default = "default_brush_blend_mode")]
    pub brush_blend_mode: BrushBlendMode,
    #[serde(default = "default_use_tablet_pressure")]
    pub use_tablet_pressure: bool,
    #[serde(default = "default_use_pressure_for_size")]
    pub use_pressure_for_size: bool,
    #[serde(default = "default_use_pressure_for_strength")]
    pub use_pressure_for_strength: bool,
    #[serde(default = "default_seam_padding_iterations")]
    pub seam_padding_iterations: usize,
}

fn default_seam_padding_iterations() -> usize {
    2
}

fn default_brush_blend_mode() -> BrushBlendMode {
    BrushBlendMode::Normal
}

fn default_brush_size_px() -> f32 {
    24.0
}

fn default_brush_strength() -> f32 {
    1.0
}

fn default_use_tablet_pressure() -> bool {
    true
}

fn default_use_pressure_for_size() -> bool {
    true
}

fn default_use_pressure_for_strength() -> bool {
    true
}

#[derive(Debug, Clone)]
pub enum PendingLoadAction {
    NewProject,
    OpenProjectPicker,
    LoadProject(PathBuf),
    LoadAutosave,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct RecentProjects {
    paths: Vec<String>,
}

pub fn default_storage_dir() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".pinturapp")
}

fn recent_projects_path(storage_dir: &Path) -> PathBuf {
    storage_dir.join("recent_projects.json")
}

pub fn load_recent_projects(storage_dir: &Path) -> Vec<PathBuf> {
    let path = recent_projects_path(storage_dir);
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(recent) = serde_json::from_str::<RecentProjects>(&text) else {
        return Vec::new();
    };
    recent.paths.into_iter().map(PathBuf::from).collect()
}

pub fn save_recent_projects(storage_dir: &Path, paths: &[PathBuf]) -> Result<(), String> {
    let recent = RecentProjects {
        paths: paths
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect(),
    };
    let text = serde_json::to_string_pretty(&recent)
        .map_err(|e| format!("Failed to serialize recent projects: {e}"))?;
    fs::write(recent_projects_path(storage_dir), text)
        .map_err(|e| format!("Failed to save recent projects: {e}"))
}

pub fn unix_timestamp_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
