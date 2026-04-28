use crate::io::mesh_loader::MeshData;
use crate::renderer::SurfaceHit;
use glam::{Vec2, Vec3, vec3};
use image::RgbaImage;
use serde::{Deserialize, Serialize};
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
    tile_size: usize,
    tiles_w: usize,
    tiles_h: usize,
    tri_bins: Vec<Vec<u32>>,
    tri_marks: Vec<u32>,
    tri_mark_epoch: u32,
    tri_scratch: Vec<u32>,
}

impl UvCoverageCache {
    fn ensure_for(&mut self, mesh: &MeshData, width: usize, height: usize) {
        let signature = mesh_signature(mesh);
        if self.width == width
            && self.height == height
            && self.mesh_signature == signature
            && self.covered.len() == width * height
            && !self.tri_bins.is_empty()
        {
            return;
        }
        self.covered = build_uv_coverage_map(mesh, width, height);
        let (tile_size, tiles_w, tiles_h, tri_bins) = build_uv_tile_bins(mesh, width, height);
        let tri_count = mesh.indices.len() / 3;
        self.tile_size = tile_size;
        self.tiles_w = tiles_w;
        self.tiles_h = tiles_h;
        self.tri_bins = tri_bins;
        self.tri_marks = vec![0; tri_count];
        self.tri_mark_epoch = 1;
        self.tri_scratch.clear();
        self.width = width;
        self.height = height;
        self.mesh_signature = signature;
    }

    fn coverage(&self) -> &[bool] {
        &self.covered
    }

    fn gather_candidate_triangles(&mut self, center_uv: [f32; 2], radius_texels: f32) -> &[u32] {
        self.tri_scratch.clear();
        if self.tri_bins.is_empty() || self.tile_size == 0 || self.tiles_w == 0 || self.tiles_h == 0 {
            return &self.tri_scratch;
        }

        if self.tri_mark_epoch == u32::MAX {
            self.tri_marks.fill(0);
            self.tri_mark_epoch = 1;
        } else {
            self.tri_mark_epoch += 1;
        }
        let mark = self.tri_mark_epoch;

        let cx = (center_uv[0] * (self.width.saturating_sub(1) as f32)).clamp(0.0, (self.width.saturating_sub(1)) as f32);
        let cy = (center_uv[1] * (self.height.saturating_sub(1) as f32)).clamp(0.0, (self.height.saturating_sub(1)) as f32);
        let r = radius_texels.max(1.0);
        let min_x = ((cx - r).floor().max(0.0) as usize) / self.tile_size;
        let max_x = ((cx + r).ceil().min((self.width.saturating_sub(1)) as f32) as usize) / self.tile_size;
        let min_y = ((cy - r).floor().max(0.0) as usize) / self.tile_size;
        let max_y = ((cy + r).ceil().min((self.height.saturating_sub(1)) as f32) as usize) / self.tile_size;
        let tx1 = max_x.min(self.tiles_w.saturating_sub(1));
        let ty1 = max_y.min(self.tiles_h.saturating_sub(1));

        for ty in min_y.min(ty1)..=ty1 {
            for tx in min_x.min(tx1)..=tx1 {
                let tile_idx = ty * self.tiles_w + tx;
                if let Some(bin) = self.tri_bins.get(tile_idx) {
                    for tri_id in bin {
                        let tid = *tri_id as usize;
                        if tid >= self.tri_marks.len() || self.tri_marks[tid] == mark {
                            continue;
                        }
                        self.tri_marks[tid] = mark;
                        self.tri_scratch.push(*tri_id);
                    }
                }
            }
        }
        &self.tri_scratch
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

#[derive(Debug, Clone, Copy)]
pub struct BrushInput {
    pub hit: SurfaceHit,
    pub center_world: Vec3,
    pub center_uv: [f32; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrushBlendMode {
    Normal,
    Multiply,
    Screen,
}

impl Default for BrushBlendMode {
    fn default() -> Self {
        Self::Normal
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrushFalloff {
    Smooth,
    Sphere,
    Root,
    Sharp,
    Linear,
    Constant,
}

impl Default for BrushFalloff {
    fn default() -> Self {
        Self::Smooth
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BrushDispatch {
    pub screen_pos: [f32; 2],
    pub radius_px: f32,
    pub strength: f32,
    pub color: [u8; 4],
    pub pressure: f32,
    pub blend_mode: BrushBlendMode,
    pub falloff: BrushFalloff,
}

impl BrushDispatch {
    pub fn resolved_color(self) -> [u8; 4] {
        let alpha = (self.color[3] as f32
            * self.strength.clamp(0.0, 1.0)
            * self.pressure.clamp(0.0, 1.0))
        .round() as u8;
        [self.color[0], self.color[1], self.color[2], alpha]
    }
}

pub fn paint_projected_brush_into(
    texture: &mut RgbaImage,
    mesh: &MeshData,
    input: BrushInput,
    dispatch: BrushDispatch,
    mut uv_coverage_cache: Option<&mut UvCoverageCache>,
    config: &PaintPipelineConfig,
) -> bool {
    let _ = input.hit.bary;
    let _ = dispatch.screen_pos;
    let _ = dispatch.blend_mode;
    let w = texture.width().max(1) as usize;
    let h = texture.height().max(1) as usize;
    let Some(world_radius) = hit_brush_radius(mesh, input.hit, w, h, dispatch.radius_px) else {
        return false;
    };
    let candidate_triangles = if let Some(cache) = uv_coverage_cache.as_deref_mut() {
        cache.ensure_for(mesh, w, h);
        Some(cache.gather_candidate_triangles(input.center_uv, dispatch.radius_px).to_vec())
    } else {
        None
    };
    let Some(mask) = build_projected_brush_mask(
        mesh,
        input.center_world,
        world_radius,
        dispatch.falloff,
        w,
        h,
        candidate_triangles.as_deref(),
    ) else {
        return false;
    };

    let brush_color = dispatch.resolved_color();
    let src_alpha = brush_color[3] as f32 / 255.0;
    let painted = apply_brush_mask(texture, &mask, brush_color, src_alpha, dispatch.blend_mode);
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

pub fn hit_brush_radius(
    mesh: &MeshData,
    hit: SurfaceHit,
    tex_w: usize,
    tex_h: usize,
    brush_radius_px: f32,
) -> Option<f32> {
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
    Some((brush_radius_px.max(1.0) * world_per_texel).max(1e-5))
}

pub fn build_projected_brush_mask(
    mesh: &MeshData,
    center_pos: Vec3,
    world_radius: f32,
    falloff_mode: BrushFalloff,
    tex_w: usize,
    tex_h: usize,
    tri_candidates: Option<&[u32]>,
) -> Option<BrushMask> {
    let mut mask = BrushMask::new(tex_w, tex_h);
    let r2 = world_radius * world_radius;

    let mut rasterize_triangle = |tri: [u32; 3]| {
        let (Some(v0), Some(v1), Some(v2)) = (
            mesh.vertices.get(tri[0] as usize),
            mesh.vertices.get(tri[1] as usize),
            mesh.vertices.get(tri[2] as usize),
        ) else {
            return;
        };
        let p0 = vec3(v0.position[0], v0.position[1], v0.position[2]);
        let p1 = vec3(v1.position[0], v1.position[1], v1.position[2]);
        let p2 = vec3(v2.position[0], v2.position[1], v2.position[2]);
        if point_triangle_distance_sq(center_pos, p0, p1, p2) > r2 {
            return;
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
            return;
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
            return;
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
                let normalized = (1.0 - dist / world_radius).clamp(0.0, 1.0);
                let falloff = apply_falloff_curve(normalized, falloff_mode);
                if falloff <= 0.0 {
                    continue;
                }

                let idx = y as usize * tex_w + x as usize;
                mask.set_max(idx, falloff);
            }
        }
    };

    if let Some(candidates) = tri_candidates {
        for tri_id in candidates {
            let i = *tri_id as usize * 3;
            if i + 2 >= mesh.indices.len() {
                continue;
            }
            rasterize_triangle([mesh.indices[i], mesh.indices[i + 1], mesh.indices[i + 2]]);
        }
    } else {
        for tri in mesh.indices.chunks_exact(3) {
            rasterize_triangle([tri[0], tri[1], tri[2]]);
        }
    }

    if mask.is_empty() { None } else { Some(mask) }
}

fn apply_falloff_curve(t: f32, mode: BrushFalloff) -> f32 {
    let t = t.clamp(0.0, 1.0);
    match mode {
        // Matches Blender-like smoothstep behavior.
        BrushFalloff::Smooth => t * t * (3.0 - 2.0 * t),
        BrushFalloff::Sphere => (1.0 - (1.0 - t) * (1.0 - t)).sqrt(),
        BrushFalloff::Root => t.sqrt(),
        BrushFalloff::Sharp => t * t,
        BrushFalloff::Linear => t,
        BrushFalloff::Constant => {
            if t > 0.0 {
                1.0
            } else {
                0.0
            }
        }
    }
}

pub fn apply_brush_mask(
    texture: &mut RgbaImage,
    mask: &BrushMask,
    src: [u8; 4],
    src_alpha: f32,
    blend_mode: BrushBlendMode,
) -> bool {
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
        let out_r = blend_channel(src[0], dst[0], alpha, blend_mode);
        let out_g = blend_channel(src[1], dst[1], alpha, blend_mode);
        let out_b = blend_channel(src[2], dst[2], alpha, blend_mode);
        *px = image::Rgba([out_r, out_g, out_b, 255]);
        painted_any = true;
    }
    painted_any
}

fn blend_channel(src: u8, dst: u8, alpha: f32, blend_mode: BrushBlendMode) -> u8 {
    let src_f = src as f32;
    let dst_f = dst as f32;
    let blended = match blend_mode {
        BrushBlendMode::Normal => src_f,
        BrushBlendMode::Multiply => (src_f * dst_f) / 255.0,
        BrushBlendMode::Screen => 255.0 - ((255.0 - src_f) * (255.0 - dst_f)) / 255.0,
    };
    let out = blended * alpha + dst_f * (1.0 - alpha);
    out.clamp(0.0, 255.0).round() as u8
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
        let mut next_frontier = Vec::new();
        let mut next_mark = vec![false; width * height];

        for idx in &frontier {
            if *idx >= width * height {
                continue;
            }
            let x = idx % width;
            let y = idx / width;
            let src_color = texture.get_pixel(x as u32, y as u32).0;
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

fn build_uv_tile_bins(
    mesh: &MeshData,
    width: usize,
    height: usize,
) -> (usize, usize, usize, Vec<Vec<u32>>) {
    let tile_size = 32_usize;
    let tiles_w = ((width + tile_size - 1) / tile_size).max(1);
    let tiles_h = ((height + tile_size - 1) / tile_size).max(1);
    let mut tri_bins = vec![Vec::<u32>::new(); tiles_w * tiles_h];

    for (tri_id, tri) in mesh.indices.chunks_exact(3).enumerate() {
        if tri_id > u32::MAX as usize {
            break;
        }
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
        let min_x = t0.x.min(t1.x).min(t2.x).floor().max(0.0) as usize;
        let max_x = t0
            .x
            .max(t1.x)
            .max(t2.x)
            .ceil()
            .min((width.saturating_sub(1)) as f32) as usize;
        let min_y = t0.y.min(t1.y).min(t2.y).floor().max(0.0) as usize;
        let max_y = t0
            .y
            .max(t1.y)
            .max(t2.y)
            .ceil()
            .min((height.saturating_sub(1)) as f32) as usize;
        if min_x > max_x || min_y > max_y {
            continue;
        }

        let tx0 = (min_x / tile_size).min(tiles_w - 1);
        let tx1 = (max_x / tile_size).min(tiles_w - 1);
        let ty0 = (min_y / tile_size).min(tiles_h - 1);
        let ty1 = (max_y / tile_size).min(tiles_h - 1);
        for ty in ty0..=ty1 {
            for tx in tx0..=tx1 {
                tri_bins[ty * tiles_w + tx].push(tri_id as u32);
            }
        }
    }

    (tile_size, tiles_w, tiles_h, tri_bins)
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
