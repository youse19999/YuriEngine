use std::sync::{Arc, Mutex};

use imgui::{Condition, Context, Ui};
use imgui_wgpu::{Renderer as ImGuiRenderer, RendererConfig};
use imgui_winit_support::{HiDpiMode, WinitPlatform};
use wgpu::CurrentSurfaceTexture;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

mod capture;
mod fbx;
mod mesh;
use mesh::MeshPipeline;

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
    mesh: MeshPipeline,
    mesh_bounds: ([f32; 3], [f32; 3]),
    imgui: ImGuiRenderer,
    uncaptured_error: Arc<Mutex<Option<String>>>,
}

impl Renderer {
    async fn new(
        window: Arc<Window>,
        imgui_context: &mut Context,
        model_path: &std::path::Path,
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
        let model = fbx::load(model_path).map_err(|error| error.to_string())?;
        eprintln!(
            "Loaded {} meshes / {} triangles from {}",
            model.source_meshes,
            model.triangles,
            model_path.display()
        );
        let mesh_bounds = (model.bounds_min, model.bounds_max);
        let mesh = MeshPipeline::new(&device, config.format, &model);
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
            mesh,
            mesh_bounds,
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
                ..Default::default()
            });
            self.mesh.draw(
                &self.queue,
                &mut pass,
                self.mesh_bounds,
                self.config.width,
                self.config.height,
            );
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
    model_path: std::path::PathBuf,
}

impl App {
    fn new(smoke_frames: Option<u32>, model_path: std::path::PathBuf) -> Self {
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
            model_path,
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
                    &self.model_path,
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
                build_sample_ui(ui);
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
    let model_path = args
        .iter()
        .find_map(|arg| arg.strip_prefix("--model=").map(std::path::PathBuf::from))
        .unwrap_or_else(|| {
            std::path::PathBuf::from(r"G:\Tsugu\sample\MUMUS_Avatar\fbx\Mumus_fbx.fbx")
        });
    if let Some(path) = args
        .iter()
        .find_map(|arg| arg.strip_prefix("--capture-frame=").map(str::to_owned))
    {
        return pollster::block_on(capture::render_to_png(
            std::path::Path::new(&path),
            &model_path,
        ));
    }

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let smoke_frames = args
        .iter()
        .find_map(|arg| arg.strip_prefix("--smoke-frames=")?.parse::<u32>().ok());
    event_loop.run_app(&mut App::new(smoke_frames, model_path))?;
    Ok(())
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
