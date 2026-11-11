//! Game objects, components, and shared model assets.

use crate::{fbx, fbx_animation};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ModelId(usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ObjectId(usize);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub translation: [f32; 3],
    /// Quaternion in `[x, y, z, w]` order.
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            translation: [0.0; 3],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0; 3],
        }
    }
}

impl Transform {
    pub fn matrix(self) -> [[f32; 4]; 4] {
        let [x, y, z, w] = normalize_quaternion(self.rotation);
        let [sx, sy, sz] = self.scale;
        let [tx, ty, tz] = self.translation;

        // Column-major matrix, matching WGSL's `matrix * vector` convention.
        [
            [
                (1.0 - 2.0 * (y * y + z * z)) * sx,
                (2.0 * (x * y + z * w)) * sx,
                (2.0 * (x * z - y * w)) * sx,
                0.0,
            ],
            [
                (2.0 * (x * y - z * w)) * sy,
                (1.0 - 2.0 * (x * x + z * z)) * sy,
                (2.0 * (y * z + x * w)) * sy,
                0.0,
            ],
            [
                (2.0 * (x * z + y * w)) * sz,
                (2.0 * (y * z - x * w)) * sz,
                (1.0 - 2.0 * (x * x + y * y)) * sz,
                0.0,
            ],
            [tx, ty, tz, 1.0],
        ]
    }
}

fn normalize_quaternion([x, y, z, w]: [f32; 4]) -> [f32; 4] {
    let length = (x * x + y * y + z * z + w * w).sqrt();
    if length <= f32::EPSILON {
        [0.0, 0.0, 0.0, 1.0]
    } else {
        [x / length, y / length, z / length, w / length]
    }
}

pub fn multiply_matrices(a: [[f32; 4]; 4], b: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut result = [[0.0; 4]; 4];
    for column in 0..4 {
        for row in 0..4 {
            result[column][row] = (0..4).map(|k| a[k][row] * b[column][k]).sum();
        }
    }
    result
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MeshRenderer {
    pub model: ModelId,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Animator {
    pub clip_index: usize,
    pub elapsed_seconds: f32,
    pub playing: bool,
    pub looping: bool,
}

impl Animator {
    pub fn new(clip_index: usize) -> Self {
        Self {
            clip_index,
            elapsed_seconds: 0.0,
            playing: true,
            looping: true,
        }
    }

    pub fn sample<'a>(
        &self,
        animation: &'a fbx_animation::AnimationAsset,
    ) -> Option<&'a fbx_animation::AnimationFrame> {
        let clip = animation.clips.get(self.clip_index)?;
        if clip.frames.is_empty() {
            return None;
        }
        let duration = (clip.end_seconds - clip.start_seconds).max(1.0 / 30.0);
        let elapsed = if self.looping {
            self.elapsed_seconds.rem_euclid(duration)
        } else {
            self.elapsed_seconds.min(duration)
        };
        let time = clip.start_seconds + elapsed;
        let frame_index = clip
            .frames
            .partition_point(|frame| frame.time_seconds <= time)
            .saturating_sub(1)
            .min(clip.frames.len() - 1);
        clip.frames.get(frame_index)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Component {
    MeshRenderer(MeshRenderer),
    Animator(Animator),
}

#[derive(Clone, Debug)]
pub struct GameObject {
    pub id: ObjectId,
    pub name: String,
    pub transform: Transform,
    parent: Option<ObjectId>,
    components: Vec<Component>,
}

impl GameObject {
    pub fn parent(&self) -> Option<ObjectId> {
        self.parent
    }

    pub fn components(&self) -> &[Component] {
        &self.components
    }

    pub fn add_component(&mut self, component: Component) -> bool {
        let already_has_component = match &component {
            Component::MeshRenderer(_) => self
                .components
                .iter()
                .any(|existing| matches!(existing, Component::MeshRenderer(_))),
            Component::Animator(_) => self
                .components
                .iter()
                .any(|existing| matches!(existing, Component::Animator(_))),
        };
        if already_has_component {
            return false;
        }
        self.components.push(component);
        true
    }

    pub fn animator(&self) -> Option<&Animator> {
        self.components
            .iter()
            .find_map(|component| match component {
                Component::Animator(animator) => Some(animator),
                _ => None,
            })
    }

    fn animator_mut(&mut self) -> Option<&mut Animator> {
        self.components
            .iter_mut()
            .find_map(|component| match component {
                Component::Animator(animator) => Some(animator),
                _ => None,
            })
    }
}

#[derive(Debug)]
pub struct ModelAsset {
    pub mesh: fbx::MeshData,
}

#[derive(Default)]
pub struct Scene {
    models: Vec<ModelAsset>,
    objects: Vec<GameObject>,
}

impl Scene {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_model(&mut self, mesh: fbx::MeshData) -> ModelId {
        let id = ModelId(self.models.len());
        self.models.push(ModelAsset { mesh });
        id
    }

    pub fn model(&self, id: ModelId) -> Option<&ModelAsset> {
        self.models.get(id.0)
    }

    pub fn create_game_object(&mut self, name: impl Into<String>) -> ObjectId {
        let id = ObjectId(self.objects.len());
        self.objects.push(GameObject {
            id,
            name: name.into(),
            transform: Transform::default(),
            parent: None,
            components: Vec::new(),
        });
        id
    }

    pub fn spawn_model(&mut self, name: impl Into<String>, model: ModelId) -> Option<ObjectId> {
        let animation_clip_count = self.model(model)?.mesh.animation.clips.len();
        let id = self.create_game_object(name);
        if !self.add_component(id, Component::MeshRenderer(MeshRenderer { model })) {
            return None;
        }
        if animation_clip_count > 0 {
            self.add_component(id, Component::Animator(Animator::new(0)));
        }
        Some(id)
    }

    pub fn add_component(&mut self, id: ObjectId, component: Component) -> bool {
        let Some(object) = self.object_mut(id) else {
            return false;
        };
        object.add_component(component)
    }

    pub fn set_parent(&mut self, child: ObjectId, parent: Option<ObjectId>) -> bool {
        if self.object(child).is_none()
            || parent.is_some_and(|parent| self.object(parent).is_none())
        {
            return false;
        }
        if parent == Some(child) {
            return false;
        }
        let mut ancestor = parent;
        while let Some(id) = ancestor {
            if id == child {
                return false;
            }
            ancestor = self.object(id).and_then(GameObject::parent);
        }
        self.object_mut(child).is_some_and(|object| {
            object.parent = parent;
            true
        })
    }

    pub fn object(&self, id: ObjectId) -> Option<&GameObject> {
        self.objects.get(id.0).filter(|object| object.id == id)
    }

    pub fn object_mut(&mut self, id: ObjectId) -> Option<&mut GameObject> {
        self.objects.get_mut(id.0).filter(|object| object.id == id)
    }

    pub fn objects(&self) -> &[GameObject] {
        &self.objects
    }

    pub fn world_matrix(&self, id: ObjectId) -> Option<[[f32; 4]; 4]> {
        let mut chain = Vec::new();
        let mut current = Some(id);
        while let Some(object_id) = current {
            let object = self.object(object_id)?;
            chain.push(object.transform.matrix());
            current = object.parent();
        }
        Some(
            chain
                .into_iter()
                .rev()
                .fold(identity_matrix(), multiply_matrices),
        )
    }

    pub fn update(&mut self, delta_seconds: f32) {
        let delta_seconds = delta_seconds.max(0.0);
        for object in &mut self.objects {
            if let Some(animator) = object.animator_mut().filter(|animator| animator.playing) {
                animator.elapsed_seconds += delta_seconds;
            }
        }
    }
}

pub fn identity_matrix() -> [[f32; 4]; 4] {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}
