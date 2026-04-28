use crate::io::mesh_loader::MeshData;
use eframe::egui::{self, ColorImage};
use glam::{Mat4, Vec3, Vec4, vec3};
use image::RgbaImage;
use std::collections::HashSet;

pub fn compute_mesh_fit(mesh: &MeshData) -> (Vec3, f32) {
    let mut min = vec3(f32::MAX, f32::MAX, f32::MAX);
    let mut max = vec3(f32::MIN, f32::MIN, f32::MIN);

    for v in &mesh.vertices {
        let p = vec3(v.position[0], v.position[1], v.position[2]);
        min = min.min(p);
        max = max.max(p);
    }

    let center = (min + max) * 0.5;
    let extent = (max - min).max(vec3(1e-4, 1e-4, 1e-4));
    let largest = extent.x.max(extent.y).max(extent.z);
    let fit_scale = 1.8 / largest;
    (center, fit_scale)
}

pub fn render_textured_preview(
    mesh: &MeshData,
    center: Vec3,
    fit_scale: f32,
    yaw: f32,
    pitch: f32,
    distance: f32,
    size: [usize; 2],
    albedo: Option<&RgbaImage>,
) -> ColorImage {
    let width = size[0].max(1);
    let height = size[1].max(1);
    let mut pixels = vec![0_u8; width * height * 4];
    let mut depth = vec![f32::INFINITY; width * height];

    for i in 0..(width * height) {
        let p = i * 4;
        pixels[p] = 26;
        pixels[p + 1] = 29;
        pixels[p + 2] = 35;
        pixels[p + 3] = 255;
    }

    let target = vec3(0.0, 0.0, 0.0);
    let eye = target
        + vec3(
            distance * yaw.cos() * pitch.cos(),
            distance * pitch.sin(),
            distance * yaw.sin() * pitch.cos(),
        );

    let model = Mat4::from_scale(Vec3::splat(fit_scale)) * Mat4::from_translation(-center);
    let view = Mat4::look_at_rh(eye, target, Vec3::Y);
    let aspect = (width as f32 / height as f32).max(0.01);
    let proj = Mat4::perspective_rh_gl(45.0_f32.to_radians(), aspect, 0.01, 200.0);
    let mvp = proj * view * model;

    for tri in mesh.indices.chunks_exact(3) {
        let i0 = tri[0] as usize;
        let i1 = tri[1] as usize;
        let i2 = tri[2] as usize;
        let (Some(v0), Some(v1), Some(v2)) = (
            mesh.vertices.get(i0),
            mesh.vertices.get(i1),
            mesh.vertices.get(i2),
        ) else {
            continue;
        };

        let Some(p0) = project_vertex(mvp, v0.position, width, height) else {
            continue;
        };
        let Some(p1) = project_vertex(mvp, v1.position, width, height) else {
            continue;
        };
        let Some(p2) = project_vertex(mvp, v2.position, width, height) else {
            continue;
        };

        rasterize_triangle(
            &mut pixels,
            &mut depth,
            [width, height],
            [(p0.0, p0.1, p0.2, v0.uv), (p1.0, p1.1, p1.2, v1.uv), (p2.0, p2.1, p2.2, v2.uv)],
            albedo,
        );
    }

    ColorImage::from_rgba_unmultiplied([width, height], &pixels)
}

pub fn pick_paint_uv_targets_at_screen(
    mesh: &MeshData,
    center: Vec3,
    fit_scale: f32,
    yaw: f32,
    pitch: f32,
    distance: f32,
    size: [usize; 2],
    screen: [f32; 2],
    sample_radius_px: f32,
) -> Vec<[f32; 2]> {
    let mut targets = Vec::new();
    let mut seen: HashSet<(i32, i32)> = HashSet::new();

    let radius = sample_radius_px.max(1.0);
    let step = (radius / 3.0).max(1.0);
    let radius_sq = radius * radius;
    let mut oy = -radius;
    while oy <= radius + 0.001 {
        let mut ox = -radius;
        while ox <= radius + 0.001 {
            if ox * ox + oy * oy <= radius_sq + 0.25 {
                if let Some(uv) = pick_uv_at_screen(
                    mesh,
                    center,
                    fit_scale,
                    yaw,
                    pitch,
                    distance,
                    size,
                    [screen[0] + ox, screen[1] + oy],
                ) {
                    push_unique_uv(&mut targets, &mut seen, uv);
                }
            }
            ox += step;
        }
        oy += step;
    }

    if targets.is_empty() {
        if let Some(uv) = pick_uv_at_screen(mesh, center, fit_scale, yaw, pitch, distance, size, screen) {
            push_unique_uv(&mut targets, &mut seen, uv);
        }
    }

    targets
}

fn pick_uv_at_screen(
    mesh: &MeshData,
    center: Vec3,
    fit_scale: f32,
    yaw: f32,
    pitch: f32,
    distance: f32,
    size: [usize; 2],
    screen: [f32; 2],
) -> Option<[f32; 2]> {
    let width = size[0].max(1);
    let height = size[1].max(1);

    let target = vec3(0.0, 0.0, 0.0);
    let eye = target
        + vec3(
            distance * yaw.cos() * pitch.cos(),
            distance * pitch.sin(),
            distance * yaw.sin() * pitch.cos(),
        );
    let model = Mat4::from_scale(Vec3::splat(fit_scale)) * Mat4::from_translation(-center);
    let view = Mat4::look_at_rh(eye, target, Vec3::Y);
    let aspect = (width as f32 / height as f32).max(0.01);
    let proj = Mat4::perspective_rh_gl(45.0_f32.to_radians(), aspect, 0.01, 200.0);
    let mvp = proj * view * model;

    let mut best_z = f32::INFINITY;
    let mut best_uv: Option<[f32; 2]> = None;

    for tri in mesh.indices.chunks_exact(3) {
        let (Some(v0), Some(v1), Some(v2)) = (
            mesh.vertices.get(tri[0] as usize),
            mesh.vertices.get(tri[1] as usize),
            mesh.vertices.get(tri[2] as usize),
        ) else {
            continue;
        };
        let Some(p0) = project_vertex(mvp, v0.position, width, height) else {
            continue;
        };
        let Some(p1) = project_vertex(mvp, v1.position, width, height) else {
            continue;
        };
        let Some(p2) = project_vertex(mvp, v2.position, width, height) else {
            continue;
        };

        let area = edge_fn(p0.0, p0.1, p1.0, p1.1, p2.0, p2.1);
        if area.abs() < 1e-6 {
            continue;
        }

        let px = screen[0] + 0.5;
        let py = screen[1] + 0.5;
        let w0 = edge_fn(p1.0, p1.1, p2.0, p2.1, px, py) / area;
        let w1 = edge_fn(p2.0, p2.1, p0.0, p0.1, px, py) / area;
        let w2 = edge_fn(p0.0, p0.1, p1.0, p1.1, px, py) / area;
        if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
            continue;
        }

        let z = w0 * p0.2 + w1 * p1.2 + w2 * p2.2;
        if z < best_z {
            best_z = z;
            let u = w0 * v0.uv[0] + w1 * v1.uv[0] + w2 * v2.uv[0];
            let v = w0 * v0.uv[1] + w1 * v1.uv[1] + w2 * v2.uv[1];
            best_uv = Some([u, v]);
        }
    }

    best_uv
}

fn push_unique_uv(targets: &mut Vec<[f32; 2]>, seen: &mut HashSet<(i32, i32)>, uv: [f32; 2]) {
    let key = (
        (uv[0] * 1_000_000.0).round() as i32,
        (uv[1] * 1_000_000.0).round() as i32,
    );
    if seen.insert(key) {
        targets.push(uv);
    }
}

pub fn draw_mesh_wireframe(
    painter: &egui::Painter,
    rect: egui::Rect,
    mesh: &MeshData,
    center: Vec3,
    fit_scale: f32,
    yaw: f32,
    pitch: f32,
    distance: f32,
) {
    let target = vec3(0.0, 0.0, 0.0);
    let eye = target
        + vec3(
            distance * yaw.cos() * pitch.cos(),
            distance * pitch.sin(),
            distance * yaw.sin() * pitch.cos(),
        );

    let model = Mat4::from_scale(Vec3::splat(fit_scale)) * Mat4::from_translation(-center);
    let view = Mat4::look_at_rh(eye, target, Vec3::Y);
    let aspect = (rect.width() / rect.height()).max(0.01);
    let proj = Mat4::perspective_rh_gl(45.0_f32.to_radians(), aspect, 0.01, 200.0);
    let mvp = proj * view * model;

    let mut unique_edges: HashSet<(u32, u32)> = HashSet::new();

    for tri in mesh.indices.chunks_exact(3) {
        let a = tri[0];
        let b = tri[1];
        let c = tri[2];
        add_edge(&mut unique_edges, a, b);
        add_edge(&mut unique_edges, b, c);
        add_edge(&mut unique_edges, c, a);
    }

    let stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(150, 200, 255));
    for (a, b) in unique_edges {
        let p0 = mesh.vertices.get(a as usize).map(|v| v.position);
        let p1 = mesh.vertices.get(b as usize).map(|v| v.position);
        let (Some(p0), Some(p1)) = (p0, p1) else {
            continue;
        };

        let Some(s0) = project_to_screen(rect, mvp, p0) else {
            continue;
        };
        let Some(s1) = project_to_screen(rect, mvp, p1) else {
            continue;
        };

        painter.line_segment([s0, s1], stroke);
    }
}

fn project_vertex(mvp: Mat4, pos: [f32; 3], width: usize, height: usize) -> Option<(f32, f32, f32)> {
    let clip = mvp * Vec4::new(pos[0], pos[1], pos[2], 1.0);
    if clip.w <= 0.0 {
        return None;
    }

    let ndc = clip.truncate() / clip.w;
    if ndc.x.abs() > 1.3 || ndc.y.abs() > 1.3 || ndc.z.abs() > 1.3 {
        return None;
    }

    let sx = (ndc.x * 0.5 + 0.5) * width as f32;
    let sy = (-ndc.y * 0.5 + 0.5) * height as f32;
    Some((sx, sy, ndc.z))
}

fn rasterize_triangle(
    pixels: &mut [u8],
    depth: &mut [f32],
    size: [usize; 2],
    tri: [(f32, f32, f32, [f32; 2]); 3],
    albedo: Option<&RgbaImage>,
) {
    let width = size[0];
    let height = size[1];
    let (x0, y0, z0, uv0) = tri[0];
    let (x1, y1, z1, uv1) = tri[1];
    let (x2, y2, z2, uv2) = tri[2];

    let area = edge_fn(x0, y0, x1, y1, x2, y2);
    if area.abs() < 1e-6 {
        return;
    }

    let min_x = x0.min(x1).min(x2).floor().max(0.0) as i32;
    let max_x = x0.max(x1).max(x2).ceil().min((width - 1) as f32) as i32;
    let min_y = y0.min(y1).min(y2).floor().max(0.0) as i32;
    let max_y = y0.max(y1).max(y2).ceil().min((height - 1) as f32) as i32;

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;

            let w0 = edge_fn(x1, y1, x2, y2, px, py) / area;
            let w1 = edge_fn(x2, y2, x0, y0, px, py) / area;
            let w2 = edge_fn(x0, y0, x1, y1, px, py) / area;
            if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                continue;
            }

            let z = w0 * z0 + w1 * z1 + w2 * z2;
            let idx = y as usize * width + x as usize;
            if z >= depth[idx] {
                continue;
            }
            depth[idx] = z;

            let u = w0 * uv0[0] + w1 * uv1[0] + w2 * uv2[0];
            let v = w0 * uv0[1] + w1 * uv1[1] + w2 * uv2[1];
            let mut color = if let Some(tex) = albedo {
                sample_texture(tex, u, v)
            } else {
                let ru = (u.fract().abs() * 255.0) as u8;
                let gv = (v.fract().abs() * 255.0) as u8;
                [ru, gv, 220, 255]
            };

            let shade = (1.2 - z * 0.35).clamp(0.35, 1.15);
            color[0] = ((color[0] as f32) * shade).clamp(0.0, 255.0) as u8;
            color[1] = ((color[1] as f32) * shade).clamp(0.0, 255.0) as u8;
            color[2] = ((color[2] as f32) * shade).clamp(0.0, 255.0) as u8;

            let p = idx * 4;
            pixels[p] = color[0];
            pixels[p + 1] = color[1];
            pixels[p + 2] = color[2];
            pixels[p + 3] = 255;
        }
    }
}

fn sample_texture(tex: &RgbaImage, u: f32, v: f32) -> [u8; 4] {
    let w = tex.width().max(1);
    let h = tex.height().max(1);
    let uu = u.rem_euclid(1.0);
    let vv = v.rem_euclid(1.0);
    let x = (uu * (w.saturating_sub(1) as f32)).round() as u32;
    let y = (vv * (h.saturating_sub(1) as f32)).round() as u32;
    tex.get_pixel(x, y).0
}

fn edge_fn(ax: f32, ay: f32, bx: f32, by: f32, px: f32, py: f32) -> f32 {
    (px - ax) * (by - ay) - (py - ay) * (bx - ax)
}

fn add_edge(edges: &mut HashSet<(u32, u32)>, a: u32, b: u32) {
    if a < b {
        edges.insert((a, b));
    } else {
        edges.insert((b, a));
    }
}

fn project_to_screen(rect: egui::Rect, mvp: Mat4, pos: [f32; 3]) -> Option<egui::Pos2> {
    let clip: Vec4 = mvp * Vec4::new(pos[0], pos[1], pos[2], 1.0);
    if clip.w <= 0.0 {
        return None;
    }

    let ndc = clip.truncate() / clip.w;
    if ndc.x.abs() > 1.5 || ndc.y.abs() > 1.5 || ndc.z.abs() > 1.5 {
        return None;
    }

    let sx = rect.left() + (ndc.x * 0.5 + 0.5) * rect.width();
    let sy = rect.top() + (-ndc.y * 0.5 + 0.5) * rect.height();
    Some(egui::pos2(sx, sy))
}
