use super::{fan, group};
use crate::{style::*, workbench::Workbench};
use airflow::{
    circuit::{self, Element, Symbol},
    model::BranchKind,
    physics,
};
use eframe::egui::*;
use uuid::Uuid;

impl Workbench {
    pub(super) fn inspect_circuit_symbol(
        &self,
        ui: &mut Ui,
        p: Palette,
        id: Uuid,
        symbol: Symbol,
    ) -> bool {
        let net = &self.project.network;
        if let Some(node) = net.node(id) {
            let element = circuit::node_element(node, net.air);
            if element.symbol != symbol {
                return false;
            }
            element_details(ui, p, &node.name, &element);
            if let Some(pressure) = self.pressure(id) {
                ui.label(format!("Gauge pressure {pressure:+.3} Pa"));
            }
            return true;
        }
        let Some(branch) = net.branch(id) else {
            return false;
        };
        let Some(mut element) =
            circuit::branch_elements(branch, net.air).find(|e| e.symbol == symbol)
        else {
            return false;
        };
        let speed = self
            .startup_sample()
            .and_then(|(run, sample)| {
                run.snapshot
                    .branches
                    .iter()
                    .position(|b| b.id == id)
                    .and_then(|i| sample.fan_fraction[i])
            })
            .unwrap_or_else(|| branch.fan_speed_fraction.unwrap_or(1.0));
        if symbol == Symbol::VoltageSource {
            element.value = Some((physics::pressure_fan_rise(branch, speed, 0.0), "Pa"));
        }
        element_details(ui, p, &branch.name, &element);
        let flow = self.flow(id);
        let (unit, factor) = self.flow_unit();
        match symbol {
            Symbol::Resistor => {
                let loss =
                    |q| circuit::resistor_loss(branch, net.air, speed, q).unwrap_or(f64::NAN);
                let range = match branch.kind {
                    BranchKind::Duct { duct, .. } => {
                        let area = physics::area_and_diameter(duct.section).0;
                        (3.0 * area).max(if duct.additional_loss_pa > 0.0 {
                            duct.loss_airflow_m3_s
                        } else {
                            0.0
                        })
                    }
                    BranchKind::PressureFan { .. } => {
                        branch.fan_rated_flow().unwrap_or(0.1) * speed.max(0.25)
                    }
                    _ => 0.1,
                };
                if let Some(q) = flow {
                    let dp = loss(q);
                    ui.label(format!("Operating airflow {:+.2} {unit}", q * factor));
                    ui.label(format!("Pressure loss {dp:+.3} Pa"));
                }
                let span = range.max(flow.map_or(0.0, |q| 1.25 * q.abs())).max(1.0e-6);
                let points: Vec<_> = (0..=64)
                    .map(|i| {
                        let q = span * (f64::from(i) / 32.0 - 1.0);
                        (q * factor, loss(q))
                    })
                    .collect();
                loss_plot(ui, p, &points, flow.map(|q| (q * factor, loss(q))), unit);
            }
            Symbol::Inductor => {
                if self.current_run().is_some() && self.startup_sample().is_none() {
                    ui.label("Steady pressure contribution 0 Pa");
                }
                ui.small("The startup solver uses quasi-steady duct flow.");
            }
            Symbol::VoltageSource => {
                ui.label(format!("Fan speed {:.0}%", 100.0 * speed));
                group(ui, p, "FAN CHARACTERISTIC");
                let point = flow.zip(
                    self.pressure(branch.from)
                        .zip(self.pressure(branch.to))
                        .map(|(a, b)| b - a),
                );
                fan::curve_plot(ui, p, branch, speed, point);
            }
            Symbol::CurrentSource => {
                if let Some(q) = flow {
                    ui.label(format!("Operating airflow {:+.2} {unit}", q * factor));
                }
            }
            _ => {}
        }
        true
    }
}

fn element_details(ui: &mut Ui, p: Palette, name: &str, element: &Element) {
    ui.label(tag(element.symbol.label(), p.accent));
    ui.label(name);
    group(ui, p, "ELEMENT");
    ui.label(element.equation);
    if let Some((value, unit)) = element.value {
        let value = if value != 0.0 && (value.abs() < 0.01 || value.abs() >= 1.0e5) {
            format!("{value:.4e}")
        } else {
            format!("{value:.4}")
        };
        ui.monospace(format!("{value} {unit}"));
    }
}

fn loss_plot(
    ui: &mut Ui,
    p: Palette,
    points: &[(f64, f64)],
    operating: Option<(f64, f64)>,
    unit: &str,
) {
    if points
        .iter()
        .any(|(q, loss)| !q.is_finite() || !loss.is_finite())
    {
        ui.small("Check the component parameters to plot its loss curve.");
        return;
    }
    let x_max = points.last().map_or(1.0, |(x, _)| *x);
    let y_max = points
        .iter()
        .map(|(_, y)| y.abs())
        .fold(1.0e-6_f64, f64::max);
    group(ui, p, "PRESSURE LOSS CURVE");
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), 144.0), Sense::hover());
    let plot = Rect::from_min_max(rect.min + vec2(47.0, 14.0), rect.max - vec2(13.0, 25.0));
    let at = |(x, y): (f64, f64)| {
        plot.center()
            + vec2(
                (x / x_max) as f32 * plot.width() * 0.5,
                -(y / y_max) as f32 * plot.height() * 0.5,
            )
    };
    let painter = ui.painter();
    painter.rect_filled(rect, 5, p.canvas);
    let axes = Stroke::new(1.0_f32, p.edge);
    painter.line_segment([plot.left_center(), plot.right_center()], axes);
    painter.line_segment([plot.center_top(), plot.center_bottom()], axes);
    painter.add(Shape::line(
        points.iter().copied().map(at).collect(),
        Stroke::new(1.8_f32, p.accent),
    ));
    let font = FontId::monospace(9.0);
    for y in [y_max, 0.0, -y_max] {
        painter.text(
            pos2(plot.left() - 4.0, at((0.0, y)).y),
            Align2::RIGHT_CENTER,
            format!("{y:.2}"),
            font.clone(),
            p.muted,
        );
    }
    painter.text(
        rect.left_top() + vec2(5.0, 3.0),
        Align2::LEFT_TOP,
        "Pa",
        font.clone(),
        p.muted,
    );
    painter.text(
        plot.left_bottom() + vec2(0.0, 5.0),
        Align2::LEFT_TOP,
        format!("{:.0}", -x_max),
        font.clone(),
        p.muted,
    );
    painter.text(
        plot.right_bottom() + vec2(0.0, 5.0),
        Align2::RIGHT_TOP,
        format!("{x_max:.0} {unit}"),
        font,
        p.muted,
    );
    if let Some(point) = operating {
        painter.circle_filled(at(point), 3.5, p.good);
    }
    if let Some(pointer) = response.hover_pos().filter(|pos| plot.contains(*pos)) {
        let index =
            ((pointer.x - plot.left()) / plot.width() * (points.len() - 1) as f32).round() as usize;
        let (flow, loss) = points[index];
        painter.circle_filled(at((flow, loss)), 3.0, p.accent);
        response.on_hover_text_at_pointer(format!("{flow:+.2} {unit} · {loss:+.3} Pa"));
    }
}
