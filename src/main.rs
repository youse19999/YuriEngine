use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Instant,
};

use imgui::{Condition, Context, TreeNodeFlags, Ui};
use imgui_wgpu::{Renderer as ImGuiRenderer, RendererConfig};
use imgui_winit_support::{HiDpiMode, WinitPlatform};
use wgpu::CurrentSurfaceTexture;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

mod capture;
mod fbx;
mod fbx_animation;
mod fbx_hierarchy;
mod fbx_materials;
mod fbx_skin;
mod mesh;
mod scene;
mod texture;
use mesh::MeshPipeline;
use scene::{Component, ObjectId, Scene};

const PINK_CLEAR: wgpu::Color = wgpu::Color {
    r: 1.0,
    g: 0.0,
    b: 0.55,
    a: 1.0,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrameAction {
    Render,
    Reconfigure,
    Skip,
    Fail,
}

fn classify_frame(result: &CurrentSurfaceTexture) -> FrameAction {
    match result {
        CurrentSurfaceTexture::Success(_) => FrameAction::Render,
        CurrentSurfaceTexture::Suboptimal(_)
        | CurrentSurfaceTexture::Outdated
        | CurrentSurfaceTexture::Lost => FrameAction::Reconfigure,
        CurrentSurfaceTexture::Timeout | CurrentSurfaceTexture::Occluded => FrameAction::Skip,
        CurrentSurfaceTexture::Validation => FrameAction::Fail,
    }
}

fn should_configure(current_width: u32, current_height: u32, width: u32, height: u32) -> bool {
    width > 0 && height > 0 && (current_width != width || current_height != height)
}

struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    _depth_texture: wgpu::Texture,
    depth_view: wgpu::TextureView,
    scene: Scene,
    mesh_pipelines: HashMap<ObjectId, MeshPipeline>,
    last_update: Instant,
    imgui: ImGuiRenderer,
    uncaptured_error: Arc<Mutex<Option<String>>>,
}

impl Renderer {
    async fn new(
        window: Arc<Window>,
        imgui_context: &mut Context,
        model_paths: &[std::path::PathBuf],
        animation_path: Option<&std::path::Path>,
    ) -> Result<Self, String> {
        let size = window.inner_size();
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(window)
            .map_err(|e| format!("create surface: {e}"))?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
            })
            .await
            .map_err(|e| format!("request adapter: {e}"))?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("YuriEngine device"),
                ..Default::default()
            })
            .await
            .map_err(|e| format!("request device: {e}"))?;
        let uncaptured_error = Arc::new(Mutex::new(None));
        let error_slot = Arc::clone(&uncaptured_error);
        device.on_uncaptured_error(Arc::new(move |error| {
            let message = match &error {
                wgpu::Error::OutOfMemory { .. } => {
                    format!("fatal GPU out-of-memory error: {error}")
                }
                wgpu::Error::Validation { .. } => format!("fatal GPU validation error: {error}"),
                wgpu::Error::Internal { .. } => format!("fatal internal GPU error: {error}"),
            };
            if let Ok(mut slot) = error_slot.lock() {
                *slot = Some(message);
            }
        }));

        let capabilities = surface.get_capabilities(&adapter);
        let format = capabilities
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .or_else(|| capabilities.formats.first().copied())
            .ok_or_else(|| "surface exposes no formats".to_owned())?;
        let present_mode = if capabilities
            .present_modes
            .contains(&wgpu::PresentMode::Fifo)
        {
            wgpu::PresentMode::Fifo
        } else {
            *capabilities
                .present_modes
                .first()
                .ok_or_else(|| "surface exposes no present modes".to_owned())?
        };
        let alpha_mode = *capabilities
            .alpha_modes
            .first()
            .ok_or_else(|| "surface exposes no alpha modes".to_owned())?;
        let config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or_else(|| "surface has no default configuration".to_owned())?;
        let config = wgpu::SurfaceConfiguration {
            format,
            present_mode,
            alpha_mode,
            ..config
        };
        surface.configure(&device, &config);
        let (depth_texture, depth_view) = mesh::create_depth_texture(
            &device,
            config.width,
            config.height,
            "YuriEngine window depth",
        );
        let mut scene = Scene::new();
        let root_id = scene.create_game_object("Scene Root");
        let mut mesh_pipelines = HashMap::new();
        let source_animation = animation_path
            .map(|path| fbx_animation::load(path, 30.0).map_err(|error| error.to_string()))
            .transpose()?;
        for model_path in model_paths {
            let mut model = fbx::load(model_path).map_err(|error| error.to_string())?;
            let animation = if let Some(source) = &source_animation {
                let (animation, matched_bones) = fbx_animation::retarget(source, &model.animation);
                eprintln!("Retargeted animation onto {} rig: {} mapped bones / {} target bones / {} clips", model_path.display(), matched_bones, model.animation.bones.len(), animation.clips.len());
                animation
            } else {
                model.animation.clone()
            };
            model.animation = animation;
            eprintln!(
                "Loaded {} meshes / {} triangles / {} material bindings / {} scene nodes / {} bones / {} animation clips from {}",
                model.source_meshes,
                model.triangles,
                model.materials.len(),
                model.hierarchy.nodes.len(),
                model.animation.bones.len(),
                model.animation.clips.len(),
                model_path.display()
            );
            let model_id = scene.add_model(model);
            let object_name = model_path
                .file_stem()
                .and_then(std::ffi::OsStr::to_str)
                .unwrap_or("Model");
            let object_id = scene.spawn_model(object_name, model_id).ok_or_else(|| {
                format!("failed to create game object for {}", model_path.display())
            })?;
            if !scene.set_parent(object_id, Some(root_id)) {
                return Err(format!(
                    "failed to parent {} to scene root",
                    model_path.display()
                ));
            }
            eprintln!(
                "Created game object '{}' under '{}'",
                scene.object(object_id).unwrap().name,
                scene.object(root_id).unwrap().name
            );
            let model_asset = scene
                .model(model_id)
                .ok_or_else(|| "loaded model asset disappeared".to_owned())?;
            let mesh_pipeline =
                MeshPipeline::new(&device, &queue, config.format, &model_asset.mesh);
            mesh_pipelines.insert(object_id, mesh_pipeline);
        }
        if mesh_pipelines.is_empty() {
            return Err("scene has no model game objects".into());
        }
        let imgui = ImGuiRenderer::new(
            imgui_context,
            &device,
            &queue,
            RendererConfig {
                texture_format: config.format,
                ..Default::default()
            },
        );

        Ok(Self {
            surface,
            device,
            queue,
            config,
            _depth_texture: depth_texture,
            depth_view,
            scene,
            mesh_pipelines,
            last_update: Instant::now(),
            imgui,
            uncaptured_error,
        })
    }

    fn resize(&mut self, width: u32, height: u32) {
        if !should_configure(self.config.width, self.config.height, width, height) {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        let (depth_texture, depth_view) =
            mesh::create_depth_texture(&self.device, width, height, "YuriEngine window depth");
        self._depth_texture = depth_texture;
        self.depth_view = depth_view;
    }

    fn render(&mut self, draw_data: &imgui::DrawData) -> Result<bool, String> {
        if let Some(error) = self
            .uncaptured_error
            .lock()
            .map_err(|_| "GPU error state lock poisoned".to_owned())?
            .take()
        {
            return Err(error);
        }
        let frame = self.surface.get_current_texture();
        match classify_frame(&frame) {
            FrameAction::Reconfigure => {
                self.surface.configure(&self.device, &self.config);
                return Ok(false);
            }
            FrameAction::Skip => return Ok(false),
            FrameAction::Fail => return Err("surface returned a validation error".into()),
            FrameAction::Render => {}
        }
        let texture = match frame {
            CurrentSurfaceTexture::Success(texture) => texture,
            CurrentSurfaceTexture::Suboptimal(texture) => texture,
            _ => unreachable!("frame classification and texture result diverged"),
        };
        let view = texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("FBX frame"),
            });
        let delta_seconds = self.last_update.elapsed().as_secs_f32();
        self.last_update = Instant::now();
        self.scene.update(delta_seconds);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("FBX mesh pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(PINK_CLEAR),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            for object in self.scene.objects() {
                for component in object.components() {
                    let Component::MeshRenderer(mesh_renderer) = component else {
                        continue;
                    };
                    let Some(model) = self.scene.model(mesh_renderer.model) else {
                        continue;
                    };
                    let Some(pipeline) = self.mesh_pipelines.get(&object.id) else {
                        continue;
                    };
                    let Some(world_transform) = self.scene.world_matrix(object.id) else {
                        continue;
                    };
                    let frame = object
                        .animator()
                        .and_then(|animator| animator.sample(&model.mesh.animation));
                    let palette = fbx_animation::skin_palette(&model.mesh.animation, frame);
                    let bounds = (model.mesh.bounds_min, model.mesh.bounds_max);
                    pipeline.draw(
                        &self.queue,
                        &mut pass,
                        bounds,
                        self.config.width,
                        self.config.height,
                        Some(&palette),
                        world_transform,
                    );
                }
            }
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ImGui render pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            self.imgui
                .render(draw_data, &self.queue, &self.device, &mut pass)
                .map_err(|error| format!("render ImGui: {error}"))?;
        }
        self.queue.submit([encoder.finish()]);
        texture.present();
        Ok(true)
    }
}

struct App {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    imgui: Context,
    imgui_platform: WinitPlatform,
    smoke_frames: Option<u32>,
    presented_frames: u32,
    selected_object: Option<ObjectId>,
    model_paths: Vec<std::path::PathBuf>,
    animation_path: Option<std::path::PathBuf>,
}

impl App {
    fn new(
        smoke_frames: Option<u32>,
        model_paths: Vec<std::path::PathBuf>,
        animation_path: Option<std::path::PathBuf>,
    ) -> Self {
        let mut imgui = Context::create();
        imgui.set_ini_filename(None);
        let imgui_platform = WinitPlatform::new(&mut imgui);
        Self {
            window: None,
            renderer: None,
            smoke_frames,
            imgui,
            imgui_platform,
            presented_frames: 0,
            selected_object: None,
            model_paths,
            animation_path,
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("YuriEngine SwapChain")
            .with_inner_size(winit::dpi::LogicalSize::new(800.0, 600.0));
        match event_loop.create_window(attributes) {
            Ok(window) => {
                let window = Arc::new(window);
                self.imgui_platform
                    .attach_window(self.imgui.io_mut(), &window, HiDpiMode::Default);
                match pollster::block_on(Renderer::new(
                    window.clone(),
                    &mut self.imgui,
                    &self.model_paths,
                    self.animation_path.as_deref(),
                )) {
                    Ok(renderer) => {
                        self.renderer = Some(renderer);
                        self.window = Some(window);
                    }
                    Err(error) => {
                        eprintln!("Renderer initialization failed: {error}");
                        event_loop.exit();
                    }
                }
            }
            Err(error) => {
                eprintln!("Failed to create window: {error}");
                event_loop.exit();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(window) = self
            .window
            .as_ref()
            .filter(|window| window.id() == window_id)
        else {
            return;
        };
        self.imgui_platform.handle_event::<()>(
            self.imgui.io_mut(),
            window,
            &winit::event::Event::WindowEvent {
                window_id,
                event: event.clone(),
            },
        );
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.resize(size.width, size.height);
                }
            }
            WindowEvent::RedrawRequested => {
                if self
                    .imgui_platform
                    .prepare_frame(self.imgui.io_mut(), window)
                    .is_err()
                {
                    eprintln!("Failed to prepare ImGui frame");
                    event_loop.exit();
                    return;
                }
                let ui = self.imgui.frame();
                if let Some(renderer) = self.renderer.as_mut() {
                    build_scene_ui(ui, &mut renderer.scene, &mut self.selected_object);
                }
                self.imgui_platform.prepare_render(ui, window);
                let draw_data = self.imgui.render();
                if let Some(renderer) = self.renderer.as_mut() {
                    match renderer.render(draw_data) {
                        Ok(true) => {
                            self.presented_frames += 1;
                            if self
                                .smoke_frames
                                .is_some_and(|limit| self.presented_frames >= limit)
                            {
                                event_loop.exit();
                            }
                        }
                        Ok(false) => {}
                        Err(error) => {
                            eprintln!("Rendering failed: {error}");
                            event_loop.exit();
                        }
                    }
                }
            }
            _ => {}
        }
        window.request_redraw();
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if let Some(path) = args.iter().find_map(|arg| {
        arg.strip_prefix("--inspect-animation=")
            .map(std::path::PathBuf::from)
    }) {
        let animation = fbx_animation::load(&path, 30.0)?;
        println!(
            "Animation import: {} bones / {} skinned meshes / {} clips",
            animation.bones.len(),
            animation.skinned_meshes.len(),
            animation.clips.len()
        );
        for bone in &animation.bones {
            println!(
                "bone {} id={} name={} parent={:?} rest_t={:?}",
                bone.index,
                bone.source_node_id,
                bone.name,
                bone.parent,
                bone.bind_local.translation
            );
        }
        for mesh in &animation.skinned_meshes {
            let weighted_vertices = mesh
                .vertex_influences
                .iter()
                .filter(|influences| !influences.is_empty())
                .count();
            let max_influences = mesh
                .vertex_influences
                .iter()
                .map(Vec::len)
                .max()
                .unwrap_or(0);
            let max_weight_sum_error = mesh
                .vertex_influences
                .iter()
                .filter(|influences| !influences.is_empty())
                .map(|influences| {
                    (influences
                        .iter()
                        .map(|influence| influence.weight)
                        .sum::<f32>()
                        - 1.0)
                        .abs()
                })
                .fold(0.0_f32, f32::max);
            println!(
                "skinned mesh id={} name={} control_points={} weighted={} max_influences={} max_weight_sum_error={:.6}",
                mesh.source_node_id,
                mesh.name,
                mesh.vertex_influences.len(),
                weighted_vertices,
                max_influences,
                max_weight_sum_error,
            );
        }
        for clip in &animation.clips {
            let changed_bones = clip
                .frames
                .first()
                .zip(clip.frames.last())
                .map(|(first, last)| {
                    first
                        .bone_transforms
                        .iter()
                        .zip(&last.bone_transforms)
                        .filter(|(a, b)| {
                            (0..3).any(|axis| {
                                (a.translation[axis] - b.translation[axis]).abs() > 1e-4
                                    || (a.scale[axis] - b.scale[axis]).abs() > 1e-4
                            }) || (0..4)
                                .any(|axis| (a.rotation[axis] - b.rotation[axis]).abs() > 1e-4)
                        })
                        .count()
                })
                .unwrap_or(0);
            println!(
                "clip {}: {:.3}s..{:.3}s, {} samples, {} bones changed end-to-end",
                clip.name,
                clip.start_seconds,
                clip.end_seconds,
                clip.frames.len(),
                changed_bones
            );
            if let Some(first) = clip.frames.first() {
                println!("  first sample time: {:.3}s", first.time_seconds);
            }
        }
        if let Some(reference_path) = args
            .iter()
            .find_map(|arg| arg.strip_prefix("--model=").map(std::path::PathBuf::from))
        {
            let reference = fbx_animation::load(&reference_path, 30.0)?;
            let (_, semantic_matches) = fbx_animation::retarget(&animation, &reference);
            let names: std::collections::HashSet<_> = reference
                .bones
                .iter()
                .map(|bone| bone.name.as_str())
                .collect();
            let matching = animation
                .bones
                .iter()
                .filter(|bone| names.contains(bone.name.as_str()))
                .count();
            println!(
                "reference rig {}: {} bones, {} exact name matches / {} semantic retarget matches out of {} animation bones",
                reference_path.display(),
                reference.bones.len(),
                matching,
                semantic_matches,
                animation.bones.len()
            );
        }
        return Ok(());
    }
    let model_paths = args
        .iter()
        .filter_map(|arg| arg.strip_prefix("--model=").map(std::path::PathBuf::from))
        .collect::<Vec<_>>();
    let animation_path = args.iter().find_map(|arg| {
        arg.strip_prefix("--animation=")
            .map(std::path::PathBuf::from)
    });
    if let Some(path) = args
        .iter()
        .find_map(|arg| arg.strip_prefix("--capture-frame=").map(str::to_owned))
    {
        let Some(model_path) = model_paths.first() else {
            return Err("--capture-frame requires at least one --model=<path> argument".into());
        };
        return pollster::block_on(capture::render_to_png(
            std::path::Path::new(&path),
            model_path,
        ));
    }

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let smoke_frames = args
        .iter()
        .find_map(|arg| arg.strip_prefix("--smoke-frames=")?.parse::<u32>().ok());
    event_loop.run_app(&mut App::new(smoke_frames, model_paths, animation_path))?;
    Ok(())
}

fn draw_hierarchy_nodes(
    ui: &Ui,
    scene: &Scene,
    parent: Option<ObjectId>,
    selected_object: &mut Option<ObjectId>,
) {
    let children = scene
        .objects()
        .iter()
        .filter(|object| object.parent() == parent)
        .map(|object| {
            let has_children = scene
                .objects()
                .iter()
                .any(|candidate| candidate.parent() == Some(object.id));
            (object.id, object.name.clone(), has_children)
        })
        .collect::<Vec<_>>();

    for (object_id, name, has_children) in children {
        let label = format!("{name}##{object_id:?}");
        let node = ui
            .tree_node_config(label.as_str())
            .selected(*selected_object == Some(object_id))
            .flags(TreeNodeFlags::SPAN_AVAIL_WIDTH)
            .leaf(!has_children)
            .tree_push_on_open(has_children);
        let open = node.push();
        if ui.is_item_clicked() {
            *selected_object = Some(object_id);
        }
        if let Some(token) = open {
            draw_hierarchy_nodes(ui, scene, Some(object_id), selected_object);
            token.pop();
        }
    }
}

fn build_scene_ui(ui: &Ui, scene: &mut Scene, selected_object: &mut Option<ObjectId>) {
    ui.window("Scene")
        .size([340.0, 580.0], Condition::FirstUseEver)
        .build(|| {
            ui.text("Hierarchy");
            ui.separator();
            draw_hierarchy_nodes(ui, scene, None, selected_object);

            if ui.button("Add Empty GameObject") {
                let object_id = scene.create_game_object("GameObject");
                if let Some(parent) = *selected_object {
                    scene.set_parent(object_id, Some(parent));
                }
                *selected_object = Some(object_id);
            }

            ui.separator();
            ui.text("Inspector");
            if let Some(object_id) = *selected_object {
                if let Some(object) = scene.object_mut(object_id) {
                    ui.text(format!("Name: {}", object.name));
                    ui.input_float3("Position", &mut object.transform.translation)
                        .build();
                    ui.input_float4("Rotation (x, y, z, w)", &mut object.transform.rotation)
                        .build();
                    ui.input_float3("Scale", &mut object.transform.scale)
                        .build();
                    ui.separator();
                    ui.text("Components");
                    for component in object.components() {
                        match component {
                            Component::MeshRenderer(_) => ui.bullet_text("MeshRenderer"),
                            Component::Animator(animator) => {
                                ui.bullet_text(format!("Animator (clip {})", animator.clip_index))
                            }
                        }
                    }
                } else {
                    *selected_object = None;
                    ui.text("No object selected");
                }
            } else {
                ui.text("Select an object to edit its transform.");
            }
        });
}

pub(crate) fn build_sample_ui(ui: &Ui) {
    ui.window("YuriEngine ImGui")
        .size([300.0, 120.0], Condition::FirstUseEver)
        .build(|| {
            ui.text("Dear ImGui is running inside YuriEngine.");
            ui.separator();
            ui.text("Renderer: wgpu");
            ui.text("Platform: winit");
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pink_clear_is_opaque_and_pink() {
        assert_eq!(
            PINK_CLEAR,
            wgpu::Color {
                r: 1.0,
                g: 0.0,
                b: 0.55,
                a: 1.0
            }
        );
    }

    #[test]
    fn surface_results_have_explicit_retry_policy() {
        assert_eq!(
            classify_frame(&CurrentSurfaceTexture::Timeout),
            FrameAction::Skip
        );
        assert_eq!(
            classify_frame(&CurrentSurfaceTexture::Occluded),
            FrameAction::Skip
        );
        assert_eq!(
            classify_frame(&CurrentSurfaceTexture::Outdated),
            FrameAction::Reconfigure
        );
        assert_eq!(
            classify_frame(&CurrentSurfaceTexture::Lost),
            FrameAction::Reconfigure
        );
        assert_eq!(
            classify_frame(&CurrentSurfaceTexture::Validation),
            FrameAction::Fail
        );
    }

    #[test]
    fn resize_skips_zero_and_unchanged_sizes() {
        assert!(!should_configure(800, 600, 0, 600));
        assert!(!should_configure(800, 600, 800, 0));
        assert!(!should_configure(800, 600, 800, 600));
        assert!(should_configure(800, 600, 1024, 768));
    }
}
