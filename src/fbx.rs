use std::{error::Error, fmt, path::Path};

use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
}

#[derive(Debug)]
pub struct MeshData {
    pub vertices: Vec<Vertex>,
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
    pub source_meshes: usize,
    pub triangles: usize,
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
    for node in &scene.nodes {
        let Some(mesh) = &node.mesh else { continue };
        if !mesh.vertex_position.exists {
            continue;
        }
        source_meshes += 1;
        let mut triangle_indices = Vec::new();
        for face in &mesh.faces {
            triangle_indices.resize((face.num_indices as usize).saturating_sub(2) * 3, 0);
            if triangle_indices.is_empty() {
                continue;
            }
            let count = ufbx::triangulate_face(&mut triangle_indices, mesh, *face) as usize * 3;
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
                vertices.push(Vertex {
                    position: p,
                    normal,
                });
            }
        }
    }

    if vertices.is_empty() {
        return Err(FbxError::NoTriangles);
    }
    let triangles = vertices.len() / 3;
    Ok(MeshData {
        vertices,
        bounds_min,
        bounds_max,
        source_meshes,
        triangles,
    })
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
