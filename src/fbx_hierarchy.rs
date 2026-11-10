//! FBX scene node hierarchy extracted from `ufbx`.
//!
//! Node indices in this module are stable for the lifetime of a `Hierarchy` and
//! match the order of `ufbx::Scene::nodes`. Parent/child links use those indices,
//! so downstream systems can retain the tree without keeping the imported scene
//! alive.

use std::{collections::HashMap, error::Error, fmt};

/// A hierarchy copied from an imported FBX scene.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hierarchy {
    pub nodes: Vec<HierarchyNode>,
    /// Indices of nodes that do not have a parent in `scene.nodes`.
    pub roots: Vec<usize>,
}

/// One FBX transform node. Matrix values are row-major affine 3x4 values.
#[derive(Clone, Debug, PartialEq)]
pub struct HierarchyNode {
    /// Stable index in this hierarchy, matching the source scene node order.
    pub index: usize,
    /// FBX element identifier, useful when correlating other imported data.
    pub element_id: u32,
    pub name: String,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    /// Local node transform, excluding geometry transform.
    pub local_transform: [f64; 12],
    /// World node transform, excluding geometry transform.
    pub world_transform: [f64; 12],
    pub has_mesh: bool,
    pub is_bone: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HierarchyError {
    ParentNotInScene { child_id: u32, parent_id: u32 },
}

impl fmt::Display for HierarchyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ParentNotInScene { child_id, parent_id } => write!(
                f,
                "FBX node {child_id} references parent {parent_id}, which is absent from the scene node list"
            ),
        }
    }
}

impl Error for HierarchyError {}

impl Hierarchy {
    /// Copy the complete parent/child structure and transforms from an `ufbx` scene.
    pub fn from_scene(scene: &ufbx::Scene) -> Result<Self, HierarchyError> {
        let nodes: Vec<_> = (&scene.nodes).into_iter().collect();
        let index_by_id: HashMap<u32, usize> = nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.element.element_id, index))
            .collect();

        let mut hierarchy = Self {
            nodes: Vec::with_capacity(nodes.len()),
            roots: Vec::new(),
        };

        for (index, node) in nodes.iter().enumerate() {
            let parent =
                match node.parent.as_ref() {
                    Some(parent_node) => {
                        let parent_id = parent_node.element.element_id;
                        Some(*index_by_id.get(&parent_id).ok_or(
                            HierarchyError::ParentNotInScene {
                                child_id: node.element.element_id,
                                parent_id,
                            },
                        )?)
                    }
                    None => None,
                };

            hierarchy.nodes.push(HierarchyNode {
                index,
                element_id: node.element.element_id,
                name: node.element.name.as_ref().to_owned(),
                parent,
                children: Vec::new(),
                local_transform: matrix_to_row_major(&node.node_to_parent),
                world_transform: matrix_to_row_major(&node.node_to_world),
                has_mesh: node.mesh.is_some(),
                is_bone: node.bone.is_some(),
            });
        }

        for index in 0..hierarchy.nodes.len() {
            match hierarchy.nodes[index].parent {
                Some(parent) => hierarchy.nodes[parent].children.push(index),
                None => hierarchy.roots.push(index),
            }
        }

        Ok(hierarchy)
    }
}

fn matrix_to_row_major(matrix: &ufbx::Matrix) -> [f64; 12] {
    [
        matrix.m00, matrix.m01, matrix.m02, matrix.m03, matrix.m10, matrix.m11, matrix.m12,
        matrix.m13, matrix.m20, matrix.m21, matrix.m22, matrix.m23,
    ]
}
