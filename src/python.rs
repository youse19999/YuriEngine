use std::{
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard},
};

use pyo3::{
    exceptions::{PyRuntimeError, PyValueError},
    prelude::*,
};

use crate::{
    engine, fbx, fbx_animation,
    scene::{ObjectId, Scene},
};

fn scene_lock(scene: &Arc<Mutex<Scene>>) -> PyResult<MutexGuard<'_, Scene>> {
    scene
        .lock()
        .map_err(|_| PyRuntimeError::new_err("scene state lock poisoned"))
}

#[pyclass(name = "Scene")]
pub struct PyScene {
    inner: Arc<Mutex<Scene>>,
}

#[pymethods]
impl PyScene {
    #[new]
    fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Scene::new())),
        }
    }

    #[pyo3(signature = (path, animation_path=None, name=None))]
    fn load_fbx(
        &self,
        path: PathBuf,
        animation_path: Option<PathBuf>,
        name: Option<String>,
    ) -> PyResult<PyGameObject> {
        let mut model =
            fbx::load(&path).map_err(|error| PyValueError::new_err(error.to_string()))?;
        if let Some(animation_path) = animation_path {
            let animation = fbx_animation::load(&animation_path, 30.0)
                .map_err(|error| PyValueError::new_err(error.to_string()))?;
            model.animation = fbx_animation::retarget(&animation, &model.animation).0;
        }
        let object_name = name.unwrap_or_else(|| {
            path.file_stem()
                .and_then(std::ffi::OsStr::to_str)
                .unwrap_or("Model")
                .to_owned()
        });
        let mut scene = scene_lock(&self.inner)?;
        let model_id = scene.add_model(model);
        let id = scene
            .spawn_model(object_name, model_id)
            .ok_or_else(|| PyRuntimeError::new_err("failed to create model game object"))?;
        Ok(PyGameObject {
            scene: Arc::clone(&self.inner),
            id,
        })
    }

    fn create_game_object(&self, name: String) -> PyResult<PyGameObject> {
        let id = scene_lock(&self.inner)?.create_game_object(name);
        Ok(PyGameObject {
            scene: Arc::clone(&self.inner),
            id,
        })
    }

    fn __len__(&self) -> PyResult<usize> {
        Ok(scene_lock(&self.inner)?.objects().len())
    }
}

#[pyclass(name = "GameObject")]
pub struct PyGameObject {
    scene: Arc<Mutex<Scene>>,
    id: ObjectId,
}

impl PyGameObject {
    fn with_model<T>(&self, f: impl FnOnce(&fbx::MeshData) -> T) -> PyResult<T> {
        let scene = scene_lock(&self.scene)?;
        let object = scene
            .object(self.id)
            .ok_or_else(|| PyRuntimeError::new_err("game object no longer exists"))?;
        let renderer = object
            .mesh_renderer()
            .ok_or_else(|| PyValueError::new_err("game object has no MeshRenderer component"))?;
        let model = scene
            .model(renderer.model)
            .ok_or_else(|| PyRuntimeError::new_err("game object references a missing model"))?;
        Ok(f(&model.mesh))
    }
}

#[pymethods]
impl PyGameObject {
    #[getter]
    fn name(&self) -> PyResult<String> {
        let scene = scene_lock(&self.scene)?;
        let object = scene
            .object(self.id)
            .ok_or_else(|| PyRuntimeError::new_err("game object no longer exists"))?;
        Ok(object.name.clone())
    }

    #[getter]
    fn position(&self) -> PyResult<(f32, f32, f32)> {
        let scene = scene_lock(&self.scene)?;
        let object = scene
            .object(self.id)
            .ok_or_else(|| PyRuntimeError::new_err("game object no longer exists"))?;
        let [x, y, z] = object.transform.translation;
        Ok((x, y, z))
    }

    #[setter]
    fn set_position(&self, position: (f32, f32, f32)) -> PyResult<()> {
        let mut scene = scene_lock(&self.scene)?;
        let object = scene
            .object_mut(self.id)
            .ok_or_else(|| PyRuntimeError::new_err("game object no longer exists"))?;
        object.transform.translation = [position.0, position.1, position.2];
        Ok(())
    }

    #[getter]
    fn rotation(&self) -> PyResult<(f32, f32, f32, f32)> {
        let scene = scene_lock(&self.scene)?;
        let object = scene
            .object(self.id)
            .ok_or_else(|| PyRuntimeError::new_err("game object no longer exists"))?;
        let [x, y, z, w] = object.transform.rotation;
        Ok((x, y, z, w))
    }

    #[setter]
    fn set_rotation(&self, rotation: (f32, f32, f32, f32)) -> PyResult<()> {
        let mut scene = scene_lock(&self.scene)?;
        let object = scene
            .object_mut(self.id)
            .ok_or_else(|| PyRuntimeError::new_err("game object no longer exists"))?;
        object.transform.rotation = [rotation.0, rotation.1, rotation.2, rotation.3];
        Ok(())
    }

    #[getter]
    fn scale(&self) -> PyResult<(f32, f32, f32)> {
        let scene = scene_lock(&self.scene)?;
        let object = scene
            .object(self.id)
            .ok_or_else(|| PyRuntimeError::new_err("game object no longer exists"))?;
        let [x, y, z] = object.transform.scale;
        Ok((x, y, z))
    }

    #[setter]
    fn set_scale(&self, scale: (f32, f32, f32)) -> PyResult<()> {
        let mut scene = scene_lock(&self.scene)?;
        let object = scene
            .object_mut(self.id)
            .ok_or_else(|| PyRuntimeError::new_err("game object no longer exists"))?;
        object.transform.scale = [scale.0, scale.1, scale.2];
        Ok(())
    }

    fn set_parent(&self, parent: &PyGameObject) -> PyResult<()> {
        if !Arc::ptr_eq(&self.scene, &parent.scene) {
            return Err(PyValueError::new_err(
                "both game objects must belong to the same scene",
            ));
        }
        if !scene_lock(&self.scene)?.set_parent(self.id, Some(parent.id)) {
            return Err(PyValueError::new_err(
                "invalid parent; this would create a cycle",
            ));
        }
        Ok(())
    }

    #[pyo3(signature = (clip_index=0))]
    fn play(&self, clip_index: usize) -> PyResult<()> {
        if !scene_lock(&self.scene)?.play_animation(self.id, clip_index) {
            return Err(PyValueError::new_err(
                "game object has no animator or the animation clip index is invalid",
            ));
        }
        Ok(())
    }

    #[getter]
    fn source_meshes(&self) -> PyResult<usize> {
        self.with_model(|model| model.source_meshes)
    }

    #[getter]
    fn triangles(&self) -> PyResult<usize> {
        self.with_model(|model| model.triangles)
    }

    #[getter]
    fn vertex_count(&self) -> PyResult<usize> {
        self.with_model(|model| model.vertices.len())
    }

    #[getter]
    fn vertex_positions(&self) -> PyResult<Vec<(f32, f32, f32)>> {
        self.with_model(|model| {
            model
                .vertices
                .iter()
                .map(|vertex| (vertex.position[0], vertex.position[1], vertex.position[2]))
                .collect()
        })
    }

    #[getter]
    fn bounds_min(&self) -> PyResult<(f32, f32, f32)> {
        self.with_model(|model| {
            (
                model.bounds_min[0],
                model.bounds_min[1],
                model.bounds_min[2],
            )
        })
    }

    #[getter]
    fn bounds_max(&self) -> PyResult<(f32, f32, f32)> {
        self.with_model(|model| {
            (
                model.bounds_max[0],
                model.bounds_max[1],
                model.bounds_max[2],
            )
        })
    }

    #[getter]
    fn bone_count(&self) -> PyResult<usize> {
        self.with_model(|model| model.animation.bones.len())
    }

    #[getter]
    fn animation_clip_count(&self) -> PyResult<usize> {
        self.with_model(|model| model.animation.clips.len())
    }
}

#[pyclass(name = "Engine")]
pub struct PyEngine;

#[pymethods]
impl PyEngine {
    #[new]
    fn new() -> Self {
        Self
    }

    #[pyo3(signature = (scene, max_frames=None))]
    fn start(
        &self,
        py: Python<'_>,
        scene: PyRef<'_, PyScene>,
        max_frames: Option<u32>,
    ) -> PyResult<()> {
        let scene = Arc::clone(&scene.inner);
        py.detach(move || engine::run_scene(scene, max_frames))
            .map_err(PyRuntimeError::new_err)
    }

    #[pyo3(signature = (scene, frames=1))]
    fn render(&self, py: Python<'_>, scene: PyRef<'_, PyScene>, frames: u32) -> PyResult<()> {
        if frames == 0 {
            return Err(PyValueError::new_err("frames must be greater than zero"));
        }
        let scene = Arc::clone(&scene.inner);
        py.detach(move || engine::run_scene(scene, Some(frames)))
            .map_err(PyRuntimeError::new_err)
    }
}

#[pymodule]
fn yuriengine(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyEngine>()?;
    module.add_class::<PyScene>()?;
    module.add_class::<PyGameObject>()?;
    Ok(())
}
