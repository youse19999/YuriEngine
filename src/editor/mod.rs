mod hierarchy;
mod inspector;
pub mod scene_menu;

use imgui::{Condition, Ui};

pub use hierarchy::build_hierarchy;
pub use inspector::build_inspector;

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
