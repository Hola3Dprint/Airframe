use crate::{style::*, workbench::Workbench};
use airflow::{model::*, physics};
use eframe::egui::*;
use uuid::Uuid;

mod bulk;
pub(crate) mod fan;

pub fn number(ui: &mut Ui, label: &str, value: &mut f64, unit: &str, speed: f64) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if !unit.is_empty() {
                ui.label(RichText::new(unit).size(10.0));
            }
            ui.add(DragValue::new(value).speed(speed).max_decimals(12));
        });
    });
}
fn group(ui: &mut Ui, p: Palette, name: &str) {
    ui.add_space(4.0);
    ui.separator();
    eyebrow(ui, name, p);
}

impl Workbench {
    pub fn inspector(&mut self, ctx: &Context, p: Palette) {
        SidePanel::right("inspector")
            .default_width(272.0)
            .width_range(224.0..=420.0)
            .frame(panel(p, 13))
            .show(ctx, |ui| {
                ScrollArea::vertical().show(ui, |ui| {
                    eyebrow(ui, "03  /  PROPERTIES", p);
                    heading(ui, "Inspector");
                    let prev = self.project.clone();
                    if let Some(id) = self.picked() {
                        if self.selection.len() == 1 && let Some(container) = self.project.drawing.containers.get(&id) {
                            let editable = self.component_editable(id);
                            let mut name = container.name.clone();
                            ui.label(tag("CONTAINER", p.accent));
                            if ui.add_enabled(editable, TextEdit::singleline(&mut name)).changed() {
                                self.project.drawing.containers.get_mut(&id).unwrap().name = name;
                            }
                            ui.small("Drag the header to move everything inside. Delete removes the container only.");
                        } else if self.selection.len() > 1 {
                            self.inspect_many(ui, p);
                        } else {
                            ui.add_enabled_ui(self.component_editable(id), |ui| {
                                self.inspect_component(ui, p, id);
                            });
                            if !self.component_editable(id) {
                                ui.label(tag("READ ONLY · SWITCH TO ITS FLOOR", p.warning));
                            }
                            if self.project.network.node(id).is_some()
                                && ui.button("Save as custom node…").clicked()
                            {
                                self.preset_from(id);
                            }
                        }
                        if ui
                            .add_enabled(
                                self.selection.iter().any(|id| self.component_editable(*id)),
                                Button::new("Delete selection"),
                            )
                            .clicked()
                        {
                            self.remove_selected();
                            return;
                        }
                    } else {
                        ui.label(
                            RichText::new(
                                "Select a room, boundary or branch to edit its properties.",
                            )
                            .color(p.muted),
                        );
                        group(ui, p, "ACTIVE FLOOR");
                        if let Some(f) = self
                            .project
                            .network
                            .floors
                            .iter_mut()
                            .find(|f| f.id == self.active_floor)
                        {
                            ui.text_edit_singleline(&mut f.name);
                            number(ui, "Elevation", &mut f.elevation_m, "m", 0.1);
                        }
                        group(ui, p, "PROJECT NOTES");
                        ui.label(RichText::new(&self.project.notes).color(p.muted));
                    }
                    self.checkpoint(prev);
                });
            });
    }
    fn inspect_component(&mut self, ui: &mut Ui, p: Palette, id: Uuid) {
        let net = &self.project.network;
        let air = net.air;
        let (unit, factor) = self.flow_unit();
        let pressure = self.pressure(id);
        let flow = self.flow(id);
        let balance = self.display_balance(id);
        if let Some(riser) = self.project.drawing.riser_landings.get(&id).copied() {
            ui.label(tag("RISER LANDING", p.accent));
            if self.riser_controls(ui, p, riser, Some(id)) {
                return;
            }
        } else if net
            .branch(id)
            .is_some_and(|b| matches!(b.kind, BranchKind::Duct { riser: true, .. }))
            && self.riser_controls(ui, p, id, None)
        {
            return;
        }
        let net = &self.project.network;
        if let Some(i) = net.nodes.iter().position(|n| n.id == id) {
            let node = &mut self.project.network.nodes[i];
            ui.label(tag(&node.kind.label().to_uppercase(), p.accent));
            ui.add(TextEdit::singleline(&mut node.name).desired_width(f32::INFINITY));
            let floor = self
                .project
                .network
                .floors
                .iter()
                .find(|f| f.id == node.floor);
            ui.label(
                RichText::new(floor.map_or("Unknown floor", |f| f.name.as_str())).color(p.muted),
            );
            group(ui, p, "PHYSICAL PARAMETERS");
            match &mut node.kind {
                NodeKind::Room { volume_m3, .. } => {
                    number(ui, "Room volume", volume_m3, "m³", 1.0);
                    ui.label("Room pressure is calculated from its flow balance.");
                }
                NodeKind::Boundary { gauge_pa } => number(ui, "Fixed gauge", gauge_pa, "Pa", 0.1),
                NodeKind::Atmosphere => {
                    ui.label("Outdoor reference at 0 Pa gauge.");
                }
                NodeKind::Junction => {
                    ui.label("Zero storage. Inflow equals outflow.");
                }
            }
            group(ui, p, "CALCULATED / EQUATION");
            node_math(ui, p, node.kind);
            if let NodeKind::Room { volume_m3, .. } = node.kind {
                group(ui, p, "ROOM CAPACITOR");
                ui.monospace(format!(
                    "C = {:.4e} m³/Pa",
                    physics::capacitance(volume_m3, air)
                ));
            }
            if let Some(v) = pressure {
                ui.label(format!("Gauge pressure: {v:+.3} Pa"));
            }
            if let Some(q) = balance {
                ui.label(format!("Inflow: {:.2} {unit}", q.inflow * factor));
                ui.label(format!("Outflow: {:.2} {unit}", q.outflow * factor));
                ui.label(format!("Net: {:+.2} {unit}", q.net() * factor));
            }
        } else if let Some(i) = net.branches.iter().position(|b| b.id == id) {
            let (a, z) = (net.branches[i].from, net.branches[i].to);
            let endpoints = [net.node(a).cloned(), net.node(z).cloned()];
            let dp = self.pressure(a).zip(self.pressure(z)).map(|(a, z)| a - z);
            let color = self.color(net.branches[i].system);
            let b = &mut self.project.network.branches[i];
            ui.label(tag(&b.kind.label().to_uppercase(), color));
            ui.add(TextEdit::singleline(&mut b.name).desired_width(f32::INFINITY));
            ComboBox::from_id_salt("system")
                .selected_text(b.system.label())
                .show_ui(ui, |ui| {
                    for s in System::ALL {
                        ui.selectable_value(&mut b.system, s, s.label());
                    }
                });
            if b.kind.tau().is_some() {
                let curve = matches!(b.kind, BranchKind::PressureFan { .. });
                if fan::mode_button(ui, b, flow) {
                    self.solve_requested = true;
                }
                if curve && matches!(b.kind, BranchKind::PressureFan { .. }) {
                    group(ui, p, "FAN CURVE");
                    self.solve_requested |=
                        fan::controls(ui, p, b, unit, factor, flow.zip(dp.map(|v| -v)));
                }
            }
            group(ui, p, "CONNECTIONS");
            for (j, node) in endpoints.iter().enumerate() {
                let prefix = if j == 0 { "From" } else { "To" };
                ui.horizontal(|ui| {
                    if ui
                        .small_button(prefix)
                        .on_hover_text("Choose a replacement endpoint on the building canvas")
                        .clicked()
                    {
                        self.view = crate::workbench::View::Building;
                        self.tool = None;
                        self.from = None;
                        self.reconnect = Some((id, j == 0));
                    }
                    ui.label(
                        node.as_ref()
                            .map_or("Missing endpoint", |n| n.name.as_str()),
                    );
                    if let Some(node) = node
                        && node.floor != self.active_floor
                        && ui.small_button("Go").clicked()
                    {
                        self.active_floor = node.floor;
                    }
                });
            }
            group(ui, p, "PHYSICAL PARAMETERS");
            match &mut b.kind {
                BranchKind::Opening { k } => {
                    if let Some(aperture) = &mut b.aperture {
                        number(ui, "Free area", &mut aperture.area_m2, "m²", 0.01);
                        number(
                            ui,
                            "Discharge coefficient",
                            &mut aperture.discharge,
                            "",
                            0.01,
                        );
                        ui.add(
                            Slider::new(&mut aperture.open_fraction, 0.0..=1.0)
                                .text("Open fraction"),
                        );
                        ui.small(format!(
                            "Effective loss coefficient {:.3} Pa·s²/m⁶",
                            aperture.resistance(air.density)
                        ));
                    } else {
                        number(ui, "Loss coefficient", k, "Pa·s²/m⁶", 10.0);
                    }
                }
                BranchKind::Duct { duct, .. } => {
                    let rect = matches!(duct.section, Section::Rectangular { .. });
                    ui.horizontal(|ui| {
                        if ui.selectable_label(!rect, "Round").clicked() && rect {
                            duct.section = Section::Round { diameter_m: 0.25 };
                        }
                        if ui.selectable_label(rect, "Rectangular").clicked() && !rect {
                            duct.section = Section::Rectangular {
                                width_m: 0.3,
                                height_m: 0.2,
                            };
                        }
                    });
                    match &mut duct.section {
                        Section::Round { diameter_m } => {
                            number(ui, "Diameter", diameter_m, "m", 0.005)
                        }
                        Section::Rectangular { width_m, height_m } => {
                            number(ui, "Width", width_m, "m", 0.005);
                            number(ui, "Height", height_m, "m", 0.005);
                        }
                    }
                    number(ui, "Length", &mut duct.length_m, "m", 0.1);
                    let mut rough = duct.roughness_m * 1000.0;
                    number(ui, "Roughness", &mut rough, "mm", 0.002);
                    if (rough - duct.roughness_m * 1000.0).abs() > 1e-12 {
                        duct.roughness_m = rough / 1000.0;
                    }
                    number(ui, "Fitting losses ΣK", &mut duct.minor_k, "", 0.05);
                    number(
                        ui,
                        "Additional loss",
                        &mut duct.additional_loss_pa,
                        "Pa",
                        0.5,
                    );
                    let mut reference_flow = duct.loss_airflow_m3_s * factor;
                    number(ui, "At airflow", &mut reference_flow, unit, 1.0);
                    if reference_flow != duct.loss_airflow_m3_s * factor {
                        duct.loss_airflow_m3_s = reference_flow / factor;
                    }
                    ui.small("Additional loss scales with airflow squared.");
                    let (a, d) = physics::area_and_diameter(duct.section);
                    ui.label(format!("Area {a:.4} m²  ·  Dₕ {d:.3} m"));
                    ui.label(format!(
                        "L = {:.2} Pa·s²/m³",
                        physics::inertance(*duct, air)
                    ));
                    ui.label("L · dQ/dt = 0 at steady airflow.");
                }
                BranchKind::FlowFan { design_m3_s, .. } => {
                    let mut q = *design_m3_s * factor;
                    number(ui, "Set airflow", &mut q, unit, 1.0);
                    if (q - *design_m3_s * factor).abs() > 1e-12 {
                        *design_m3_s = q / factor;
                    }
                }
                BranchKind::PressureFan { .. } => {}
            }
            number(ui, "Start elevation", &mut b.start_elevation_m, "m", 0.1);
            number(ui, "End elevation", &mut b.end_elevation_m, "m", 0.1);
            group(ui, p, "CALCULATED / EQUATION");
            branch_math(ui, p, b);
            if let Some(q) = flow {
                ui.label(format!("Flow: {:+.2} {unit}", q * factor));
            }
            if let Some(dp) = dp {
                if matches!(b.kind, BranchKind::PressureFan { .. }) {
                    ui.label(format!("Fan pressure rise: {:+.3} Pa", -dp));
                } else {
                    ui.label(format!("Pressure difference: {dp:+.3} Pa"));
                }
            }
        }
        ui.add_space(4.0);
        ui.label(tag(&id.to_string()[..8], p.muted))
            .on_hover_text(id.to_string());
    }
    fn riser_controls(&mut self, ui: &mut Ui, p: Palette, id: Uuid, landing: Option<Uuid>) -> bool {
        let net = &self.project.network;
        let Some(riser) = net.branch(id) else {
            return false;
        };
        let (Some(from), Some(to)) = (net.node(riser.from), net.node(riser.to)) else {
            return false;
        };
        let from_floor = from.floor;
        let to_floor = to.floor;
        let editable = self.component_editable(id);
        group(ui, p, "CONNECTED FLOORS");
        let current = landing
            .and_then(|id| net.node(id))
            .map_or(self.active_floor, |node| node.floor);
        if let Some(floor) = net.floor(current) {
            ui.label(format!("Landing floor: {}", floor.name));
        }
        if editable {
            ui.small("Destination floor");
            let mut chosen = to_floor;
            ComboBox::from_id_salt(("riser-destination", id))
                .selected_text(
                    net.floor(to_floor)
                        .map_or("Choose destination", |f| f.name.as_str()),
                )
                .show_ui(ui, |ui| {
                    for floor in net.floors.iter().filter(|f| f.visible) {
                        ui.selectable_value(&mut chosen, floor.id, &floor.name);
                    }
                })
                .response
                .on_hover_text("Destination floor. Selecting one opens its riser landing.");
            if chosen != to_floor {
                self.set_riser_destination(id, chosen);
                return true;
            }
            if landing.is_some() && ui.small_button("Edit riser properties").clicked() {
                self.selection = vec![id];
                return true;
            }
        }
        let other = if landing == Some(to.id)
            || (self.active_floor == to_floor && from_floor != to_floor)
        {
            from
        } else {
            to
        };
        if let Some(floor) = net.floor(other.floor)
            && ui
                .add_enabled(
                    floor.visible,
                    Button::new(if from_floor == to_floor {
                        "Go to other end".into()
                    } else {
                        format!("Go to {}", floor.name)
                    }),
                )
                .clicked()
        {
            self.go_to_landing(other.id);
            return true;
        }
        false
    }
    pub fn equation_window(&mut self, ctx: &Context, p: Palette) {
        if !self.equations {
            return;
        }
        Window::new("Network equations")
            .open(&mut self.equations)
            .default_width(570.0)
            .show(ctx, |ui| {
                eyebrow(ui, "PRESSURE ↔ VOLTAGE   /   FLOW ↔ CURRENT", p);
                ui.label("Fluid engineering units · pressure in Pa, flow in m³/s");
                let net = &self.project.network;
                ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                    for n in &net.nodes {
                        let flows = net
                            .branches
                            .iter()
                            .filter_map(|b| {
                                if b.from == n.id {
                                    Some(format!("− {}", b.name))
                                } else if b.to == n.id {
                                    Some(format!("+ {}", b.name))
                                } else {
                                    None
                                }
                            })
                            .collect::<Vec<_>>()
                            .join("  ·  ");
                        ui.label(RichText::new(&n.name).strong());
                        node_math(ui, p, n.kind);
                        if !flows.is_empty() {
                            ui.small(flows);
                        }
                    }
                    ui.separator();
                    for b in &net.branches {
                        let from = net.node(b.from).map_or("?", |n| n.name.as_str());
                        let to = net.node(b.to).map_or("?", |n| n.name.as_str());
                        ui.label(RichText::new(format!("{}  ·  {from} → {to}", b.name)).strong());
                        branch_math(ui, p, b);
                    }
                });
            });
    }
}
enum Math<'a> {
    Text(&'a str),
    Subscript(&'a str, &'a str),
    Fraction(&'a str, &'a str),
}
use Math::{Fraction as F, Subscript as S, Text as T};

fn display_math(ui: &mut Ui, p: Palette, terms: &[Math<'_>]) {
    let font = FontId::proportional(15.0);
    let text = |x: &str| {
        ui.painter()
            .layout_no_wrap(x.to_owned(), font.clone(), p.accent)
    };
    let glyphs: Vec<_> = terms
        .iter()
        .map(|term| match term {
            T(value) => (text(value), None, false),
            F(top, bottom) => (text(top), Some(text(bottom)), true),
            S(value, sub) => (
                text(value),
                Some(ui.painter().layout_no_wrap(
                    (*sub).to_owned(),
                    FontId::proportional(10.0),
                    p.accent,
                )),
                false,
            ),
        })
        .collect();
    let width = glyphs
        .iter()
        .map(|(a, b, fraction)| {
            b.as_ref().map_or(a.size().x, |b| {
                if *fraction {
                    a.size().x.max(b.size().x) + 8.0
                } else {
                    a.size().x + b.size().x
                }
            })
        })
        .sum::<f32>();
    let height = if glyphs.iter().any(|(_, _, fraction)| *fraction) {
        47.0
    } else {
        28.0
    };
    let (rect, painter) = ui.allocate_painter(vec2(ui.available_width(), height), Sense::hover());
    painter.rect_filled(rect.rect, 5, p.canvas);
    let center = rect.rect.center();
    let mut x = (center.x - width / 2.0).max(rect.rect.left() + 4.0);
    for (a, b, fraction) in glyphs {
        if fraction {
            let b = b.expect("fraction denominator");
            let w = a.size().x.max(b.size().x) + 8.0;
            let mid = x + w * 0.5;
            painter.galley(
                pos2(mid - a.size().x / 2.0, center.y - a.size().y - 2.0),
                a,
                p.accent,
            );
            painter.galley(pos2(mid - b.size().x / 2.0, center.y + 2.0), b, p.accent);
            painter.line_segment(
                [pos2(x + 2.0, center.y), pos2(x + w - 2.0, center.y)],
                Stroke::new(1.0_f32, p.accent),
            );
            x += w;
        } else {
            let w = a.size().x;
            painter.galley(pos2(x, center.y - a.size().y / 2.0), a, p.accent);
            x += w;
            if let Some(sub) = b {
                let w = sub.size().x;
                painter.galley(pos2(x, center.y), sub, p.accent);
                x += w;
            }
        }
    }
}

fn node_math(ui: &mut Ui, p: Palette, kind: NodeKind) {
    match kind {
        NodeKind::Boundary { .. } => display_math(ui, p, &[T("P = "), S("P", "set")]),
        NodeKind::Atmosphere => display_math(ui, p, &[T("P = 0 Pa")]),
        NodeKind::Junction => display_math(
            ui,
            p,
            &[S("∑ Q", "in"), T(" − "), S("∑ Q", "out"), T(" = 0")],
        ),
        NodeKind::Room { .. } => {
            display_math(
                ui,
                p,
                &[S("∑ Q", "in"), T(" − "), S("∑ Q", "out"), T(" = 0")],
            );
            display_math(
                ui,
                p,
                &[
                    T("C = "),
                    F("V", "P(abs)"),
                    T("     C "),
                    F("dP", "dt"),
                    T(" = "),
                    S("Q", "net"),
                ],
            );
        }
    }
}

fn branch_math(ui: &mut Ui, p: Palette, branch: &Branch) {
    match branch.kind {
        BranchKind::FlowFan { .. } => {
            display_math(ui, p, &[T("Q = "), S("Q", "set")]);
            display_math(
                ui,
                p,
                &[T("τ "), F("dQ", "dt"), T(" = "), S("Q", "set"), T(" − Q")],
            );
        }
        BranchKind::PressureFan { .. } if branch.custom_fan_curve.is_empty() => {
            display_math(ui, p, &[T("P₂ − P₁ = P₀s² − kQ√(Q² + ε²)")])
        }
        BranchKind::PressureFan { .. } => {
            display_math(ui, p, &[T("P₂ − P₁ = s² F ("), F("Q", "s"), T(")")]);
            ui.small("Rated curve F uses linear interpolation; reverse flow or zero speed uses a resistance.");
        }
        BranchKind::Duct { .. } => {
            display_math(
                ui,
                p,
                &[T("P₁ − P₂ = "), S("ΔP", "loss"), T(" + L "), F("dQ", "dt")],
            );
            display_math(ui, p, &[T("L = "), F("ρℓ", "A")]);
            ui.small("Duct loss includes wall friction, fittings, and any additional loss set at a reference airflow.");
        }
        BranchKind::Opening { .. } => {
            display_math(ui, p, &[T("P₁ − P₂ = kQ√(Q² + ε²)")]);
            if branch.aperture.is_some() {
                display_math(ui, p, &[T("k = "), F("ρ", "2 (Cd A φ)²")]);
            }
        }
    }
}
