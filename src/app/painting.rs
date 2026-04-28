use crate::PinturappUi;
use crate::io::mesh_loader::MeshData;
use crate::renderer::SurfaceHit;
use glam::{Vec2, Vec3, vec3};
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
        let Some(texture_ref) = self.albedo_texture.as_ref() else {
            return;
        };
        let w = texture_ref.width().max(1) as usize;
        let h = texture_ref.height().max(1) as usize;
        let Some((center_pos, world_radius)) = self.hit_brush_center_and_radius(mesh, hit, w, h) else {
            return;
        };
        let Some(mask) = self.build_projected_brush_mask(mesh, center_pos, world_radius, w, h) else {
            return;
        };
        if mask.touched.is_empty() {
            return;
        }

        let Some(texture) = self.albedo_texture.as_mut() else {
            return;
        };
        let src = self.brush_color.to_array();
        let src_alpha = src[3] as f32 / 255.0;
        if apply_brush_mask(texture, &mask, src, src_alpha) {
            self.is_dirty = true;
        }
    }

    fn build_projected_brush_mask(
        &self,
        mesh: &MeshData,
        center_pos: Vec3,
        world_radius: f32,
        tex_w: usize,
        tex_h: usize,
    ) -> Option<BrushMask> {
        let mut mask = BrushMask::new(tex_w, tex_h);
        let r2 = world_radius * world_radius;

        for tri in mesh.indices.chunks_exact(3) {
            let (Some(v0), Some(v1), Some(v2)) = (
                mesh.vertices.get(tri[0] as usize),
                mesh.vertices.get(tri[1] as usize),
                mesh.vertices.get(tri[2] as usize),
            ) else {
                continue;
            };
            let p0 = vec3(v0.position[0], v0.position[1], v0.position[2]);
            let p1 = vec3(v1.position[0], v1.position[1], v1.position[2]);
            let p2 = vec3(v2.position[0], v2.position[1], v2.position[2]);
            if point_triangle_distance_sq(center_pos, p0, p1, p2) > r2 {
                continue;
            }

            let t0 = Vec2::new(
                v0.uv[0] * (tex_w.saturating_sub(1) as f32),
                v0.uv[1] * (tex_h.saturating_sub(1) as f32),
            );
            let t1 = Vec2::new(
                v1.uv[0] * (tex_w.saturating_sub(1) as f32),
                v1.uv[1] * (tex_h.saturating_sub(1) as f32),
            );
            let t2 = Vec2::new(
                v2.uv[0] * (tex_w.saturating_sub(1) as f32),
                v2.uv[1] * (tex_h.saturating_sub(1) as f32),
            );
            let area = edge_fn_2d(t0, t1, t2);
            if area.abs() < 1e-6 {
                continue;
            }

            let min_x = t0.x.min(t1.x).min(t2.x).floor().max(0.0) as i32;
            let max_x = t0.x.max(t1.x).max(t2.x).ceil().min((tex_w.saturating_sub(1)) as f32) as i32;
            let min_y = t0.y.min(t1.y).min(t2.y).floor().max(0.0) as i32;
            let max_y = t0.y.max(t1.y).max(t2.y).ceil().min((tex_h.saturating_sub(1)) as f32) as i32;
            if min_x > max_x || min_y > max_y {
                continue;
            }

            for y in min_y..=max_y {
                for x in min_x..=max_x {
                    let uvp = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                    let w0 = edge_fn_2d(t1, t2, uvp) / area;
                    let w1 = edge_fn_2d(t2, t0, uvp) / area;
                    let w2 = edge_fn_2d(t0, t1, uvp) / area;
                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                        continue;
                    }

                    let p = p0 * w0 + p1 * w1 + p2 * w2;
                    let dist_sq = (p - center_pos).length_squared();
                    if dist_sq > r2 {
                        continue;
                    }

                    let dist = dist_sq.sqrt();
                    let falloff = (1.0 - dist / world_radius).clamp(0.0, 1.0);
                    if falloff <= 0.0 {
                        continue;
                    }

                    let idx = y as usize * tex_w + x as usize;
                    mask.set_max(idx, falloff);
                }
            }
        }

        if mask.touched.is_empty() {
            None
        } else {
            Some(mask)
        }
    }

    fn hit_brush_center_and_radius(
        &self,
        mesh: &MeshData,
        hit: SurfaceHit,
        tex_w: usize,
        tex_h: usize,
    ) -> Option<(Vec3, f32)> {
        let (Some(v0), Some(v1), Some(v2)) = (
            mesh.vertices.get(hit.tri[0] as usize),
            mesh.vertices.get(hit.tri[1] as usize),
            mesh.vertices.get(hit.tri[2] as usize),
        ) else {
            return None;
        };
        let p0 = vec3(v0.position[0], v0.position[1], v0.position[2]);
        let p1 = vec3(v1.position[0], v1.position[1], v1.position[2]);
        let p2 = vec3(v2.position[0], v2.position[1], v2.position[2]);
        let center_pos = p0 * hit.bary[0] + p1 * hit.bary[1] + p2 * hit.bary[2];

        let uv0 = Vec2::new(v0.uv[0] * (tex_w.saturating_sub(1) as f32), v0.uv[1] * (tex_h.saturating_sub(1) as f32));
        let uv1 = Vec2::new(v1.uv[0] * (tex_w.saturating_sub(1) as f32), v1.uv[1] * (tex_h.saturating_sub(1) as f32));
        let uv2 = Vec2::new(v2.uv[0] * (tex_w.saturating_sub(1) as f32), v2.uv[1] * (tex_h.saturating_sub(1) as f32));
        let area_obj = 0.5 * (p1 - p0).cross(p2 - p0).length();
        let area_tex = 0.5 * (uv1 - uv0).perp_dot(uv2 - uv0).abs();
        if area_obj <= 1e-8 || area_tex <= 1e-8 {
            return None;
        }
        let world_per_texel = (area_obj / area_tex).sqrt();
        let world_radius = (self.brush_radius_px.max(1.0) * world_per_texel).max(1e-5);
        Some((center_pos, world_radius))
    }
}

fn edge_fn_2d(a: Vec2, b: Vec2, p: Vec2) -> f32 {
    (p.x - a.x) * (b.y - a.y) - (p.y - a.y) * (b.x - a.x)
}

struct BrushMask {
    weights: Vec<f32>,
    touched: Vec<usize>,
}

impl BrushMask {
    fn new(width: usize, height: usize) -> Self {
        Self {
            weights: vec![0.0; width * height],
            touched: Vec::new(),
        }
    }

    fn set_max(&mut self, idx: usize, value: f32) {
        if value <= 0.0 || idx >= self.weights.len() {
            return;
        }
        if self.weights[idx] <= 0.0 {
            self.touched.push(idx);
        }
        self.weights[idx] = self.weights[idx].max(value);
    }
}

fn apply_brush_mask(texture: &mut RgbaImage, mask: &BrushMask, src: [u8; 4], src_alpha: f32) -> bool {
    let width = texture.width().max(1) as usize;
    let mut painted_any = false;
    for idx in &mask.touched {
        let falloff = mask.weights[*idx];
        if falloff <= 0.0 {
            continue;
        }
        let alpha = src_alpha * falloff;
        if alpha <= 0.0 {
            continue;
        }

        let x = (*idx % width) as u32;
        let y = (*idx / width) as u32;
        let px = texture.get_pixel_mut(x, y);
        let dst = px.0;
        let out_r = src[0] as f32 * alpha + dst[0] as f32 * (1.0 - alpha);
        let out_g = src[1] as f32 * alpha + dst[1] as f32 * (1.0 - alpha);
        let out_b = src[2] as f32 * alpha + dst[2] as f32 * (1.0 - alpha);
        *px = image::Rgba([out_r as u8, out_g as u8, out_b as u8, 255]);
        painted_any = true;
    }
    painted_any
}

fn point_triangle_distance_sq(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> f32 {
    let ab = b - a;
    let ac = c - a;
    let ap = p - a;
    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return ap.length_squared();
    }

    let bp = p - b;
    let d3 = ab.dot(bp);
    let d4 = ac.dot(bp);
    if d3 >= 0.0 && d4 <= d3 {
        return bp.length_squared();
    }

    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        let proj = a + ab * v;
        return (p - proj).length_squared();
    }

    let cp = p - c;
    let d5 = ab.dot(cp);
    let d6 = ac.dot(cp);
    if d6 >= 0.0 && d5 <= d6 {
        return cp.length_squared();
    }

    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        let proj = a + ac * w;
        return (p - proj).length_squared();
    }

    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let u = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        let proj = b + (c - b) * u;
        return (p - proj).length_squared();
    }

    let n = ab.cross(ac).normalize_or_zero();
    let dist = (p - a).dot(n);
    dist * dist
}
