//! FBX skeleton, skin-weight, and animation extraction backed by `ufbx`.
//!
//! This module exposes CPU-side animation data. The renderer can upload these
//! transforms and skin influences later without coupling FBX parsing to wgpu.

use std::{error::Error, fmt, path::Path};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub translation: [f32; 3],
    /// Quaternion in `[x, y, z, w]` order.
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}

#[derive(Clone, Debug)]
pub struct Bone {
    /// Dense index used by `parent` and `SkinInfluence`.
    pub index: usize,
    /// Stable ID assigned by ufbx for the source node.
    pub source_node_id: u32,
    pub name: String,
    pub parent: Option<usize>,
    pub bind_local: Transform,
    /// Bind-pose transform from this bone's local space to scene space.
    pub bind_world: [[f32; 4]; 4],
}

#[derive(Clone, Copy, Debug)]
pub struct SkinInfluence {
    pub bone_index: usize,
    pub weight: f32,
}

#[derive(Clone, Debug)]
pub struct SkinnedMesh {
    pub source_node_id: u32,
    pub name: String,
    /// Control-point influences, indexed by source FBX vertex/control point.
    pub vertex_influences: Vec<Vec<SkinInfluence>>,
}

#[derive(Clone, Debug)]
pub struct AnimationFrame {
    pub time_seconds: f32,
    /// One local-space transform per `AnimationAsset::bones` entry.
    pub bone_transforms: Vec<Transform>,
}

#[derive(Clone, Debug)]
pub struct AnimationClip {
    pub name: String,
    pub start_seconds: f32,
    pub end_seconds: f32,
    pub frames: Vec<AnimationFrame>,
}

#[derive(Clone, Debug)]
pub struct AnimationAsset {
    pub bones: Vec<Bone>,
    pub skinned_meshes: Vec<SkinnedMesh>,
    pub clips: Vec<AnimationClip>,
}

#[derive(Debug)]
pub enum AnimationImportError {
    PathEncoding,
    Import(String),
    InvalidSampleRate,
    TooManySamples,
}

impl fmt::Display for AnimationImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PathEncoding => write!(f, "FBX path is not valid UTF-8"),
            Self::Import(message) => write!(f, "FBX animation import failed: {message}"),
            Self::InvalidSampleRate => {
                write!(f, "animation sample rate must be finite and positive")
            }
            Self::TooManySamples => {
                write!(f, "animation clip exceeds the 100,000-frame import limit")
            }
        }
    }
}

impl Error for AnimationImportError {}

/// Loads skeleton hierarchy, per-control-point skin weights, and uniformly
/// sampled animation clips from an FBX file.
///
/// `sample_rate` is expressed in frames per second. Each clip includes its
/// start and end samples where possible; an empty/zero-length clip gets one.
pub fn load(path: &Path, sample_rate: f64) -> Result<AnimationAsset, AnimationImportError> {
    if !sample_rate.is_finite() || sample_rate <= 0.0 {
        return Err(AnimationImportError::InvalidSampleRate);
    }

    let path = path.to_str().ok_or(AnimationImportError::PathEncoding)?;
    let scene = ufbx::load_file(
        path,
        ufbx::LoadOpts {
            pivot_handling: ufbx::PivotHandling::AdjustToPivot,
            geometry_transform_handling: ufbx::GeometryTransformHandling::ModifyGeometry,
            space_conversion: ufbx::SpaceConversion::ModifyGeometry,
            target_unit_meters: 1.0,
            generate_missing_normals: true,
            ..Default::default()
        },
    )
    .map_err(|error| {
        AnimationImportError::Import(format!("{} ({:?})", error.description, error.type_))
    })?;

    from_scene(&scene, sample_rate)
}

/// Extract animation data from an already loaded scene.
pub fn from_scene(
    scene: &ufbx::Scene,
    sample_rate: f64,
) -> Result<AnimationAsset, AnimationImportError> {
    if !sample_rate.is_finite() || sample_rate <= 0.0 {
        return Err(AnimationImportError::InvalidSampleRate);
    }

    let mut bone_nodes = Vec::new();
    let mut bone_indices = std::collections::HashMap::new();
    for node in &scene.nodes {
        if node.bone.is_some() {
            let index = bone_nodes.len();
            bone_indices.insert(node.element.element_id, index);
            bone_nodes.push(node);
        }
    }

    // Some FBX files mark weighted cluster nodes as ordinary nodes rather than
    // as explicit Bone attributes. Include those nodes in the same skeleton.
    for cluster in &scene.skin_clusters {
        if let Some(node) = &cluster.bone_node {
            let node = node.as_ref();
            if !bone_indices.contains_key(&node.element.element_id) {
                let index = bone_nodes.len();
                bone_indices.insert(node.element.element_id, index);
                bone_nodes.push(node);
            }
        }
    }

    let bones = bone_nodes
        .iter()
        .enumerate()
        .map(|(index, node)| Bone {
            index,
            source_node_id: node.element.element_id,
            name: node.element.name.to_string(),
            parent: node
                .parent
                .as_ref()
                .and_then(|parent| bone_indices.get(&parent.element.element_id).copied()),
            bind_local: transform(node.local_transform),
            bind_world: matrix4(node.node_to_world),
        })
        .collect::<Vec<_>>();

    let mut skinned_meshes = Vec::new();
    for node in &scene.nodes {
        let Some(mesh) = &node.mesh else { continue };
        if mesh.skin_deformers.is_empty() {
            continue;
        }

        let mut vertex_influences: Vec<Vec<SkinInfluence>> = Vec::new();
        for deformer in &mesh.skin_deformers {
            if vertex_influences.len() < deformer.vertices.len() {
                vertex_influences.resize_with(deformer.vertices.len(), Vec::new);
            }
            for (vertex_index, skin_vertex) in deformer.vertices.iter().enumerate() {
                let start = skin_vertex.weight_begin as usize;
                let end = start.saturating_add(skin_vertex.num_weights as usize);
                if end > deformer.weights.len() {
                    continue;
                }
                for weight in &deformer.weights.as_ref()[start..end] {
                    let Some(cluster) = deformer.clusters.get(weight.cluster_index as usize) else {
                        continue;
                    };
                    let Some(bone_node) = &cluster.bone_node else {
                        continue;
                    };
                    let Some(&bone_index) = bone_indices.get(&bone_node.element.element_id) else {
                        continue;
                    };
                    let weight = weight.weight as f32;
                    if weight.is_finite() && weight > 0.0 {
                        vertex_influences[vertex_index].push(SkinInfluence { bone_index, weight });
                    }
                }
            }
        }
        skinned_meshes.push(SkinnedMesh {
            source_node_id: node.element.element_id,
            name: node.element.name.to_string(),
            vertex_influences,
        });
    }

    for mesh in &mut skinned_meshes {
        for influences in &mut mesh.vertex_influences {
            let mut merged = std::collections::BTreeMap::<usize, f32>::new();
            for influence in influences.drain(..) {
                *merged.entry(influence.bone_index).or_default() += influence.weight;
            }
            let total: f32 = merged.values().sum();
            if total.is_finite() && total > f32::EPSILON {
                influences.extend(
                    merged
                        .into_iter()
                        .map(|(bone_index, weight)| SkinInfluence {
                            bone_index,
                            weight: weight / total,
                        }),
                );
            }
        }
    }

    let mut clips = Vec::new();
    for stack in &scene.anim_stacks {
        let start = stack.time_begin;
        let end = stack.time_end.max(start);
        let frame_count = (((end - start) * sample_rate).ceil() as usize)
            .saturating_add(1)
            .max(1);
        if frame_count > 100_000 {
            return Err(AnimationImportError::TooManySamples);
        }
        let mut frames = Vec::with_capacity(frame_count);
        for frame_index in 0..frame_count {
            let time = (start + frame_index as f64 / sample_rate).min(end);
            let bone_transforms = bone_nodes
                .iter()
                .map(|node| transform(ufbx::evaluate_transform(&stack.anim, node, time)))
                .collect();
            frames.push(AnimationFrame {
                time_seconds: time as f32,
                bone_transforms,
            });
        }
        clips.push(AnimationClip {
            name: stack.element.name.to_string(),
            start_seconds: start as f32,
            end_seconds: end as f32,
            frames,
        });
    }

    Ok(AnimationAsset {
        bones,
        skinned_meshes,
        clips,
    })
}

/// Retarget a clip onto an existing model skeleton using Mixamo-to-model bone aliases.
pub fn retarget(source: &AnimationAsset, target: &AnimationAsset) -> (AnimationAsset, usize) {
    let mut map = Vec::new();
    for (source_index, bone) in source.bones.iter().enumerate() {
        let Some(alias) = target_bone_name(&bone.name) else {
            continue;
        };
        if let Some(target_bone) = target
            .bones
            .iter()
            .find(|candidate| candidate.name == alias)
        {
            map.push((source_index, target_bone.index));
        }
    }
    let clips = source
        .clips
        .iter()
        .map(|clip| {
            let frames = clip
                .frames
                .iter()
                .map(|frame| {
                    let mut transforms = target
                        .bones
                        .iter()
                        .map(|bone| bone.bind_local)
                        .collect::<Vec<_>>();
                    for &(source_index, target_index) in &map {
                        let src_bind = source.bones[source_index].bind_local;
                        let src_pose = frame.bone_transforms[source_index];
                        let dst_bind = target.bones[target_index].bind_local;
                        transforms[target_index] = Transform {
                            translation: std::array::from_fn(|axis| {
                                dst_bind.translation[axis] + src_pose.translation[axis]
                                    - src_bind.translation[axis]
                            }),
                            rotation: quat_mul(
                                dst_bind.rotation,
                                quat_mul(quat_inverse(src_bind.rotation), src_pose.rotation),
                            ),
                            scale: std::array::from_fn(|axis| {
                                dst_bind.scale[axis] * src_pose.scale[axis]
                                    / src_bind.scale[axis].abs().max(1e-6)
                            }),
                        };
                    }
                    AnimationFrame {
                        time_seconds: frame.time_seconds,
                        bone_transforms: transforms,
                    }
                })
                .collect();
            AnimationClip {
                name: clip.name.clone(),
                start_seconds: clip.start_seconds,
                end_seconds: clip.end_seconds,
                frames,
            }
        })
        .collect();
    (
        AnimationAsset {
            bones: target.bones.clone(),
            skinned_meshes: target.skinned_meshes.clone(),
            clips,
        },
        map.len(),
    )
}

fn target_bone_name(name: &str) -> Option<String> {
    let name = name.rsplit(':').next().unwrap_or(name);
    let side = if name.starts_with("Left") {
        Some("L")
    } else if name.starts_with("Right") {
        Some("R")
    } else {
        None
    };
    let body = match name {
        "Hips" => return Some("Hips".into()),
        "Spine" => return Some("Spine".into()),
        "Spine1" => return Some("Chest".into()),
        _ => name,
    };
    let Some(side) = side else { return None };
    let part = body
        .strip_prefix("Left")
        .or_else(|| body.strip_prefix("Right"))?;
    let mapped = match part {
        "Shoulder" => "Shoulder",
        "Arm" => "UpperArm",
        "ForeArm" => "LowerArm",
        "Hand" => "Hand",
        "UpLeg" => "UpperLeg",
        "Leg" => "LowerLeg",
        "Foot" => "Foot",
        "ToeBase" => "Toe",
        "HandThumb1" => "ThumbProximal",
        "HandThumb2" => "ThumbIntermediate",
        "HandThumb3" => "ThumbDistal",
        "HandIndex1" => "IndexProximal",
        "HandIndex2" => "IndexIntermediate",
        "HandIndex3" => "IndexDistal",
        "HandMiddle1" => "MiddleProximal",
        "HandMiddle2" => "MiddleIntermediate",
        "HandMiddle3" => "MiddleDistal",
        "HandRing1" => "RingProximal",
        "HandRing2" => "RingIntermediate",
        "HandRing3" => "RingDistal",
        "HandPinky1" => "LittleProximal",
        "HandPinky2" => "LittleIntermediate",
        "HandPinky3" => "LittleDistal",
        _ => return None,
    };
    Some(format!("{mapped}.{side}"))
}

fn quat_inverse(q: [f32; 4]) -> [f32; 4] {
    [-q[0], -q[1], -q[2], q[3]]
}
fn quat_mul(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let q = [
        a[3] * b[0] + a[0] * b[3] + a[1] * b[2] - a[2] * b[1],
        a[3] * b[1] - a[0] * b[2] + a[1] * b[3] + a[2] * b[0],
        a[3] * b[2] + a[0] * b[1] - a[1] * b[0] + a[2] * b[3],
        a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2],
    ];
    let n = q.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-8);
    q.map(|x| x / n)
}

fn matrix4(m: ufbx::Matrix) -> [[f32; 4]; 4] {
    [
        [m.m00 as f32, m.m10 as f32, m.m20 as f32, 0.0],
        [m.m01 as f32, m.m11 as f32, m.m21 as f32, 0.0],
        [m.m02 as f32, m.m12 as f32, m.m22 as f32, 0.0],
        [m.m03 as f32, m.m13 as f32, m.m23 as f32, 1.0],
    ]
}

/// Build per-mesh skin matrices in the same world-space coordinate system as imported vertices.
pub fn skin_palette(asset: &AnimationAsset, frame: Option<&AnimationFrame>) -> Vec<[[f32; 4]; 4]> {
    let transforms = frame.map(|frame| frame.bone_transforms.as_slice());
    let mut globals = vec![None; asset.bones.len()];
    fn global(
        index: usize,
        asset: &AnimationAsset,
        transforms: Option<&[Transform]>,
        cache: &mut [Option<[[f32; 4]; 4]>],
    ) -> [[f32; 4]; 4] {
        if let Some(value) = cache[index] {
            return value;
        }
        let bone = &asset.bones[index];
        let pose = transforms
            .and_then(|items| items.get(index))
            .copied()
            .unwrap_or(bone.bind_local);
        let local = compose(pose);
        let value = if let Some(parent) = bone.parent {
            // Preserve FBX nodes/pivots between bone nodes with a per-bone bind offset.
            // This makes the bind pose exactly identity even when the bone hierarchy
            // omits intermediary non-bone nodes.
            let parent_bind = asset.bones[parent].bind_world;
            let offset = mul(
                mul(inverse_affine(parent_bind), bone.bind_world),
                inverse_affine(compose(bone.bind_local)),
            );
            mul(mul(global(parent, asset, transforms, cache), offset), local)
        } else {
            let bind_local_inv = inverse_affine(compose(bone.bind_local));
            mul(mul(bone.bind_world, bind_local_inv), local)
        };
        cache[index] = Some(value);
        value
    }
    let worlds = (0..asset.bones.len())
        .map(|i| global(i, asset, transforms, &mut globals))
        .collect::<Vec<_>>();
    let mut palette = Vec::with_capacity(asset.bones.len() * asset.skinned_meshes.len());
    for _mesh in &asset.skinned_meshes {
        for (bone, world) in asset.bones.iter().zip(&worlds) {
            palette.push(mul(*world, inverse_affine(bone.bind_world)));
        }
    }
    palette
}

fn compose(t: Transform) -> [[f32; 4]; 4] {
    let [x, y, z, w] = t.rotation;
    let (x2, y2, z2) = (x + x, y + y, z + z);
    let (xx, xy, xz) = (x * x2, x * y2, x * z2);
    let (yy, yz, zz) = (y * y2, y * z2, z * z2);
    let (wx, wy, wz) = (w * x2, w * y2, w * z2);
    [
        [
            (1.0 - (yy + zz)) * t.scale[0],
            (xy + wz) * t.scale[0],
            (xz - wy) * t.scale[0],
            0.0,
        ],
        [
            (xy - wz) * t.scale[1],
            (1.0 - (xx + zz)) * t.scale[1],
            (yz + wx) * t.scale[1],
            0.0,
        ],
        [
            (xz + wy) * t.scale[2],
            (yz - wx) * t.scale[2],
            (1.0 - (xx + yy)) * t.scale[2],
            0.0,
        ],
        [t.translation[0], t.translation[1], t.translation[2], 1.0],
    ]
}
fn mul(a: [[f32; 4]; 4], b: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut c = [[0.0; 4]; 4];
    for col in 0..4 {
        for row in 0..4 {
            for k in 0..4 {
                c[col][row] += a[k][row] * b[col][k];
            }
        }
    }
    c
}
fn inverse_affine(m: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let a = m[0][0];
    let b = m[1][0];
    let c = m[2][0];
    let d = m[0][1];
    let e = m[1][1];
    let f = m[2][1];
    let g = m[0][2];
    let h = m[1][2];
    let i = m[2][2];
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    if det.abs() < 1e-10 {
        return identity4();
    };
    let q = 1.0 / det;
    let mut out = identity4();
    out[0][0] = (e * i - f * h) * q;
    out[1][0] = (c * h - b * i) * q;
    out[2][0] = (b * f - c * e) * q;
    out[0][1] = (f * g - d * i) * q;
    out[1][1] = (a * i - c * g) * q;
    out[2][1] = (c * d - a * f) * q;
    out[0][2] = (d * h - e * g) * q;
    out[1][2] = (b * g - a * h) * q;
    out[2][2] = (a * e - b * d) * q;
    for row in 0..3 {
        out[3][row] = -(out[0][row] * m[3][0] + out[1][row] * m[3][1] + out[2][row] * m[3][2]);
    }
    out
}
fn identity4() -> [[f32; 4]; 4] {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

fn transform(value: ufbx::Transform) -> Transform {
    Transform {
        translation: [
            value.translation.x as f32,
            value.translation.y as f32,
            value.translation.z as f32,
        ],
        rotation: [
            value.rotation.x as f32,
            value.rotation.y as f32,
            value.rotation.z as f32,
            value.rotation.w as f32,
        ],
        scale: [
            value.scale.x as f32,
            value.scale.y as f32,
            value.scale.z as f32,
        ],
    }
}
