use crate::PinturappUi;
use crate::io::mesh_loader::MeshData;
use crate::renderer::{SurfaceHit, paint_projected_brush_into};
use image::RgbaImage;

impl PinturappUi {
    pub(crate) fn clear_history(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
    }

    fn ensure_albedo_texture(&mut self) {
        if self.albedo_texture.is_none() {
            self.albedo_texture = Some(RgbaImage::from_pixel(1024, 1024, image::Rgba([200, 200, 200, 255])));
        }
    }

    pub(crate) fn begin_paint_stroke(&mut self) {
        self.ensure_albedo_texture();
        if let Some(texture) = &self.albedo_texture {
            self.undo_stack.push_back(texture.clone());
            if self.undo_stack.len() > Self::MAX_HISTORY {
                self.undo_stack.pop_front();
            }
            self.redo_stack.clear();
            self.is_dirty = true;
        }
    }

    pub(crate) fn undo_paint(&mut self) {
        let Some(current) = self.albedo_texture.take() else {
            return;
        };
        let Some(previous) = self.undo_stack.pop_back() else {
            self.albedo_texture = Some(current);
            return;
        };
        self.redo_stack.push_back(current);
        if self.redo_stack.len() > Self::MAX_HISTORY {
            self.redo_stack.pop_front();
        }
        self.albedo_texture = Some(previous);
        self.is_dirty = true;
    }

    pub(crate) fn redo_paint(&mut self) {
        let Some(current) = self.albedo_texture.take() else {
            return;
        };
        let Some(next) = self.redo_stack.pop_back() else {
            self.albedo_texture = Some(current);
            return;
        };
        self.undo_stack.push_back(current);
        if self.undo_stack.len() > Self::MAX_HISTORY {
            self.undo_stack.pop_front();
        }
        self.albedo_texture = Some(next);
        self.is_dirty = true;
    }

    pub(crate) fn paint_projected_brush(&mut self, mesh: &MeshData, hit: SurfaceHit) {
        self.ensure_albedo_texture();
        let Some(texture) = self.albedo_texture.as_mut() else {
            return;
        };
        if paint_projected_brush_into(
            texture,
            mesh,
            hit,
            self.brush_radius_px,
            self.brush_color.to_array(),
            &self.paint_pipeline_config,
        ) {
            self.is_dirty = true;
        }
    }
}
