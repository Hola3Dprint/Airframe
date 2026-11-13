use crate::{
    component_help,
    inspector::number,
    style::*,
    workbench::{LibraryDrag, View, Workbench, primary_press_in},
};
use airflow::model::NodeKind;
use eframe::egui::*;
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};
use uuid::Uuid;

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct NodePreset {
    pub name: String,
    pub kind: NodeKind,
    pub size: Option<[f32; 2]>,
}

fn library_path() -> PathBuf {
    crate::local_data_folder().join("library/node-library.json")
}

pub(crate) fn load() -> Vec<NodePreset> {
    fs::read(library_path())
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

impl Workbench {
    pub(crate) fn custom_library(&mut self, ui: &mut Ui, p: Palette, query: &str) {
        ui.add_space(5.0);
        ui.horizontal(|ui| {
            eyebrow(ui, "CUSTOM NODES", p);
            if ui.small_button("+ New").clicked() {
                self.preset_draft = Some(NodePreset {
                    name: String::new(),
                    kind: NodeKind::Room {
                        volume_m3: 100.0,
                        initial_pa: 0.0,
                    },
                    size: None,
                });
            }
        });
        let mut delete = None;
        for (i, preset) in self
            .presets
            .iter()
            .enumerate()
            .filter(|(_, preset)| preset.name.to_lowercase().contains(query))
        {
            let response = ui.add(
                Button::new(&preset.name)
                    .min_size(vec2(ui.available_width(), 27.0))
                    .sense(Sense::click_and_drag()),
            );
            if let Some(origin) = primary_press_in(ui, response.rect) {
                self.library_press = Some((LibraryDrag::Preset(preset.clone()), origin));
            }
            if response.clicked() {
                self.view = View::Building;
                self.draw_container = false;
                self.container_start = None;
                self.tool = None;
                self.from = None;
                self.reconnect = None;
                self.port_drag = None;
                self.placing_preset = Some(preset.clone());
            }
            response.clone().on_hover_ui(|ui| {
                component_help::tooltip(
                    ui,
                    &preset.name,
                    component_help::node_summary(preset.kind),
                    "Drag onto the canvas to place a copy of this template.",
                )
            });
            response.context_menu(|ui| {
                if ui.button("Edit template…").clicked() {
                    self.preset_draft = Some(preset.clone());
                    ui.close();
                }
                if ui.button("Remove from library").clicked() {
                    delete = Some(i);
                    ui.close();
                }
            });
        }
        if let Some(i) = delete {
            let mut library = self.presets.clone();
            library.remove(i);
            self.write_presets(library);
        }
    }

    pub(crate) fn preset_from(&mut self, id: Uuid) {
        if let Some(node) = self.project.network.node(id) {
            self.preset_draft = Some(NodePreset {
                name: node.name.clone(),
                kind: node.kind,
                size: self.project.drawing.sizes.get(&id).copied(),
            });
        }
    }

    pub(crate) fn preset_window(&mut self, ctx: &Context) {
        let Some(mut preset) = self.preset_draft.take() else {
            return;
        };
        let mut close = false;
        let mut save = false;
        Window::new("Custom node")
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label("Library name");
                ui.text_edit_singleline(&mut preset.name);
                ComboBox::from_id_salt("custom-kind")
                    .selected_text(preset.kind.label())
                    .show_ui(ui, |ui| {
                        for kind in [
                            NodeKind::Room {
                                volume_m3: 100.0,
                                initial_pa: 0.0,
                            },
                            NodeKind::Boundary { gauge_pa: 0.0 },
                            NodeKind::Atmosphere,
                            NodeKind::Junction,
                        ] {
                            if ui
                                .selectable_label(preset.kind.label() == kind.label(), kind.label())
                                .clicked()
                                && preset.kind.label() != kind.label()
                            {
                                preset.kind = kind;
                            }
                        }
                    });
                match &mut preset.kind {
                    NodeKind::Room {
                        volume_m3,
                        initial_pa,
                    } => {
                        number(ui, "Volume", volume_m3, "m³", 1.0);
                        number(ui, "Initial pressure", initial_pa, "Pa", 0.1);
                    }
                    NodeKind::Boundary { gauge_pa } => {
                        number(ui, "Fixed pressure", gauge_pa, "Pa", 0.1)
                    }
                    NodeKind::Atmosphere => {
                        ui.label("Outdoor reference at 0 Pa gauge.");
                    }
                    NodeKind::Junction => {
                        ui.label("A pressure point with no air storage.");
                    }
                }
                let valid = !preset.name.trim().is_empty()
                    && match preset.kind {
                        NodeKind::Room {
                            volume_m3,
                            initial_pa,
                        } => volume_m3.is_finite() && volume_m3 > 0.0 && initial_pa.is_finite(),
                        NodeKind::Boundary { gauge_pa } => gauge_pa.is_finite(),
                        NodeKind::Junction | NodeKind::Atmosphere => true,
                    };
                let replacing = self
                    .presets
                    .iter()
                    .any(|item| item.name.eq_ignore_ascii_case(preset.name.trim()));
                ui.horizontal(|ui| {
                    save = ui
                        .add_enabled(
                            valid,
                            Button::new(if replacing {
                                "Update template"
                            } else {
                                "Save template"
                            }),
                        )
                        .clicked();
                    close = ui.button("Cancel").clicked();
                });
            });
        if save {
            preset.name = preset.name.trim().to_owned();
            let mut library = self.presets.clone();
            if let Some(existing) = library
                .iter_mut()
                .find(|item| item.name.eq_ignore_ascii_case(&preset.name))
            {
                *existing = preset.clone();
            } else {
                library.push(preset.clone());
            }
            close = self.write_presets(library);
        }
        if !close {
            self.preset_draft = Some(preset);
        }
    }

    fn write_presets(&mut self, mut library: Vec<NodePreset>) -> bool {
        library.sort_by_cached_key(|item| item.name.to_lowercase());
        let file = library_path();
        let result = (|| -> Result<(), Box<dyn std::error::Error>> {
            let data = serde_json::to_vec_pretty(&library)?;
            fs::create_dir_all(file.parent().expect("library folder"))?;
            fs::write(file, data)?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.presets = library;
                self.notice("Custom node library saved", false);
                true
            }
            Err(error) => {
                self.notice(format!("Could not save node library: {error}"), true);
                false
            }
        }
    }

    pub(crate) fn place_preset(&mut self, preset: &NodePreset, xy: [f32; 2]) {
        if !self.floor_editable(self.active_floor) {
            self.notice("Choose a visible, unlocked floor.", true);
            return;
        }
        let previous = self.project.clone();
        let id = self
            .project
            .add_node(&preset.name, self.active_floor, preset.kind, xy);
        if let Some(size) = preset.size {
            self.project.drawing.sizes.insert(id, size);
        }
        self.selection = vec![id];
        self.placing_preset = None;
        self.checkpoint(previous);
    }
}
