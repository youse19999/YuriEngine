use imgui::{Condition, Ui};

use crate::scene::{Component, ObjectId, Scene};

pub fn build_inspector(ui: &Ui, scene: &mut Scene, selected_object: &mut Option<ObjectId>) {
    ui.window("Inspector")
        .position([320.0, 40.0], Condition::FirstUseEver)
        .size([340.0, 580.0], Condition::FirstUseEver)
        .build(|| {
            if let Some(object_id) = *selected_object {
                if let Some(object) = scene.object_mut(object_id) {
                    ui.text(format!("Name: {}", object.name));
                    {
                        let transform = object.transform_mut();
                        ui.input_float3("Position", &mut transform.translation)
                            .build();
                        ui.input_float4("Rotation (x, y, z, w)", &mut transform.rotation)
                            .build();
                        ui.input_float3("Scale", &mut transform.scale).build();
                    }
                    ui.separator();
                    ui.text("Components");
                    for component in object.components() {
                        match component {
                            Component::Transform(_) => ui.bullet_text("Transform"),
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
                ui.text("Select an object in the hierarchy.");
            }
        });
}
