use super::number;
use crate::{style::*, workbench::Workbench};
use airflow::model::*;
use eframe::egui::*;
use uuid::Uuid;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Volume,
    FixedPressure,
    Airflow,
    Loss,
    FreeArea,
    Discharge,
    OpenFraction,
    Shutoff,
    Length,
    Roughness,
    Fittings,
    Diameter,
    Width,
    Height,
    Start,
    End,
}
impl Field {
    const ALL: [Self; 16] = [
        Self::Volume,
        Self::FixedPressure,
        Self::Airflow,
        Self::Loss,
        Self::FreeArea,
        Self::Discharge,
        Self::OpenFraction,
        Self::Shutoff,
        Self::Length,
        Self::Roughness,
        Self::Fittings,
        Self::Diameter,
        Self::Width,
        Self::Height,
        Self::Start,
        Self::End,
    ];
    fn label(
        self,
        flow_unit: &'static str,
        flow_scale: f64,
    ) -> (&'static str, &'static str, f64, f64) {
        match self {
            Self::Volume => ("Room volume", "m³", 1.0, 1.0),
            Self::FixedPressure => ("Fixed gauge", "Pa", 0.1, 1.0),
            Self::Airflow => ("Set airflow", flow_unit, 1.0, flow_scale),
            Self::Loss => ("Loss coefficient", "Pa·s²/m⁶", 10.0, 1.0),
            Self::FreeArea => ("Free area", "m²", 0.01, 1.0),
            Self::Discharge => ("Discharge coefficient", "", 0.01, 1.0),
            Self::OpenFraction => ("Open fraction", "", 0.01, 1.0),
            Self::Shutoff => ("Shutoff rise", "Pa", 0.5, 1.0),
            Self::Length => ("Length", "m", 0.1, 1.0),
            Self::Roughness => ("Roughness", "mm", 0.002, 1000.0),
            Self::Fittings => ("Fitting losses ΣK", "", 0.05, 1.0),
            Self::Diameter => ("Diameter", "m", 0.005, 1.0),
            Self::Width => ("Duct width", "m", 0.005, 1.0),
            Self::Height => ("Duct height", "m", 0.005, 1.0),
            Self::Start => ("Start elevation", "m", 0.1, 1.0),
            Self::End => ("End elevation", "m", 0.1, 1.0),
        }
    }
    fn get(self, net: &mut Network, id: Uuid) -> Option<&mut f64> {
        if matches!(self, Self::Volume | Self::FixedPressure) {
            let n = net.nodes.iter_mut().find(|n| n.id == id)?;
            return match (&mut n.kind, self) {
                (NodeKind::Boundary { gauge_pa }, Self::FixedPressure) => Some(gauge_pa),
                (NodeKind::Room { volume_m3, .. }, Self::Volume) => Some(volume_m3),
                _ => None,
            };
        }
        let b = net.branches.iter_mut().find(|b| b.id == id)?;
        match self {
            Self::Start => Some(&mut b.start_elevation_m),
            Self::End => Some(&mut b.end_elevation_m),
            Self::FreeArea => Some(&mut b.aperture.as_mut()?.area_m2),
            Self::Discharge => Some(&mut b.aperture.as_mut()?.discharge),
            Self::OpenFraction => Some(&mut b.aperture.as_mut()?.open_fraction),
            _ => match (&mut b.kind, self) {
                (BranchKind::FlowFan { design_m3_s, .. }, Self::Airflow) => Some(design_m3_s),
                (BranchKind::Opening { k }, Self::Loss) if b.aperture.is_none() => Some(k),
                (BranchKind::PressureFan { k, .. }, Self::Loss)
                    if b.custom_fan_curve.is_empty() =>
                {
                    Some(k)
                }
                (BranchKind::PressureFan { shutoff_pa, .. }, Self::Shutoff)
                    if b.custom_fan_curve.is_empty() =>
                {
                    Some(shutoff_pa)
                }
                (BranchKind::Duct { duct, .. }, field) => match field {
                    Self::Length => Some(&mut duct.length_m),
                    Self::Roughness => Some(&mut duct.roughness_m),
                    Self::Fittings => Some(&mut duct.minor_k),
                    _ => match (&mut duct.section, field) {
                        (Section::Round { diameter_m }, Self::Diameter) => Some(diameter_m),
                        (Section::Rectangular { width_m, .. }, Self::Width) => Some(width_m),
                        (Section::Rectangular { height_m, .. }, Self::Height) => Some(height_m),
                        _ => None,
                    },
                },
                _ => None,
            },
        }
    }
}

impl Workbench {
    pub(super) fn inspect_many(&mut self, ui: &mut Ui, p: Palette) {
        let ids: Vec<_> = self
            .selection
            .iter()
            .copied()
            .filter(|id| self.component_editable(*id))
            .collect();
        let ignored = self.selection.len() - ids.len();
        ui.label(tag(
            &format!(
                "{} SELECTED  /  {} EDITABLE",
                self.selection.len(),
                ids.len()
            ),
            p.accent,
        ));
        if ignored > 0 {
            ui.label(
                RichText::new(format!("{ignored} read-only selection(s) will be skipped."))
                    .color(p.warning),
            );
        }
        if ids.is_empty() {
            return;
        }
        ui.label(RichText::new("Shared edits apply to all editable selections.").color(p.muted));
        let (unit, scale) = self.flow_unit();
        let net = &mut self.project.network;
        let mut has_shared = false;
        if ids.iter().all(|id| net.branch(*id).is_some()) {
            has_shared = true;
            let first = net.branch(ids[0]).expect("selected branch").system;
            let mixed = ids
                .iter()
                .any(|id| net.branch(*id).is_some_and(|b| b.system != first));
            let mut picked = None;
            ComboBox::from_id_salt("bulk-system")
                .selected_text(if mixed {
                    "Mixed systems"
                } else {
                    first.label()
                })
                .show_ui(ui, |ui| {
                    for system in System::ALL {
                        if ui
                            .selectable_label(!mixed && system == first, system.label())
                            .clicked()
                        {
                            picked = Some(system);
                        }
                    }
                });
            if let Some(system) = picked {
                for b in &mut net.branches {
                    if ids.contains(&b.id) {
                        b.system = system;
                    }
                }
            }
        }
        for field in Field::ALL {
            let Some(values) = ids
                .iter()
                .map(|id| field.get(net, *id).map(|v| *v))
                .collect::<Option<Vec<_>>>()
            else {
                continue;
            };
            has_shared = true;
            let first = values[0];
            let mixed = values.iter().any(|v| *v != first);
            let (label, unit, speed, scale) = field.label(unit, scale);
            let mut value = first * scale;
            let changed = ui
                .push_id(field as usize, |ui| {
                    if mixed {
                        ui.horizontal(|ui| {
                            ui.label(label);
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if !unit.is_empty() {
                                    ui.label(RichText::new(unit).size(10.0));
                                }
                                let mut edit = ui
                                    .memory(|m| m.data.get_temp::<String>(ui.id().with("mixed")))
                                    .unwrap_or_default();
                                let response = ui.add(
                                    TextEdit::singleline(&mut edit)
                                        .hint_text("Mixed")
                                        .desired_width(57.0),
                                );
                                let submit = response.lost_focus()
                                    || (response.has_focus()
                                        && ui.input(|i| i.key_pressed(Key::Enter)));
                                if submit {
                                    ui.memory_mut(|m| {
                                        m.data.remove::<String>(ui.id().with("mixed"))
                                    });
                                    if let Ok(parsed) = edit.trim().parse::<f64>()
                                        && parsed.is_finite()
                                    {
                                        value = parsed;
                                        return true;
                                    }
                                } else if response.changed() {
                                    ui.memory_mut(|m| {
                                        m.data.insert_temp(ui.id().with("mixed"), edit)
                                    });
                                }
                                false
                            })
                            .inner
                        })
                        .inner
                    } else {
                        number(ui, label, &mut value, unit, speed);
                        value != first * scale
                    }
                })
                .inner;
            if changed {
                for id in &ids {
                    if let Some(v) = field.get(net, *id) {
                        *v = value / scale;
                    }
                }
            }
        }
        if !has_shared {
            ui.label(
                RichText::new("These selections have no shared editable physical properties.")
                    .color(p.muted),
            );
        }
    }
}
