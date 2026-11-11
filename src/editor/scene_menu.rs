use std::path::PathBuf;

use imgui::Ui;

#[derive(Debug)]
pub enum SceneMenuAction {
    New,
    Open(PathBuf),
    Save(PathBuf),
}

pub struct SceneMenu {
    path: String,
    status: String,
}

impl SceneMenu {
    pub fn new(initial_path: Option<&PathBuf>) -> Self {
        Self {
            path: initial_path
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default(),
            status: String::new(),
        }
    }

    pub fn path(&self) -> Option<PathBuf> {
        let path = self.path.trim();
        (!path.is_empty()).then(|| PathBuf::from(path))
    }

    pub fn set_path(&mut self, path: &PathBuf) {
        self.path = path.to_string_lossy().into_owned();
    }

    pub fn clear_path(&mut self) {
        self.path.clear();
    }

    pub fn set_status(&mut self, status: impl Into<String>) {
        self.status = status.into();
    }

    pub fn draw(&mut self, ui: &Ui) -> Option<SceneMenuAction> {
        let mut action = None;
        ui.main_menu_bar(|| {
            ui.menu("Scene", || {
                ui.input_text("BSON path", &mut self.path).build();
                ui.separator();
                if ui.menu_item("New Scene") {
                    action = Some(SceneMenuAction::New);
                }
                let has_path = self.path().is_some();
                if ui.menu_item_config("Open Scene").enabled(has_path).build() {
                    action = self.path().map(SceneMenuAction::Open);
                }
                if ui.menu_item_config("Save Scene").enabled(has_path).build() {
                    action = self.path().map(SceneMenuAction::Save);
                }
                if !self.status.is_empty() {
                    ui.separator();
                    ui.text(&self.status);
                }
            });
        });
        action
    }
}
