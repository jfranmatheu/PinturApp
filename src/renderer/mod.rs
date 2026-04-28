pub mod gpu_paint;
pub mod paint_pipeline;
pub mod preview;

pub use paint_pipeline::{
    BrushBlendMode, BrushDispatch, BrushFalloff, BrushInput, PaintPipelineConfig, UvCoverageCache,
    paint_projected_brush_into,
};
pub use preview::{
    ScreenPickBuffer, SurfaceHit, compute_mesh_fit, draw_mesh_wireframe, render_preview_frame,
    sample_surface_from_buffer,
};
