use std::{error::Error, fmt, path::Path};

use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub base_color: [f32; 4],
    pub joints: [u32; 4],
    pub weights: [f32; 4],
}

#[derive(Debug)]
pub struct MeshData {
    pub vertices: Vec<Vertex>,
    pub draw_ranges: Vec<DrawRange>,
    pub textures: Vec<crate::texture::TextureImage>,
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
    pub source_meshes: usize,
    pub triangles: usize,
    pub materials: Vec<crate::fbx_materials::MaterialBinding>,
    pub hierarchy: crate::fbx_hierarchy::Hierarchy,
    pub animation: crate::fbx_animation::AnimationAsset,
}

#[derive(Clone, Copy, Debug)]
pub struct DrawRange {
    pub first_vertex: u32,
    pub vertex_count: u32,
    pub texture_index: Option<usize>,
}

#[derive(Debug)]
pub enum FbxError {
    PathEncoding,
    Import(String),
    NoTriangles,
    InvalidGeometry,
}

impl fmt::Display for FbxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PathEncoding => write!(f, "FBX path is not valid UTF-8"),
            Self::Import(message) => write!(f, "FBX import failed: {message}"),
            Self::NoTriangles => write!(f, "FBX contains no renderable triangles"),
            Self::InvalidGeometry => write!(f, "FBX contains invalid or non-finite geometry"),
        }
    }
}

impl Error for FbxError {}

pub fn load(path: &Path) -> Result<MeshData, FbxError> {
    let path = path.to_str().ok_or(FbxError::PathEncoding)?;
    let mut vertices = Vec::new();
    let mut draw_ranges = Vec::<DrawRange>::new();
    let mut textures = Vec::<crate::texture::TextureImage>::new();
    let mut texture_indices = std::collections::HashMap::<String, Option<usize>>::new();
    let mut bounds_min = [f32::INFINITY; 3];
    let mut bounds_max = [f32::NEG_INFINITY; 3];
    let mut source_meshes = 0;
    let load_opts = ufbx::LoadOpts {
        pivot_handling: ufbx::PivotHandling::AdjustToPivot,
        geometry_transform_handling: ufbx::GeometryTransformHandling::ModifyGeometry,
        space_conversion: ufbx::SpaceConversion::ModifyGeometry,
        target_unit_meters: 1.0,
        generate_missing_normals: true,
        ..Default::default()
    };
    let scene = ufbx::load_file(path, load_opts)
        .map_err(|error| FbxError::Import(format!("{} ({:?})", error.description, error.type_)))?;
    let hierarchy = crate::fbx_hierarchy::Hierarchy::from_scene(&scene)
        .map_err(|error| FbxError::Import(error.to_string()))?;
    let animation = crate::fbx_animation::from_scene(&scene, 30.0)
        .map_err(|error| FbxError::Import(error.to_string()))?;
    let mut materials = Vec::new();
    for node in &scene.nodes {
        let Some(mesh) = &node.mesh else { continue };
        if !mesh.vertex_position.exists {
            continue;
        }
        source_meshes += 1;
        let mut node_materials = crate::fbx_materials::extract_materials(node);
        for material in &mut node_materials {
            let Some(binding) = material.base_color_texture_binding() else {
                continue;
            };
            let base_color_uv_set = binding.uv_set.clone();
            let key = texture_cache_key(binding);
            let texture_index = if let Some(&index) = texture_indices.get(&key) {
                index
            } else {
                let index = match load_material_texture(binding, path) {
                    Ok(Some(image)) => {
                        let index = textures.len();
                        textures.push(image);
                        Some(index)
                    }
                    Ok(None) => {
                        eprintln!(
                            "FBX texture not found for material {}: {}",
                            material.material_name, binding.filename
                        );
                        None
                    }
                    Err(error) => {
                        eprintln!(
                            "{error}; using material color for {}",
                            material.material_name
                        );
                        None
                    }
                };
                texture_indices.insert(key, index);
                index
            };
            material.base_color_texture = texture_index;
            if texture_index.is_some() {
                material.base_color_uv_set = base_color_uv_set;
            }
        }
        materials.extend(node_materials.iter().cloned());
        let mut triangle_indices = Vec::new();
        for (face_index, face) in mesh.faces.iter().enumerate() {
            triangle_indices.resize((face.num_indices as usize).saturating_sub(2) * 3, 0);
            if triangle_indices.is_empty() {
                continue;
            }
            let count = ufbx::triangulate_face(&mut triangle_indices, mesh, *face) as usize * 3;
            let base_color = node_materials
                .iter()
                .find(|material| material.faces.contains(&face_index))
                .map(|material| material.base_color)
                .unwrap_or([1.0, 1.0, 1.0, 1.0]);
            let material_slot = mesh
                .face_material
                .get(face_index)
                .copied()
                .map(|slot| slot as usize);
            let texture_index = material_slot
                .and_then(|slot| node_materials.get(slot))
                .and_then(|material| material.base_color_texture);
            let uv_set = material_slot
                .and_then(|slot| node_materials.get(slot))
                .map(|material| material.base_color_uv_set.as_str())
                .filter(|name| !name.is_empty())
                .and_then(|name| mesh.uv_sets.iter().find(|set| set.name.as_ref() == name))
                .map(|set| &set.vertex_uv)
                .filter(|uv| uv.exists)
                .unwrap_or(&mesh.vertex_uv);
            let first_vertex = vertices.len() as u32;
            for &index in &triangle_indices[..count] {
                let position = mesh.vertex_position[index as usize];
                let position = ufbx::transform_position(&node.geometry_to_world, position);
                let p = [position.x as f32, position.y as f32, position.z as f32];
                if !p.iter().all(|value| value.is_finite()) {
                    return Err(FbxError::InvalidGeometry);
                }
                for axis in 0..3 {
                    bounds_min[axis] = bounds_min[axis].min(p[axis]);
                    bounds_max[axis] = bounds_max[axis].max(p[axis]);
                }
                let normal = if mesh.vertex_normal.exists
                    && (index as usize) < mesh.vertex_normal.indices.len()
                {
                    let n = mesh.vertex_normal[index as usize];
                    let normal_matrix = ufbx::matrix_for_normals(&node.geometry_to_world);
                    let normal = ufbx::transform_direction(&normal_matrix, n);
                    normalize([normal.x as f32, normal.y as f32, normal.z as f32])
                } else {
                    [0.0, 0.0, 1.0]
                };
                let uv = if uv_set.exists && (index as usize) < uv_set.indices.len() {
                    let value = uv_set[index as usize];
                    [value.x as f32, 1.0 - value.y as f32]
                } else {
                    [0.0, 0.0]
                };
                let influence = crate::fbx_skin::influence_for_corner(
                    &animation,
                    node.element.element_id,
                    mesh,
                    index as usize,
                );
                vertices.push(Vertex {
                    position: p,
                    normal,
                    uv,
                    base_color,
                    joints: influence.joints,
                    weights: influence.weights,
                });
            }
            let vertex_count = vertices.len() as u32 - first_vertex;
            if vertex_count > 0 {
                if let Some(last) = draw_ranges.last_mut().filter(|range| {
                    range.texture_index == texture_index
                        && range.first_vertex + range.vertex_count == first_vertex
                }) {
                    last.vertex_count += vertex_count;
                } else {
                    draw_ranges.push(DrawRange {
                        first_vertex,
                        vertex_count,
                        texture_index,
                    });
                }
            }
        }
    }

    eprintln!("Loaded {} FBX base-color textures", textures.len());
    for texture in &textures {
        eprintln!(
            "  {} ({}x{})",
            texture.source, texture.width, texture.height
        );
    }

    if vertices.is_empty() {
        return Err(FbxError::NoTriangles);
    }
    let triangles = vertices.len() / 3;
    Ok(MeshData {
        vertices,
        draw_ranges,
        textures,
        bounds_min,
        bounds_max,
        source_meshes,
        triangles,
        materials,
        hierarchy,
        animation,
    })
}

fn texture_cache_key(binding: &crate::fbx_materials::TextureBinding) -> String {
    if !binding.embedded_content.is_empty() {
        format!("embedded:{}", binding.filename.to_ascii_lowercase())
    } else {
        binding
            .absolute_filename
            .as_str()
            .trim()
            .to_ascii_lowercase()
            .to_owned()
            + "|"
            + &binding.relative_filename.to_ascii_lowercase()
            + "|"
            + &binding.filename.to_ascii_lowercase()
    }
}

fn load_material_texture(
    binding: &crate::fbx_materials::TextureBinding,
    model_path: &str,
) -> Result<Option<crate::texture::TextureImage>, String> {
    let mut decode_error = None;
    if !binding.embedded_content.is_empty() {
        match crate::texture::decode_png(
            format!("embedded:{}", binding.filename),
            &binding.embedded_content,
        ) {
            Ok(image) => return Ok(Some(image)),
            Err(error) => decode_error = Some(error),
        }
    }

    let model_path = Path::new(model_path);
    let model_dir = model_path.parent().unwrap_or_else(|| Path::new("."));
    let mut candidates = Vec::new();
    for filename in [
        binding.absolute_filename.as_str(),
        binding.filename.as_str(),
        binding.relative_filename.as_str(),
    ] {
        if filename.trim().is_empty() {
            continue;
        }
        let candidate = Path::new(filename);
        if candidate.is_absolute() {
            candidates.push(candidate.to_path_buf());
        }
        candidates.push(model_dir.join(candidate));
    }
    for candidate in candidates {
        if !candidate.is_file() {
            continue;
        }
        let bytes = std::fs::read(&candidate)
            .map_err(|error| format!("read FBX texture {}: {error}", candidate.display()))?;
        let source = candidate.display().to_string();
        return crate::texture::decode_png(source, &bytes)
            .map(Some)
            .map_err(|error| {
                decode_error.map_or(error.clone(), |embedded| format!("{embedded}; {error}"))
            });
    }

    let basename = [
        binding.filename.as_str(),
        binding.relative_filename.as_str(),
        binding.absolute_filename.as_str(),
    ]
    .into_iter()
    .filter_map(|name| Path::new(name).file_name())
    .next();
    if let (Some(basename), Some(search_root)) = (basename, model_dir.parent()) {
        if let Some(candidate) = find_file_by_name(search_root, basename, 8) {
            let bytes = std::fs::read(&candidate)
                .map_err(|error| format!("read FBX texture {}: {error}", candidate.display()))?;
            let source = candidate.display().to_string();
            return crate::texture::decode_png(source, &bytes)
                .map(Some)
                .map_err(|error| {
                    decode_error.map_or(error.clone(), |embedded| format!("{embedded}; {error}"))
                });
        }
    }
    if let Some(error) = decode_error {
        return Err(error);
    }
    Ok(None)
}

fn find_file_by_name(
    directory: &Path,
    basename: &std::ffi::OsStr,
    remaining_depth: usize,
) -> Option<std::path::PathBuf> {
    if remaining_depth == 0 {
        return None;
    }
    for entry in std::fs::read_dir(directory).ok()?.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_file()
            && entry
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case(&basename.to_string_lossy())
        {
            return Some(path);
        }
        if file_type.is_dir() {
            if let Some(found) = find_file_by_name(&path, basename, remaining_depth - 1) {
                return Some(found);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_requested_avatar_fbx() {
        let path = Path::new(r"G:\Tsugu\sample\MUMUS_Avatar\fbx\Mumus_fbx.fbx");
        if !path.exists() {
            return;
        }
        let model = load(path).expect("avatar FBX should import");
        assert!(model.source_meshes > 0);
        assert!(model.triangles > 0);
        assert_eq!(model.vertices.len(), model.triangles * 3);
        assert!(model
            .vertices
            .iter()
            .all(|vertex| vertex.position.iter().all(|v| v.is_finite())));
        assert!(model.bounds_max[0] - model.bounds_min[0] > 0.01);
        assert!(model.bounds_max[1] - model.bounds_min[1] > 0.01);
        assert!(model.bounds_max[2] - model.bounds_min[2] > 0.01);
    }
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len.is_finite() && len > 1e-8 {
        [v[0] / len, v[1] / len, v[2] / len]
    } else {
        [0.0, 0.0, 1.0]
    }
}
