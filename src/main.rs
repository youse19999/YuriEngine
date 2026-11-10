use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

#[derive(Default)]
struct App {
    window: Option<Window>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            let attributes = Window::default_attributes()
                .with_title("YuriEngine Inspection/001")
                .with_inner_size(winit::dpi::LogicalSize::new(800.0, 600.0));
            match event_loop.create_window(attributes) {
                Ok(window) => self.window = Some(window),
                Err(error) => {
                    eprintln!("Failed to create window: {error}");
                    event_loop.exit();
                }
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if event == WindowEvent::CloseRequested
            && self.window.as_ref().is_some_and(|window| window.id() == window_id)
        {
            event_loop.exit();
        }
    }
}

fn main() -> Result<(), winit::error::EventLoopError> {
    let event_loop = EventLoop::new()?;
    event_loop.run_app(&mut App::default())
}
