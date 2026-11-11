//! Game objects, components, and shared model assets.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{fbx, fbx_animation};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ModelId(usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ObjectId(usize);

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
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
    Transform(Transform),
    MeshRenderer(MeshRenderer),
    Animator(Animator),
}

#[derive(Clone, Debug)]
pub struct GameObject {
    pub id: ObjectId,
    pub name: String,
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

    pub fn transform(&self) -> &Transform {
        self.components
            .iter()
            .find_map(|component| match component {
                Component::Transform(transform) => Some(transform),
                _ => None,
            })
            .expect("every game object has a Transform component")
    }

    pub fn transform_mut(&mut self) -> &mut Transform {
        self.components
            .iter_mut()
            .find_map(|component| match component {
                Component::Transform(transform) => Some(transform),
                _ => None,
            })
            .expect("every game object has a Transform component")
    }

    pub fn add_component(&mut self, component: Component) -> bool {
        let already_has_component = match &component {
            Component::Transform(_) => self
                .components
                .iter()
                .any(|existing| matches!(existing, Component::Transform(_))),
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
    pub source_path: PathBuf,
    pub mesh: fbx::MeshData,
}

#[derive(Serialize, Deserialize)]
struct SceneDocument {
    format_version: u32,
    objects: Vec<GameObjectDocument>,
}

#[derive(Serialize, Deserialize)]
struct GameObjectDocument {
    name: String,
    parent: Option<usize>,
    transform: Transform,
    components: Vec<ComponentDocument>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ComponentDocument {
    MeshRenderer {
        path: String,
    },
    Animator {
        clip_index: usize,
        elapsed_seconds: f32,
        playing: bool,
        looping: bool,
    },
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

    pub fn add_model(&mut self, source_path: PathBuf, mesh: fbx::MeshData) -> ModelId {
        let id = ModelId(self.models.len());
        self.models.push(ModelAsset { source_path, mesh });
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
            parent: None,
            components: vec![Component::Transform(Transform::default())],
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

    pub fn save_bson(&self, path: &Path) -> Result<(), String> {
        let objects = self
            .objects
            .iter()
            .map(|object| {
                let transform = *object.transform();
                let mut components = Vec::new();
                for component in object.components() {
                    match component {
                        Component::Transform(_) => {}
                        Component::MeshRenderer(renderer) => {
                            let model = self.model(renderer.model).ok_or_else(|| {
                                format!("object '{}' references a missing model", object.name)
                            })?;
                            let model_path = model.source_path.to_str().ok_or_else(|| {
                                format!("model path is not valid UTF-8: {:?}", model.source_path)
                            })?;
                            components.push(ComponentDocument::MeshRenderer {
                                path: model_path.to_owned(),
                            });
                        }
                        Component::Animator(animator) => {
                            components.push(ComponentDocument::Animator {
                                clip_index: animator.clip_index,
                                elapsed_seconds: animator.elapsed_seconds,
                                playing: animator.playing,
                                looping: animator.looping,
                            });
                        }
                    }
                }
                Ok(GameObjectDocument {
                    name: object.name.clone(),
                    parent: object.parent().map(|parent| parent.0),
                    transform,
                    components,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let document = SceneDocument {
            format_version: 1,
            objects,
        };
        let bytes = bson::to_vec(&document).map_err(|error| error.to_string())?;
        fs::write(path, bytes).map_err(|error| format!("write scene {}: {error}", path.display()))
    }

    pub fn load_bson(path: &Path) -> Result<Self, String> {
        let bytes =
            fs::read(path).map_err(|error| format!("read scene {}: {error}", path.display()))?;
        let document: SceneDocument =
            bson::from_slice(&bytes).map_err(|error| format!("decode scene BSON: {error}"))?;
        if document.format_version != 1 {
            return Err(format!(
                "unsupported scene format version {}",
                document.format_version
            ));
        }

        let mut scene = Scene::new();
        let mut object_ids = Vec::with_capacity(document.objects.len());
        for object_document in &document.objects {
            let object_id = scene.create_game_object(object_document.name.clone());
            *scene
                .object_mut(object_id)
                .expect("newly created game object exists")
                .transform_mut() = object_document.transform;
            object_ids.push(object_id);
        }

        let mut loaded_models = HashMap::<String, ModelId>::new();
        for (index, object_document) in document.objects.iter().enumerate() {
            let object_id = object_ids[index];
            if let Some(parent_index) = object_document.parent {
                let parent_id = *object_ids.get(parent_index).ok_or_else(|| {
                    format!(
                        "object '{}' has an invalid parent index",
                        object_document.name
                    )
                })?;
                if !scene.set_parent(object_id, Some(parent_id)) {
                    return Err(format!(
                        "invalid parent relationship for '{}'",
                        object_document.name
                    ));
                }
            }

            for component in &object_document.components {
                let restored = match component {
                    ComponentDocument::MeshRenderer { path } => {
                        let model_id = if let Some(model_id) = loaded_models.get(path) {
                            *model_id
                        } else {
                            let model_path = PathBuf::from(path);
                            let model = fbx::load(&model_path).map_err(|error| {
                                format!(
                                    "load model referenced by scene {}: {error}",
                                    model_path.display()
                                )
                            })?;
                            let model_id = scene.add_model(model_path, model);
                            loaded_models.insert(path.clone(), model_id);
                            model_id
                        };
                        Component::MeshRenderer(MeshRenderer { model: model_id })
                    }
                    ComponentDocument::Animator {
                        clip_index,
                        elapsed_seconds,
                        playing,
                        looping,
                    } => Component::Animator(Animator {
                        clip_index: *clip_index,
                        elapsed_seconds: *elapsed_seconds,
                        playing: *playing,
                        looping: *looping,
                    }),
                };
                if !scene.add_component(object_id, restored) {
                    return Err(format!("duplicate component on '{}'", object_document.name));
                }
            }
        }
        Ok(scene)
    }

    pub fn world_matrix(&self, id: ObjectId) -> Option<[[f32; 4]; 4]> {
        let mut chain = Vec::new();
        let mut current = Some(id);
        while let Some(object_id) = current {
            let object = self.object(object_id)?;
            chain.push(object.transform().matrix());
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
