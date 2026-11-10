//! FBX material extraction kept separate from vertex decoding/rendering.
//!
//! Material indices in `face_material` are mesh-local slots. For instanced
//! meshes, ufbx recommends resolving those slots through `Node::materials`
//! where possible, so this module does that before falling back to
//! `Mesh::materials`.

/// A texture reference recorded on a material property.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextureBinding {
    pub property: String,
    pub shader_property: String,
    pub filename: String,
    pub absolute_filename: String,
    pub relative_filename: String,
    pub uv_set: String,
    pub embedded_content: Arc<[u8]>,
}

/// A material's render-facing values and the faces that use its slot.
#[derive(Clone, Debug, PartialEq)]
pub struct MaterialBinding {
    pub node_name: String,
    pub material_name: String,
    /// Index into the node/mesh material slot list.
    pub slot: usize,
    /// Face indices in the source mesh assigned to this slot.
    pub faces: Vec<usize>,
    pub base_color: [f32; 4],
    pub diffuse_color: [f32; 4],
    pub emissive_color: [f32; 4],
    pub metallic: f32,
    pub roughness: f32,
    pub textures: Vec<TextureBinding>,
    pub base_color_texture: Option<usize>,
    pub base_color_uv_set: String,
}

impl MaterialBinding {
    pub fn base_color_texture_binding(&self) -> Option<&TextureBinding> {
        self.textures.iter().find(|binding| {
            let property = format!("{} {}", binding.property, binding.shader_property)
                .to_ascii_lowercase()
                .replace([' ', '_', '-'], "");
            property.contains("basecolor") || property.contains("diffusecolor")
        })
    }
}

/// Extract material slots and face assignments for one mesh node.
///
/// Returns an empty list for nodes without meshes or without material slots.
/// Missing FBX values use conservative renderer defaults; texture paths are
/// preserved as authored and are not opened or rewritten here.
pub fn extract_materials(node: &ufbx::Node) -> Vec<MaterialBinding> {
    let Some(mesh_ref) = node.mesh.as_ref() else {
        return Vec::new();
    };
    let mesh = mesh_ref.as_ref();
    let slots = if !node.materials.is_empty() {
        node.materials.as_ref()
    } else {
        mesh.materials.as_ref()
    };

    slots
        .iter()
        .enumerate()
        .map(|(slot, material_ref)| {
            let material = material_ref.as_ref();
            let faces = mesh
                .face_material
                .iter()
                .enumerate()
                .filter_map(|(face, &material_slot)| {
                    (material_slot as usize == slot).then_some(face)
                })
                .collect();

            let pbr_base = &material.pbr.base_color;
            let fbx_diffuse = &material.fbx.diffuse_color;
            let base_color = if pbr_base.has_value {
                vec4(pbr_base.value_vec4)
            } else if fbx_diffuse.has_value {
                vec4(fbx_diffuse.value_vec4)
            } else {
                [1.0, 1.0, 1.0, 1.0]
            };
            let diffuse_color = if fbx_diffuse.has_value {
                vec4(fbx_diffuse.value_vec4)
            } else {
                base_color
            };
            let emission = &material.fbx.emission_color;
            let emissive_color = if emission.has_value {
                vec4(emission.value_vec4)
            } else {
                [0.0, 0.0, 0.0, 1.0]
            };
            let metallic = if material.pbr.metalness.has_value {
                material.pbr.metalness.value_vec4.x as f32
            } else {
                0.0
            };
            let roughness = if material.pbr.roughness.has_value {
                material.pbr.roughness.value_vec4.x as f32
            } else {
                1.0
            };
            let textures = material
                .textures
                .iter()
                .map(|binding| {
                    let texture = binding.texture.as_ref();
                    TextureBinding {
                        property: binding.material_prop.to_string(),
                        shader_property: binding.shader_prop.to_string(),
                        filename: texture.filename.to_string(),
                        absolute_filename: texture.absolute_filename.to_string(),
                        relative_filename: texture.relative_filename.to_string(),
                        uv_set: texture.uv_set.to_string(),
                        embedded_content: Arc::from(&texture.content[..]),
                    }
                })
                .collect();

            MaterialBinding {
                node_name: node.element.name.to_string(),
                material_name: material.element.name.to_string(),
                slot,
                faces,
                base_color,
                diffuse_color,
                emissive_color,
                metallic,
                roughness,
                textures,
                base_color_texture: None,
                base_color_uv_set: String::new(),
            }
        })
        .collect()
}

fn vec4(value: ufbx::Vec4) -> [f32; 4] {
    [
        value.x as f32,
        value.y as f32,
        value.z as f32,
        value.w as f32,
    ]
}
use std::sync::Arc;
