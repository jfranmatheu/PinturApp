pub mod paint_pipeline;
pub mod preview;

pub use paint_pipeline::{PaintPipelineConfig, paint_projected_brush_into};
pub use preview::{
    ScreenPickBuffer, SurfaceHit, compute_mesh_fit, draw_mesh_wireframe, pick_surface_hit_from_buffer,
    render_preview_frame,
};
