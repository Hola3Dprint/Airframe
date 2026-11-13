//! Constitutive equations. Inputs and outputs use SI.
use crate::model::{Air, Duct, Section};

pub fn capacitance(volume_m3: f64, air: Air) -> f64 {
    volume_m3 / air.absolute_pa
}
pub fn area_and_diameter(section: Section) -> (f64, f64) {
    match section {
        Section::Round { diameter_m: d } => (std::f64::consts::PI * d * d / 4.0, d),
        Section::Rectangular {
            width_m: w,
            height_m: h,
        } => (w * h, 2.0 * w * h / (w + h)),
    }
}
pub fn inertance(duct: Duct, air: Air) -> f64 {
    air.density * duct.length_m / area_and_diameter(duct.section).0
}
pub fn quadratic_loss(k: f64, q: f64) -> f64 {
    k * q * q.abs()
}
pub fn quadratic_flow(k: f64, dp: f64) -> f64 {
    dp.signum() * (dp.abs() / k).sqrt()
}

/// Nominal airflow at zero pressure rise on the rated fan curve.
pub fn rated_free_airflow(shutoff_pa: f64, k: f64) -> f64 {
    (shutoff_pa / k).sqrt()
}

pub(crate) const LOW_FLOW: f64 = 1e-5;
pub(crate) fn opening_loss(k: f64, q: f64) -> f64 {
    k * q * q.hypot(LOW_FLOW)
}

/// Fan pressure rise at a given speed fraction and signed branch airflow.
pub fn fan_pressure_rise(shutoff_pa: f64, k: f64, speed: f64, flow: f64) -> f64 {
    shutoff_pa * speed.powi(2) - opening_loss(k, flow)
}

pub fn valid_fan_curve(points: &[[f64; 2]]) -> bool {
    points.len() >= 2
        && points[0][0] == 0.0
        && points[0][1] > 0.0
        && points.last().is_some_and(|p| p[1] == 0.0)
        && points.iter().flatten().all(|v| v.is_finite() && *v >= 0.0)
        && points
            .windows(2)
            .all(|p| p[0][0] < p[1][0] && p[0][1] > p[1][1])
}

pub fn pressure_fan_rise(branch: &crate::model::Branch, speed: f64, flow: f64) -> f64 {
    use crate::model::BranchKind;
    let BranchKind::PressureFan { shutoff_pa, k, .. } = branch.kind else {
        return 0.0;
    };
    let points = &branch.custom_fan_curve;
    let [rated_flow, _] = match points.last() {
        Some(point) if points.len() >= 2 => *point,
        _ => return fan_pressure_rise(shutoff_pa, k, speed, flow),
    };
    let drag = points[0][1] / rated_flow.powi(2);
    if speed <= 1e-8 {
        return -opening_loss(drag, flow);
    }
    if flow < 0.0 {
        return points[0][1] * speed.powi(2) - opening_loss(drag, flow);
    }
    let rated_q = flow / speed;
    let next = points
        .partition_point(|point| point[0] < rated_q)
        .clamp(1, points.len() - 1);
    let ([qa, pa], [qb, pb]) = (points[next - 1], points[next]);
    speed.powi(2) * (pa + (rated_q - qa) * (pb - pa) / (qb - qa))
}

pub fn duct_loss(duct: Duct, air: Air, q: f64) -> f64 {
    let (area, diameter) = area_and_diameter(duct.section);
    let speed = q / area;
    let re = air.density * speed.abs() * diameter / air.viscosity;
    let laminar = 32.0 * air.viscosity * duct.length_m * speed / diameter.powi(2);
    let dynamic_pa = 0.5 * air.density * speed * speed.abs();
    let friction = if re <= 2300.0 {
        laminar
    } else {
        let f = 0.25
            / (duct.roughness_m / (3.7 * diameter) + 5.74 / re.powf(0.9))
                .log10()
                .powi(2);
        let turbulent = dynamic_pa * f * duct.length_m / diameter;
        let blend = ((re - 2300.0) / 1700.0).min(1.0);
        laminar * (1.0 - blend) + turbulent * blend
    };
    friction + dynamic_pa * duct.minor_k
}
pub fn duct_flow(duct: Duct, air: Air, dp: f64) -> f64 {
    if dp == 0.0 {
        return 0.0;
    }
    let target = dp.abs();
    let mut lo = 0.0;
    let mut hi = 0.01;
    while duct_loss(duct, air, hi) < target && hi < 1.0e9 {
        hi *= 2.0;
    }
    for _ in 0..48 {
        let mid = (lo + hi) * 0.5;
        if duct_loss(duct, air, mid) < target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    dp.signum() * (hi + lo) * 0.5
}
