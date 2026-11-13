use crate::style::*;
use airflow::{model::*, physics};
use eframe::egui::*;

pub(crate) fn mode_button(ui: &mut Ui, fan: &mut Branch, solved_flow: Option<f64>) -> bool {
    if matches!(fan.kind, BranchKind::PressureFan { .. }) {
        if ui.button("Use fixed airflow").clicked() {
            fan.use_fixed_airflow(solved_flow);
            return true;
        }
    } else if ui
        .button("Use fan curve")
        .on_hover_text(
            "Starts with the current flow as rated free airflow and a 75 Pa shutoff pressure.",
        )
        .clicked()
    {
        fan.enable_fan_curve();
        return true;
    }
    false
}

pub(super) fn controls(
    ui: &mut Ui,
    p: Palette,
    fan: &mut Branch,
    unit: &str,
    flow_factor: f64,
    operating_point: Option<(f64, f64)>,
) -> bool {
    let mut changed = custom_curve(ui, fan, unit, flow_factor);
    let rated_flow = fan.fan_rated_flow().unwrap_or_default() * flow_factor;
    let speed = fan.fan_speed_fraction.unwrap_or(1.0);
    if rated_flow.is_finite() && rated_flow > 0.0 {
        if let Some((q, rise)) = operating_point.filter(|_| !changed) {
            ui.strong(format!(
                "Actual {:+.2} {unit} · {rise:+.1} Pa",
                q * flow_factor
            ));
        }
        curve_plot(ui, p, fan, speed, operating_point.filter(|_| !changed));
    }
    if !fan.custom_fan_curve.is_empty() {
        return changed;
    }
    let BranchKind::PressureFan { shutoff_pa, k, .. } = &mut fan.kind else {
        return false;
    };
    if rated_flow.is_finite() && rated_flow > 0.0 {
        ui.label(tag(
            &format!(
                "{:.0}% speed  /  {:.1} Pa shutoff",
                speed * 100.0,
                *shutoff_pa * speed.powi(2)
            ),
            p.muted,
        ));
        ui.small("Adjust airflow below the canvas.");
    }
    ui.collapsing("Rated fan parameters", |ui| {
        let mut rated = if rated_flow.is_finite() && rated_flow > 0.0 {
            rated_flow
        } else {
            600.0 / M3_H * flow_factor
        };
        let original = rated;
        let original_pressure = *shutoff_pa;
        super::number(ui, "Free airflow", &mut rated, unit, 1.0);
        super::number(ui, "Shutoff pressure", shutoff_pa, "Pa", 0.5);
        if rated != original || *shutoff_pa != original_pressure {
            let coefficient = *shutoff_pa / (rated / flow_factor).powi(2);
            if rated.is_finite() && rated > 0.0 && coefficient.is_finite() && coefficient > 0.0 {
                *k = coefficient;
                changed = true;
            } else {
                *shutoff_pa = original_pressure;
            }
        }
    });
    changed
}

fn custom_curve(ui: &mut Ui, fan: &mut Branch, unit: &str, scale: f64) -> bool {
    let id = Id::new(("custom-fan-curve", fan.id, unit));
    let mut draft = ui
        .ctx()
        .data(|d| d.get_temp::<String>(id))
        .unwrap_or_else(|| {
            if fan.custom_fan_curve.is_empty() {
                let rated = fan.fan_rated_flow().unwrap_or_default() * scale;
                let pressure = match fan.kind {
                    BranchKind::PressureFan { shutoff_pa, .. } => shutoff_pa,
                    _ => 0.0,
                };
                format!(
                    "0, {pressure}\n{:.2}, {:.2}\n{rated:.2}, 0",
                    rated * 0.5,
                    pressure * 0.75
                )
            } else {
                fan.custom_fan_curve
                    .iter()
                    .map(|[q, p]| format!("{:.4}, {p:.4}", q * scale))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        });
    let mut changed = false;
    ui.collapsing(if fan.custom_fan_curve.is_empty() { "Custom fan curve" } else { "Custom fan curve · active" }, |ui| {
        ui.small(format!("One pair per line: airflow ({unit}), pressure rise (Pa). Start at zero flow and end at zero pressure."));
        if ui.add(TextEdit::multiline(&mut draft).desired_rows(4).desired_width(f32::INFINITY).code_editor()).changed() {
            ui.ctx().data_mut(|d| d.insert_temp(id, draft.clone()));
        }
        let points = draft.lines().filter(|line| !line.trim().is_empty()).map(|line| {
            let fields: Vec<_> = line.split([',', '\t', ';']).map(str::trim).collect();
            match fields.as_slice() {
                [q, p] => Some([q.parse::<f64>().ok()? / scale, p.parse::<f64>().ok()?]),
                _ => None,
            }
        }).collect::<Option<Vec<_>>>();
        let valid = points.as_ref().is_some_and(|points| physics::valid_fan_curve(points));
        ui.horizontal_wrapped(|ui| {
            if ui.add_enabled(valid, Button::new("Apply curve")).clicked() {
                fan.custom_fan_curve = points.unwrap();
                changed = true;
            }
            if !fan.custom_fan_curve.is_empty() && ui.button("Use rated curve").clicked() {
                fan.custom_fan_curve.clear();
                changed = true;
            }
        });
        if !valid { ui.small("Airflow must increase and pressure must decrease at each point."); }
    });
    changed
}

fn curve_plot(ui: &mut Ui, p: Palette, fan: &Branch, speed: f64, point: Option<(f64, f64)>) {
    let rated_flow = fan.fan_rated_flow().unwrap_or_default();
    let shutoff_pa = physics::pressure_fan_rise(fan, 1.0, 0.0);
    if !shutoff_pa.is_finite() || shutoff_pa <= 0.0 || !speed.is_finite() {
        return;
    }
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 96.0), Sense::hover());
    let plot = Rect::from_min_max(rect.min + vec2(17.0, 10.0), rect.max - vec2(10.0, 17.0));
    let x_min = point.map_or(0.0, |(q, _)| q.min(0.0));
    let x_max = point.map_or(rated_flow, |(q, _)| q.max(rated_flow));
    let y_min = point.map_or(0.0, |(_, rise)| rise.min(0.0));
    let y_max = point.map_or(shutoff_pa, |(_, rise)| rise.max(shutoff_pa));
    let xy = |flow: f64, rise: f64| {
        pos2(
            lerp(
                plot.left()..=plot.right(),
                ((flow - x_min) / (x_max - x_min)) as f32,
            ),
            lerp(
                plot.bottom()..=plot.top(),
                ((rise - y_min) / (y_max - y_min)) as f32,
            ),
        )
    };
    let painter = ui.painter();
    painter.rect(
        rect,
        6,
        p.canvas,
        Stroke::new(1.0_f32, p.edge),
        StrokeKind::Inside,
    );
    painter.line_segment(
        [
            pos2(plot.left(), xy(0.0, 0.0).y),
            pos2(plot.right(), xy(0.0, 0.0).y),
        ],
        Stroke::new(1.0_f32, p.edge),
    );
    painter.line_segment(
        [
            pos2(xy(0.0, 0.0).x, plot.top()),
            pos2(xy(0.0, 0.0).x, plot.bottom()),
        ],
        Stroke::new(1.0_f32, p.edge),
    );
    for (s, color) in [(1.0, p.muted.gamma_multiply(0.55)), (speed, p.accent)] {
        let points = (0..=32)
            .map(|i| {
                let q = s * rated_flow * f64::from(i) / 32.0;
                xy(q, physics::pressure_fan_rise(fan, s, q).max(0.0))
            })
            .collect::<Vec<_>>();
        painter.add(Shape::line(points, Stroke::new(1.6_f32, color)));
    }
    if let Some((q, rise)) = point {
        painter.circle_filled(xy(q, rise), 4.0, p.good);
    }
    painter.text(
        rect.left_top() + vec2(5.0, 3.0),
        Align2::LEFT_TOP,
        "Pa",
        FontId::monospace(9.0),
        p.muted,
    );
    painter.text(
        rect.right_bottom() - vec2(8.0, 3.0),
        Align2::RIGHT_BOTTOM,
        "Airflow →",
        FontId::monospace(9.0),
        p.muted,
    );
}
