use imgui::{Condition, TreeNodeFlags, Ui};

use crate::scene::{ObjectId, Scene};

pub fn build_hierarchy(ui: &Ui, scene: &mut Scene, selected_object: &mut Option<ObjectId>) {
    ui.window("Hierarchy")
        .position([10.0, 40.0], Condition::FirstUseEver)
        .size([300.0, 580.0], Condition::FirstUseEver)
        .build(|| {
            draw_nodes(ui, scene, None, selected_object);
            if ui.button("Add Empty GameObject") {
                let object_id = scene.create_game_object("GameObject");
                if let Some(parent) = *selected_object {
                    scene.set_parent(object_id, Some(parent));
                }
                *selected_object = Some(object_id);
            }
        });
}

fn draw_nodes(
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
            draw_nodes(ui, scene, Some(object_id), selected_object);
            token.pop();
        }
    }
}
