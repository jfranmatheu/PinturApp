use crate::io::mesh_loader::MeshData;
use crate::renderer::SurfaceHit;
use glam::{Vec2, Vec3, vec3};
use image::RgbaImage;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

#[derive(Debug, Clone)]
pub struct PaintPipelineConfig {
    pub padding_iterations: usize,
}

impl Default for PaintPipelineConfig {
    fn default() -> Self {
        Self { padding_iterations: 2 }
    }
}

#[derive(Debug, Clone, Default)]
pub struct UvCoverageCache {
    width: usize,
    height: usize,
    mesh_signature: u64,
    covered: Vec<bool>,
}

impl UvCoverageCache {
    fn ensure_for(&mut self, mesh: &MeshData, width: usize, height: usize) {
        let signature = mesh_signature(mesh);
        if self.width == width
            && self.height == height
            && self.mesh_signature == signature
            && self.covered.len() == width * height
        {
            return;
        }
        self.covered = build_uv_coverage_map(mesh, width, height);
        self.width = width;
        self.height = height;
        self.mesh_signature = signature;
    }

    fn coverage(&self) -> &[bool] {
        &self.covered
    }
}

pub struct BrushMask {
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

    pub fn is_empty(&self) -> bool {
        self.touched.is_empty()
    }
}

pub fn paint_projected_brush_into(
    texture: &mut RgbaImage,
    mesh: &MeshData,
    hit: SurfaceHit,
    brush_radius_px: f32,
    brush_color: [u8; 4],
    uv_coverage_cache: Option<&mut UvCoverageCache>,
    config: &PaintPipelineConfig,
) -> bool {
    let w = texture.width().max(1) as usize;
    let h = texture.height().max(1) as usize;
    let Some((center_pos, world_radius)) = hit_brush_center_and_radius(mesh, hit, w, h, brush_radius_px) else {
        return false;
    };
    let Some(mask) = build_projected_brush_mask(mesh, center_pos, world_radius, w, h) else {
        return false;
    };

    let src_alpha = brush_color[3] as f32 / 255.0;
    let painted = apply_brush_mask(texture, &mask, brush_color, src_alpha);
    if painted {
        if let Some(cache) = uv_coverage_cache {
            cache.ensure_for(mesh, w, h);
            apply_texture_padding(texture, &mask, cache.coverage(), config.padding_iterations);
        } else {
            let coverage = build_uv_coverage_map(mesh, w, h);
            apply_texture_padding(texture, &mask, &coverage, config.padding_iterations);
        }
    }
    painted
}

pub fn hit_brush_center_and_radius(
    mesh: &MeshData,
    hit: SurfaceHit,
    tex_w: usize,
    tex_h: usize,
    brush_radius_px: f32,
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

    let uv0 = Vec2::new(
        v0.uv[0] * (tex_w.saturating_sub(1) as f32),
        v0.uv[1] * (tex_h.saturating_sub(1) as f32),
    );
    let uv1 = Vec2::new(
        v1.uv[0] * (tex_w.saturating_sub(1) as f32),
        v1.uv[1] * (tex_h.saturating_sub(1) as f32),
    );
    let uv2 = Vec2::new(
        v2.uv[0] * (tex_w.saturating_sub(1) as f32),
        v2.uv[1] * (tex_h.saturating_sub(1) as f32),
    );
    let area_obj = 0.5 * (p1 - p0).cross(p2 - p0).length();
    let area_tex = 0.5 * (uv1 - uv0).perp_dot(uv2 - uv0).abs();
    if area_obj <= 1e-8 || area_tex <= 1e-8 {
        return None;
    }
    let world_per_texel = (area_obj / area_tex).sqrt();
    let world_radius = (brush_radius_px.max(1.0) * world_per_texel).max(1e-5);
    Some((center_pos, world_radius))
}

pub fn build_projected_brush_mask(
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
        let max_x = t0
            .x
            .max(t1.x)
            .max(t2.x)
            .ceil()
            .min((tex_w.saturating_sub(1)) as f32) as i32;
        let min_y = t0.y.min(t1.y).min(t2.y).floor().max(0.0) as i32;
        let max_y = t0
            .y
            .max(t1.y)
            .max(t2.y)
            .ceil()
            .min((tex_h.saturating_sub(1)) as f32) as i32;
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

    if mask.is_empty() { None } else { Some(mask) }
}

pub fn apply_brush_mask(texture: &mut RgbaImage, mask: &BrushMask, src: [u8; 4], src_alpha: f32) -> bool {
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

pub fn apply_texture_padding(texture: &mut RgbaImage, mask: &BrushMask, coverage: &[bool], iterations: usize) {
    if iterations == 0 || mask.is_empty() {
        return;
    }

    let width = texture.width().max(1) as usize;
    let height = texture.height().max(1) as usize;
    if coverage.len() != width * height {
        return;
    }

    let mut frontier = mask.touched.clone();
    let mut seeded = vec![false; width * height];
    for idx in &frontier {
        if *idx < seeded.len() {
            seeded[*idx] = true;
        }
    }

    for _ in 0..iterations {
        if frontier.is_empty() {
            break;
        }
        let snapshot = texture.clone();
        let mut next_frontier = Vec::new();
        let mut next_mark = vec![false; width * height];

        for idx in &frontier {
            if *idx >= width * height {
                continue;
            }
            let x = idx % width;
            let y = idx / width;
            let src_color = snapshot.get_pixel(x as u32, y as u32).0;
            let neighbors = [
                (x as i32 - 1, y as i32),
                (x as i32 + 1, y as i32),
                (x as i32, y as i32 - 1),
                (x as i32, y as i32 + 1),
            ];

            for (nx, ny) in neighbors {
                if nx < 0 || ny < 0 || nx >= width as i32 || ny >= height as i32 {
                    continue;
                }
                let nidx = ny as usize * width + nx as usize;
                if coverage[nidx] || seeded[nidx] {
                    continue;
                }

                *texture.get_pixel_mut(nx as u32, ny as u32) = image::Rgba(src_color);
                seeded[nidx] = true;
                if !next_mark[nidx] {
                    next_mark[nidx] = true;
                    next_frontier.push(nidx);
                }
            }
        }

        frontier = next_frontier;
    }
}

fn build_uv_coverage_map(mesh: &MeshData, width: usize, height: usize) -> Vec<bool> {
    let mut covered = vec![false; width * height];
    for tri in mesh.indices.chunks_exact(3) {
        let (Some(v0), Some(v1), Some(v2)) = (
            mesh.vertices.get(tri[0] as usize),
            mesh.vertices.get(tri[1] as usize),
            mesh.vertices.get(tri[2] as usize),
        ) else {
            continue;
        };
        let t0 = Vec2::new(
            v0.uv[0] * (width.saturating_sub(1) as f32),
            v0.uv[1] * (height.saturating_sub(1) as f32),
        );
        let t1 = Vec2::new(
            v1.uv[0] * (width.saturating_sub(1) as f32),
            v1.uv[1] * (height.saturating_sub(1) as f32),
        );
        let t2 = Vec2::new(
            v2.uv[0] * (width.saturating_sub(1) as f32),
            v2.uv[1] * (height.saturating_sub(1) as f32),
        );
        let area = edge_fn_2d(t0, t1, t2);
        if area.abs() < 1e-6 {
            continue;
        }

        let min_x = t0.x.min(t1.x).min(t2.x).floor().max(0.0) as i32;
        let max_x = t0
            .x
            .max(t1.x)
            .max(t2.x)
            .ceil()
            .min((width.saturating_sub(1)) as f32) as i32;
        let min_y = t0.y.min(t1.y).min(t2.y).floor().max(0.0) as i32;
        let max_y = t0
            .y
            .max(t1.y)
            .max(t2.y)
            .ceil()
            .min((height.saturating_sub(1)) as f32) as i32;
        if min_x > max_x || min_y > max_y {
            continue;
        }

        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                let w0 = edge_fn_2d(t1, t2, p) / area;
                let w1 = edge_fn_2d(t2, t0, p) / area;
                let w2 = edge_fn_2d(t0, t1, p) / area;
                if w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0 {
                    let idx = y as usize * width + x as usize;
                    covered[idx] = true;
                }
            }
        }
    }
    covered
}

fn edge_fn_2d(a: Vec2, b: Vec2, p: Vec2) -> f32 {
    (p.x - a.x) * (b.y - a.y) - (p.y - a.y) * (b.x - a.x)
}

fn mesh_signature(mesh: &MeshData) -> u64 {
    let mut hasher = DefaultHasher::new();
    mesh.source_path.hash(&mut hasher);
    mesh.vertices.len().hash(&mut hasher);
    mesh.indices.len().hash(&mut hasher);
    if let Some(v) = mesh.vertices.first() {
        v.uv[0].to_bits().hash(&mut hasher);
        v.uv[1].to_bits().hash(&mut hasher);
        v.position[0].to_bits().hash(&mut hasher);
    }
    if let Some(v) = mesh.vertices.last() {
        v.uv[0].to_bits().hash(&mut hasher);
        v.uv[1].to_bits().hash(&mut hasher);
        v.position[2].to_bits().hash(&mut hasher);
    }
    if let Some(i) = mesh.indices.first() {
        i.hash(&mut hasher);
    }
    if let Some(i) = mesh.indices.last() {
        i.hash(&mut hasher);
    }
    hasher.finish()
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
