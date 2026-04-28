use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Vertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
}

#[derive(Debug, Clone)]
pub struct MeshData {
    pub source_path: PathBuf,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

pub fn load_obj(path: &Path) -> Result<MeshData, String> {
    let content = fs::read_to_string(path)
        .map_err(|e| format!("Failed to read OBJ file {}: {e}", path.display()))?;
    parse_obj(path.to_path_buf(), &content)
}

fn parse_obj(source_path: PathBuf, content: &str) -> Result<MeshData, String> {
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut texcoords: Vec<[f32; 2]> = Vec::new();
    let mut vertices: Vec<Vertex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut vertex_map: HashMap<(usize, usize), u32> = HashMap::new();

    for (line_number, raw_line) in content.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let mut parts = line.split_whitespace();
        let Some(kind) = parts.next() else {
            continue;
        };

        match kind {
            "v" => {
                let x = parse_f32(parts.next(), line_number, "x")?;
                let y = parse_f32(parts.next(), line_number, "y")?;
                let z = parse_f32(parts.next(), line_number, "z")?;
                positions.push([x, y, z]);
            }
            "vt" => {
                let u = parse_f32(parts.next(), line_number, "u")?;
                let v = parse_f32(parts.next(), line_number, "v")?;
                texcoords.push([u, 1.0 - v]);
            }
            "f" => {
                let face_tokens: Vec<&str> = parts.collect();
                if face_tokens.len() < 3 {
                    return Err(format!("Line {}: face has fewer than 3 vertices", line_number + 1));
                }

                let mut face_indices: Vec<u32> = Vec::with_capacity(face_tokens.len());
                for token in face_tokens {
                    let (pos_idx, uv_idx) = parse_face_vertex(token, line_number)?;
                    let pos = positions.get(pos_idx).ok_or_else(|| {
                        format!("Line {}: position index out of bounds", line_number + 1)
                    })?;
                    let uv = texcoords.get(uv_idx).ok_or_else(|| {
                        format!("Line {}: UV index out of bounds", line_number + 1)
                    })?;

                    let key = (pos_idx, uv_idx);
                    let vertex_index = if let Some(existing) = vertex_map.get(&key) {
                        *existing
                    } else {
                        let idx = u32::try_from(vertices.len())
                            .map_err(|_| "Too many vertices for u32 index buffer".to_string())?;
                        vertices.push(Vertex {
                            position: *pos,
                            uv: *uv,
                        });
                        vertex_map.insert(key, idx);
                        idx
                    };
                    face_indices.push(vertex_index);
                }

                // Triangulate n-gons as a fan.
                for i in 1..(face_indices.len() - 1) {
                    indices.push(face_indices[0]);
                    indices.push(face_indices[i]);
                    indices.push(face_indices[i + 1]);
                }
            }
            _ => {}
        }
    }

    if vertices.is_empty() || indices.is_empty() {
        return Err("OBJ did not contain any renderable faces with UVs".to_string());
    }

    Ok(MeshData {
        source_path,
        vertices,
        indices,
    })
}

fn parse_f32(token: Option<&str>, line_number: usize, label: &str) -> Result<f32, String> {
    let token = token.ok_or_else(|| format!("Line {}: missing {label}", line_number + 1))?;
    token
        .parse::<f32>()
        .map_err(|_| format!("Line {}: invalid float for {label}", line_number + 1))
}

fn parse_face_vertex(token: &str, line_number: usize) -> Result<(usize, usize), String> {
    let mut split = token.split('/');
    let pos_raw = split.next().ok_or_else(|| {
        format!("Line {}: malformed face vertex '{}'", line_number + 1, token)
    })?;
    let uv_raw = split.next().ok_or_else(|| {
        format!("Line {}: face vertex '{}' is missing UV index", line_number + 1, token)
    })?;

    if uv_raw.is_empty() {
        return Err(format!(
            "Line {}: face vertex '{}' has empty UV index",
            line_number + 1,
            token
        ));
    }

    let pos_idx = pos_raw
        .parse::<usize>()
        .map_err(|_| format!("Line {}: invalid position index '{}'", line_number + 1, pos_raw))?;
    let uv_idx = uv_raw
        .parse::<usize>()
        .map_err(|_| format!("Line {}: invalid UV index '{}'", line_number + 1, uv_raw))?;

    if pos_idx == 0 || uv_idx == 0 {
        return Err(format!(
            "Line {}: OBJ indices are 1-based, found zero index",
            line_number + 1
        ));
    }

    Ok((pos_idx - 1, uv_idx - 1))
}

