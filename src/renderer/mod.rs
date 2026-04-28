pub mod paint_pipeline;
pub mod preview;

pub use paint_pipeline::{
    apply_brush_mask, apply_texture_padding, build_projected_brush_mask, hit_brush_center_and_radius,
};
pub use preview::{
    ScreenPickBuffer, SurfaceHit, compute_mesh_fit, draw_mesh_wireframe, pick_surface_hit_from_buffer,
    render_preview_frame,
};
