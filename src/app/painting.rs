use crate::PinturappUi;
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

    pub(crate) fn paint_at_uv(&mut self, uv: [f32; 2]) {
        self.ensure_albedo_texture();

        let Some(texture) = self.albedo_texture.as_mut() else {
            return;
        };
        let w = texture.width().max(1);
        let h = texture.height().max(1);
        let cx = uv[0].clamp(0.0, 1.0) * (w.saturating_sub(1) as f32);
        let cy = uv[1].clamp(0.0, 1.0) * (h.saturating_sub(1) as f32);
        let radius = self.brush_radius_px.max(1.0);
        let r2 = radius * radius;

        let min_x = (cx - radius).floor().max(0.0) as i32;
        let max_x = (cx + radius).ceil().min((w - 1) as f32) as i32;
        let min_y = (cy - radius).floor().max(0.0) as i32;
        let max_y = (cy + radius).ceil().min((h - 1) as f32) as i32;

        let src = self.brush_color.to_array();
        let src_alpha = src[3] as f32 / 255.0;
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let dx = x as f32 - cx;
                let dy = y as f32 - cy;
                if dx * dx + dy * dy > r2 {
                    continue;
                }

                let px = texture.get_pixel_mut(x as u32, y as u32);
                let dst = px.0;
                let out_r = src[0] as f32 * src_alpha + dst[0] as f32 * (1.0 - src_alpha);
                let out_g = src[1] as f32 * src_alpha + dst[1] as f32 * (1.0 - src_alpha);
                let out_b = src[2] as f32 * src_alpha + dst[2] as f32 * (1.0 - src_alpha);
                *px = image::Rgba([out_r as u8, out_g as u8, out_b as u8, 255]);
            }
        }
        self.is_dirty = true;
    }
}
