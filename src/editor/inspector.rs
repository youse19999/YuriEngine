use imgui::{Condition, Ui};

use crate::scene::{Component, ObjectId, Scene};

pub struct ComponentJsonEdit {
    pub object_id: ObjectId,
    pub component_index: usize,
    pub json: String,
}

#[derive(Default)]
pub struct InspectorState {
    selected_component: Option<(ObjectId, usize)>,
    json_text: String,
    status: String,
}

impl InspectorState {
    pub fn reset(&mut self) {
        self.selected_component = None;
        self.json_text.clear();
        self.status.clear();
    }

    pub fn set_status(&mut self, status: impl Into<String>) {
        self.status = status.into();
    }
}

pub fn build_inspector(
    ui: &Ui,
    scene: &mut Scene,
    selected_object: &mut Option<ObjectId>,
    state: &mut InspectorState,
) -> Option<ComponentJsonEdit> {
    let Some(object_id) = *selected_object else {
        state.selected_component = None;
        ui.window("Inspector")
            .position([320.0, 40.0], Condition::FirstUseEver)
            .size([440.0, 580.0], Condition::FirstUseEver)
            .build(|| ui.text("Select an object in the hierarchy."));
        return None;
    };

    if state
        .selected_component
        .is_some_and(|(selected_id, _)| selected_id != object_id)
    {
        state.selected_component = None;
        state.json_text.clear();
    }

    let mut clicked_component = None;
    let mut edit = None;
    ui.window("Inspector")
        .position([320.0, 40.0], Condition::FirstUseEver)
        .size([440.0, 580.0], Condition::FirstUseEver)
        .build(|| {
            let mut object_exists = false;
            if let Some(object) = scene.object_mut(object_id) {
                object_exists = true;
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
                for (index, component) in object.components().iter().enumerate() {
                    let label = match component {
                        Component::Transform(_) => "Transform",
                        Component::MeshRenderer(_) => "MeshRenderer",
                        Component::Animator(_) => "Animator",
                    };
                    let id_label = format!("{label}##component_{index}");
                    if ui
                        .selectable_config(&id_label)
                        .selected(state.selected_component == Some((object_id, index)))
                        .build()
                    {
                        clicked_component = Some(index);
                    }
                }
            }

            if !object_exists {
                *selected_object = None;
                state.selected_component = None;
                state.json_text.clear();
                ui.text("No object selected");
                return;
            }

            if let Some(index) = clicked_component {
                state.selected_component = Some((object_id, index));
                match scene.component_json(object_id, index) {
                    Ok(json) => {
                        state.json_text = json;
                        state.status.clear();
                    }
                    Err(error) => state.status = error,
                }
            }

            ui.separator();
            if state
                .selected_component
                .is_some_and(|(selected_id, _)| selected_id == object_id)
            {
                ui.text("Component JSON");
                ui.input_text_multiline("##component_json", &mut state.json_text, [400.0, 250.0])
                    .build();
                if ui.button("Apply JSON") {
                    if let Some((selected_id, component_index)) = state.selected_component {
                        edit = Some(ComponentJsonEdit {
                            object_id: selected_id,
                            component_index,
                            json: state.json_text.clone(),
                        });
                    }
                }
            } else {
                ui.text("Click a component to edit its JSON.");
            }
            if !state.status.is_empty() {
                ui.text_wrapped(&state.status);
            }
        });
    edit
}
