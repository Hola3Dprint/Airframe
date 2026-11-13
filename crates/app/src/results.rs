use crate::{
    inspector::fan,
    style::*,
    workbench::{View, Workbench},
};
use airflow::{
    model::{BranchKind, M3_H, NodeKind},
    storage,
};
use eframe::egui::*;

impl Workbench {
    pub fn dock(&mut self, ctx: &Context, p: Palette) {
        TopBottomPanel::bottom("flow-dock")
            .exact_height(145.0)
            .frame(panel(p, 11))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    eyebrow(ui, "04  /  OPERATING POINT", p);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("Flow report").clicked() {
                            self.view = View::Results;
                            self.tool = None;
                            self.from = None;
                            self.reconnect = None;
                        }
                    });
                });
                self.startup_controls(ui, p);
                if self.startup.visible {
                    return;
                }
                self.airflow_slider(ui, p);
                let (unit, scale) = self.flow_unit();
                if let Some(run) = self.current_run() {
                    let room = self
                        .picked()
                        .and_then(|id| run.snapshot.node(id))
                        .or_else(|| {
                            run.snapshot
                                .nodes
                                .iter()
                                .find(|n| matches!(n.kind, NodeKind::Room { .. }))
                        });
                    if let Some(node) = room {
                        let balance = run.balance(node.id).unwrap_or_default();
                        let pressure = self.pressure(node.id).unwrap_or_default();
                        ui.horizontal_wrapped(|ui| {
                            ui.strong(&node.name);
                            ui.label(tag(&format!("{pressure:+.2} Pa"), p.accent));
                            ui.separator();
                            ui.label(format!("In {:.2} {unit}", balance.inflow * scale));
                            ui.label(format!("Out {:.2} {unit}", balance.outflow * scale));
                            ui.label(tag(
                                &format!("Net {:+.2} {unit}", balance.net() * scale),
                                p.good,
                            ));
                        });
                    } else {
                        ui.label("Network solved. Open the flow report to see branch results.");
                    }
                } else {
                    self.empty_results(ui, p);
                }
            });
    }

    fn airflow_slider(&mut self, ui: &mut Ui, p: Palette) {
        let (unit, scale) = self.flow_unit();
        let fans: Vec<_> = self
            .project
            .network
            .branches
            .iter()
            .filter(|b| b.kind.tau().is_some() && self.component_editable(b.id))
            .map(|b| (b.id, b.name.clone()))
            .collect();
        let selected = self
            .picked()
            .filter(|id| fans.iter().any(|(fan, _)| fan == id));
        let Some(mut id) = selected.or_else(|| fans.first().map(|(id, _)| *id)) else {
            ui.label(
                RichText::new("Add a fan on this floor to adjust its airflow.").color(p.muted),
            );
            return;
        };
        let before = self.project.clone();
        ui.horizontal(|ui| {
            let name = fans
                .iter()
                .find(|(fan, _)| *fan == id)
                .map_or("Fan", |(_, name)| name);
            ComboBox::from_id_salt("airflow-slider-fan")
                .selected_text(name)
                .show_ui(ui, |ui| {
                    for (fan, name) in &fans {
                        if ui.selectable_value(&mut id, *fan, name).clicked() {
                            self.selection = vec![*fan];
                        }
                    }
                });
            let solved_flow = self.flow(id);
            let branch = self
                .project
                .network
                .branches
                .iter_mut()
                .find(|b| b.id == id)
                .expect("editable fan");
            self.solve_requested |= fan::mode_button(ui, branch, solved_flow);
        });
        let solved_flow = self.flow(id);
        let branch = self
            .project
            .network
            .branches
            .iter_mut()
            .find(|b| b.id == id)
            .expect("editable fan");
        let curve = matches!(branch.kind, BranchKind::PressureFan { .. });
        let (mut flow, max) = match branch.kind {
            BranchKind::PressureFan { .. } => {
                let rated = branch.fan_rated_flow().unwrap_or_default() * scale;
                (branch.fan_speed_fraction.unwrap_or(1.0) * rated, rated)
            }
            BranchKind::FlowFan { design_m3_s, .. } => {
                let flow = design_m3_s * scale;
                let limit = ui.ctx().data_mut(|data| {
                    let limit_id = Id::new(("fixed-airflow-limit", id));
                    let limit = data
                        .get_temp::<f64>(limit_id)
                        .unwrap_or_default()
                        .max(design_m3_s)
                        .max(1200.0 / M3_H);
                    data.insert_temp(limit_id, limit);
                    limit
                });
                (flow, limit * scale)
            }
            _ => return,
        };
        let mut dragging = false;
        if max.is_finite() && max > 0.0 {
            ui.horizontal(|ui| {
                ui.label(if curve { "Free airflow" } else { "Set airflow" });
                ui.spacing_mut().slider_width = (ui.available_width() - 200.0).max(90.0);
                let response = ui.add(
                    Slider::new(&mut flow, 0.0..=max)
                        .suffix(format!(" {unit}"))
                        .max_decimals(1),
                );
                dragging = response.dragged();
                if response.changed() {
                    if let BranchKind::FlowFan { design_m3_s, .. } = &mut branch.kind {
                        *design_m3_s = flow / scale;
                    } else {
                        branch.fan_speed_fraction = Some((flow / max).clamp(0.0, 1.0));
                    }
                    self.solve_requested = true;
                }
                if let Some(q) = solved_flow.filter(|_| !response.changed()) {
                    ui.label(tag(&format!("Actual {:.1} {unit}", q * scale), p.good));
                }
            });
        }
        if dragging {
            if self.fan_slider_edit.is_none() && before != self.project {
                self.fan_slider_edit = Some(before);
            }
        } else {
            let previous = self.fan_slider_edit.take().unwrap_or(before);
            self.checkpoint(previous);
        }
    }

    pub fn analysis_view(&mut self, ui: &mut Ui, p: Palette) {
        Frame::new()
            .fill(p.surface)
            .inner_margin(Margin::same(16))
            .show(ui, |ui| {
                eyebrow(ui, if self.startup.visible { "04  /  FAN STARTUP" } else { "04  /  STEADY AIRFLOW" }, p);
                heading(ui, "Flow report");
                self.startup_controls(ui, p);
                if self.startup.visible {
                    self.startup_report(ui, p);
                    return;
                }
                let has_results = self.current_run().is_some();
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.flow_cfm, true, "CFM");
                    ui.selectable_value(&mut self.flow_cfm, false, "m³/h");
                    if ui.add_enabled(has_results, Button::new("Export CSV")).clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .set_file_name("airframe-flow.csv")
                            .add_filter("CSV", &["csv"])
                            .save_file()
                        && let Some(run) = self.current_run()
                    {
                        match storage::export_operating_point(&path, run) {
                            Ok(()) => self.notice("Flow report exported", false),
                            Err(e) => self.notice(e.to_string(), true),
                        }
                    }
                    if ui.add_enabled(has_results, Button::new("Export Excel")).clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .set_file_name("airframe-flow.xlsx")
                            .add_filter("Excel workbook", &["xlsx"])
                            .save_file()
                        && let Some(run) = self.current_run()
                    {
                        let path = if path.extension().is_none() { path.with_extension("xlsx") } else { path };
                        match airflow::excel::export(&path, &self.project.name, run) {
                            Ok(()) => self.notice("Excel workbook exported", false),
                            Err(e) => self.notice(e.to_string(), true),
                        }
                    }
                });
                self.airflow_slider(ui, p);
                let Some(run) = self.current_run() else {
                    self.empty_results(ui, p);
                    return;
                };
                let (unit, scale) = self.flow_unit();
                ui.label(tag(
                    &format!("MAX INTERNAL IMBALANCE  {:.2e} {unit}", run.max_imbalance * scale),
                    p.good,
                ));
                for warning in &run.warnings {
                    ui.label(RichText::new(warning).color(p.warning));
                }
                let mut select = None;
                ScrollArea::both().show(ui, |ui| {
                    ui.separator();
                    eyebrow(ui, "PRESSURE NODES", p);
                    Grid::new("point-nodes").striped(true).spacing([18.0, 8.0]).show(ui, |ui| {
                        columns(ui, p, &["Node", "Gauge Pa", &format!("In · {unit}"), &format!("Out · {unit}"), &format!("Net · {unit}")]);
                        for (i, node) in run.snapshot.nodes.iter().enumerate() {
                            let balance = run.balances[i];
                            if ui.selectable_label(self.picked() == Some(node.id), &node.name).clicked() {
                                select = Some(node.id);
                            }
                            ui.monospace(format!("{:+.3}", run.pressures[i]));
                            ui.monospace(format!("{:.3}", balance.inflow * scale));
                            ui.monospace(format!("{:.3}", balance.outflow * scale));
                            ui.monospace(format!("{:+.3}", balance.net() * scale));
                            ui.end_row();
                        }
                    });
                    ui.add_space(10.0);
                    ui.separator();
                    eyebrow(ui, "BRANCH FLOWS", p);
                    Grid::new("point-branches").striped(true).spacing([18.0, 8.0]).show(ui, |ui| {
                        columns(ui, p, &["Branch", "From → To", &format!("Flow · {unit}"), "Pfrom − Pto · Pa"]);
                        for (i, branch) in run.snapshot.branches.iter().enumerate() {
                            let from = run.snapshot.nodes.iter().position(|n| n.id == branch.from).expect("validated endpoint");
                            let to = run.snapshot.nodes.iter().position(|n| n.id == branch.to).expect("validated endpoint");
                            if ui.selectable_label(self.picked() == Some(branch.id), &branch.name).clicked() {
                                select = Some(branch.id);
                            }
                            ui.label(format!("{} → {}", run.snapshot.nodes[from].name, run.snapshot.nodes[to].name));
                            ui.monospace(format!("{:+.3}", run.flows[i] * scale));
                            ui.monospace(format!("{:+.3}", run.pressures[from] - run.pressures[to]));
                            ui.end_row();
                        }
                    });
                    ui.add_space(8.0);
                    ui.label(RichText::new("A negative branch flow runs opposite the listed direction. Fixed-pressure boundaries can supply or absorb air.").color(p.muted));
                });
                if let Some(id) = select {
                    self.selection = vec![id];
                }
            });
    }

    fn empty_results(&self, ui: &mut Ui, p: Palette) {
        let text = if self.results.is_some() {
            "Inputs changed. Solve airflow again to refresh the report."
        } else {
            "Set fan flows and connect a passive path to a fixed-pressure boundary, then select Solve airflow."
        };
        ui.label(RichText::new(text).color(if self.results.is_some() {
            p.warning
        } else {
            p.muted
        }));
    }
}
fn columns(ui: &mut Ui, p: Palette, names: &[&str]) {
    for name in names {
        ui.label(RichText::new(*name).strong().color(p.muted));
    }
    ui.end_row();
}
