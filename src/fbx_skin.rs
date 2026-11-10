//! Skinning influences copied from an imported ufbx mesh.
//!
//! Keeping this conversion in a separate module lets renderers choose their own
//! GPU layout while the importer retains all source influences on the CPU.

/// Up to four normalized bone weights for one polygon-corner vertex.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SkinInfluence {
    pub joints: [u32; 4],
    pub weights: [f32; 4],
}

/// Resolve a triangulated polygon-corner index to the mesh control point and
/// copy its strongest four influences. Missing skin data produces an identity
/// influence with zero weights; the caller can then keep rendering it as a
/// static vertex.
pub fn influence_for_corner(
    animation: &crate::fbx_animation::AnimationAsset,
    node_id: u32,
    mesh: &ufbx::Mesh,
    corner_index: usize,
) -> SkinInfluence {
    let Some(&control_point) = mesh.vertex_indices.get(corner_index) else {
        return SkinInfluence::default();
    };
    let Some(skinned_mesh) = animation
        .skinned_meshes
        .iter()
        .find(|skinned_mesh| skinned_mesh.source_node_id == node_id)
    else {
        return SkinInfluence::default();
    };
    let Some(mesh_index) = animation
        .skinned_meshes
        .iter()
        .position(|candidate| candidate.source_node_id == node_id)
    else {
        return SkinInfluence::default();
    };
    let Some(influences) = skinned_mesh.vertex_influences.get(control_point as usize) else {
        return SkinInfluence::default();
    };

    let mut strongest: Vec<_> = influences
        .iter()
        .filter(|influence| influence.weight.is_finite() && influence.weight > 0.0)
        .map(|influence| {
            (
                (mesh_index * animation.bones.len() + influence.bone_index) as u32,
                influence.weight,
            )
        })
        .collect();
    strongest.sort_by(|a, b| b.1.total_cmp(&a.1));
    strongest.truncate(4);
    let total: f32 = strongest.iter().map(|(_, weight)| *weight).sum();
    if !total.is_finite() || total <= f32::EPSILON {
        return SkinInfluence::default();
    }

    let mut influence = SkinInfluence::default();
    for (slot, (joint, weight)) in strongest.into_iter().enumerate() {
        influence.joints[slot] = joint;
        influence.weights[slot] = weight / total;
    }
    influence
}
