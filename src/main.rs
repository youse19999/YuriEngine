use std::sync::{Arc, Mutex};

use wgpu::CurrentSurfaceTexture;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

mod triangle;
use triangle::TrianglePipeline;

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
    triangle: TrianglePipeline,
    uncaptured_error: Arc<Mutex<Option<String>>>,
}

impl Renderer {
    async fn new(window: Arc<Window>) -> Result<Self, String> {
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
                apply_limit_buckets: false,
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
        let triangle = TrianglePipeline::new(&device, config.format);

        Ok(Self {
            surface,
            device,
            queue,
            config,
            triangle,
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

    fn render(&mut self) -> Result<bool, String> {
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
                label: Some("triangle frame"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("triangle render pass"),
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
            self.triangle.draw(&mut pass);
        }
        self.queue.submit([encoder.finish()]);
        self.queue.present(texture);
        Ok(true)
    }
}

#[derive(Default)]
struct App {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    smoke_frames: Option<u32>,
    presented_frames: u32,
}

impl App {
    fn new(smoke_frames: Option<u32>) -> Self {
        Self {
            smoke_frames,
            ..Self::default()
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
                match pollster::block_on(Renderer::new(window.clone())) {
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
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.resize(size.width, size.height);
                }
            }
            WindowEvent::RedrawRequested => {
                if let Some(renderer) = self.renderer.as_mut() {
                    match renderer.render() {
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

fn main() -> Result<(), winit::error::EventLoopError> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let smoke_frames = std::env::args()
        .skip(1)
        .find_map(|arg| arg.strip_prefix("--smoke-frames=")?.parse::<u32>().ok());
    event_loop.run_app(&mut App::new(smoke_frames))
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
