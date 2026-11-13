use airflow::{
    model::*,
    physics::*,
    solver::{SolveError, simulate},
    storage,
};
use std::path::PathBuf;
const SINGLE: &[u8] = include_bytes!("../../../examples/single-room.airflow.json");
const TWO: &[u8] = include_bytes!("../../../examples/two-floor.airflow.json");
fn single() -> Project {
    storage::decode(SINGLE).unwrap()
}
fn run(p: &Project) -> airflow::solver::Run {
    simulate(&p.network, &p.scenarios[0], |_| true).unwrap()
}
fn close(actual: f64, expected: f64, atol: f64) {
    assert!(
        (actual - expected).abs() <= atol,
        "{actual:.12e} vs {expected:.12e}, tolerance {atol:e}"
    );
}

#[test]
fn off_is_zero_equilibrium() {
    let mut p = single();
    p.scenarios[0].events.clear();
    for s in run(&p).samples {
        close(s.pressures[1], 0.0, 1e-10);
        for q in s.flows {
            close(q, 0.0, 1e-10);
        }
    }
}

#[test]
fn startup_shutdown_and_analytic_fan_response() {
    let p = single();
    let r = run(&p);
    close(r.at(0.0).pressures[1], 0.0, 1e-10);
    assert!(r.at(5.0).pressures[1] < -5.0);
    close(r.at(60.0).pressures[1], -50.0, 0.004);
    assert!(r.at(70.0).pressures[1] > r.at(60.0).pressures[1]);
    close(r.at(120.0).pressures[1], 0.0, 2e-6);
    for s in &r.samples {
        let t = s.time_s;
        let q = if t <= 60.0 {
            (1.0 - (-t / 5.0).exp()) / 6.0
        } else {
            (1.0 - (-12.0_f64).exp()) / 6.0 * (-(t - 60.0) / 5.0).exp()
        };
        close(s.flows[1], q, 2e-7);
        close(s.pressures[1], -quadratic_loss(1800.0, s.flows[0]), 0.001);
    }
}

#[test]
fn independent_rk4_pressure_reference() {
    // A scalar fixed-step RK4 driven by the closed-form fan flow. The endpoint
    // pressure is only compared at times where this explicit reference is stable.
    let p = single();
    let r = run(&p);
    let dt = 0.0001;
    let mut pressure = 0.0_f64;
    let c = 100.0 / 101325.0;
    let derivative = |t: f64, p: f64| {
        let fan = if t <= 60.0 {
            (1.0 - (-t / 5.0).exp()) / 6.0
        } else {
            (1.0 - (-12.0_f64).exp()) / 6.0 * (-(t - 60.0) / 5.0).exp()
        };
        let inlet = (-p).signum() * (p.abs() / 1800.0).sqrt();
        (inlet - fan) / c
    };
    for i in 0..710_000 {
        let t = i as f64 * dt;
        let a = derivative(t, pressure);
        let b = derivative(t + dt * 0.5, pressure + dt * 0.5 * a);
        let c = derivative(t + dt * 0.5, pressure + dt * 0.5 * b);
        let d = derivative(t + dt, pressure + dt * c);
        pressure += dt * (a + 2.0 * b + 2.0 * c + d) / 6.0;
        if [10_000, 50_000, 300_000, 600_000, 650_000, 710_000].contains(&(i + 1)) {
            close(r.at((i + 1) as f64 * dt).pressures[1], pressure, 0.001);
        }
    }
}

#[test]
fn volume_and_fan_tau_change_transient() {
    let mut p = single();
    let baseline = run(&p).at(5.0).pressures[1];
    if let NodeKind::Room { volume_m3, .. } = &mut p.network.nodes[1].kind {
        *volume_m3 = 400.0;
    }
    let large = run(&p).at(5.0).pressures[1];
    assert!(large > baseline + 0.1);
    if let BranchKind::FlowFan { tau_s, .. } = &mut p.network.branches[1].kind {
        *tau_s = 10.0;
    }
    assert!(run(&p).at(5.0).pressures[1] > large + 1.0);
}

#[test]
fn signed_and_rectangular_resistance() {
    close(quadratic_loss(1800.0, -1.0 / 6.0), -50.0, 1e-12);
    close(quadratic_flow(1800.0, -50.0), -1.0 / 6.0, 1e-12);
    let section = Section::Rectangular {
        width_m: 0.3,
        height_m: 0.2,
    };
    let (area, d) = area_and_diameter(section);
    close(area, 0.06, 1e-12);
    close(d, 0.24, 1e-12);
    let duct = Duct {
        section,
        ..Duct::default()
    };
    let air = Air::default();
    let q = 0.2;
    let loss = duct_loss(duct, air, q);
    assert!(loss > 0.0);
    close(duct_loss(duct, air, -q), -loss, 1e-12);
    close(duct_flow(duct, air, -loss), -q, 1e-12);
}

#[test]
fn series_and_parallel_at_a_zero_storage_junction() {
    let mut p = Project::new("Series parallel");
    let f = p.network.floors[0].id;
    let a = p.add_node("High", f, NodeKind::Boundary { gauge_pa: 90.0 }, [0., 0.]);
    let b = p.add_node("Junction", f, NodeKind::Junction, [100., 0.]);
    let c = p.add_node("Low", f, NodeKind::Boundary { gauge_pa: 0.0 }, [200., 0.]);
    p.add_branch(
        "Feed",
        a,
        b,
        System::Supply,
        BranchKind::Opening { k: 1000.0 },
    );
    p.add_branch(
        "Split A",
        b,
        c,
        System::Supply,
        BranchKind::Opening { k: 1000.0 },
    );
    p.add_branch(
        "Split B",
        b,
        c,
        System::Supply,
        BranchKind::Opening { k: 4000.0 },
    );
    let r = run(&p);
    assert!(r.max_junction_residual < 1e-8);
    let s = r.at(60.0);
    close(s.flows[0], s.flows[1] + s.flows[2], 1e-8);
    close(s.flows[1], 2.0 * s.flows[2], 1e-7);
    close(
        quadratic_loss(1000.0, s.flows[0]) + quadratic_loss(1000.0, s.flows[1]),
        90.0,
        1e-5,
    );
    close(quadratic_loss(4000.0, s.flows[2]), s.pressures[1], 1e-5);
}

#[test]
fn pressure_fan_solves_operating_point() {
    let mut p = Project::new("Fan curve");
    let f = p.network.floors[0].id;
    let a = p.add_node("In", f, NodeKind::Boundary { gauge_pa: 0.0 }, [0., 0.]);
    let b = p.add_node("Out", f, NodeKind::Boundary { gauge_pa: 20.0 }, [100., 0.]);
    let fan = p.add_branch(
        "Fan",
        a,
        b,
        System::Supply,
        BranchKind::PressureFan {
            shutoff_pa: 60.0,
            k: 1000.0,
            tau_s: 1.0,
        },
    );
    p.scenarios[0].events.push(Event {
        fan,
        time_s: 0.0,
        command: 1.0,
    });
    let r = run(&p);
    assert!(r.at(0.0).flows[0] < 0.0);
    close(r.at(60.0).flows[0], 0.2, 1e-7);
}

#[test]
fn stacked_layout_preserves_riser_and_independent_endpoints() {
    let mut p = storage::decode(TWO).unwrap();
    let r = run(&p);
    assert!(r.samples.last().unwrap().time_s == 120.0);
    let stacked: Vec<_> = p
        .network
        .branches
        .iter()
        .filter(|b| b.name.contains("transfer duct"))
        .collect();
    assert_eq!(stacked.len(), 2);
    assert_ne!(stacked[0].from, stacked[1].from);
    assert_ne!(stacked[0].to, stacked[1].to);
    assert_eq!(
        p.drawing.positions[&stacked[0].from],
        p.drawing.positions[&stacked[1].from]
    );
    let riser = p
        .network
        .branches
        .iter()
        .find(|b| matches!(b.kind, BranchKind::Duct { riser: true, .. }))
        .unwrap();
    assert_ne!(
        p.network.node(riser.from).unwrap().floor,
        p.network.node(riser.to).unwrap().floor
    );
    let id = riser.id;
    p.drawing.bends.insert(id, [65.0, 14.0]);
    p.drawing.circuit.insert(id, [80.0, 0.0]);
    assert_eq!(r.snapshot, p.network);
    assert!(
        r.at(15.0).flows[p.network.branches.iter().position(|b| b.id == id).unwrap()].abs() > 0.001
    );
}

#[test]
fn save_replace_reload_and_csv() {
    let mut p = single();
    let temp = |extension: &str| -> PathBuf {
        std::env::temp_dir().join(format!("airframe-{}.{extension}", uuid::Uuid::new_v4()))
    };
    let json = temp("json");
    let csv = temp("csv");
    storage::save(&json, &p).unwrap();
    p.name = "Revised".into();
    storage::save(&json, &p).unwrap();
    assert_eq!(storage::load(&json).unwrap(), p);
    storage::export_csv(&csv, &run(&p)).unwrap();
    let text = std::fs::read_to_string(&csv).unwrap();
    assert!(text.starts_with("time_s,"));
    assert!(text.contains("fan_fraction"));
    assert!(text.lines().last().unwrap().starts_with("120,"));
    std::fs::remove_file(json).unwrap();
    std::fs::remove_file(csv).unwrap();
}

#[test]
fn invalid_topology_and_cancel_report_errors() {
    let mut p = single();
    assert!(matches!(
        simulate(&p.network, &p.scenarios[0], |_| false),
        Err(SolveError::Cancelled)
    ));
    let floor = p.network.floors[0].id;
    p.add_node("Unconnected", floor, NodeKind::Junction, [0., 0.]);
    assert!(matches!(
        simulate(&p.network, &p.scenarios[0], |_| true),
        Err(SolveError::Invalid(_))
    ));
}

#[test]
fn off_grid_events_are_recorded_at_exact_times() {
    let mut p = single();
    p.scenarios[0].events[0].time_s = 3.037;
    p.scenarios[0].events[1].time_s = 19.867;
    let r = run(&p);
    assert_eq!(r.at(3.037).time_s, 3.037);
    close(r.at(3.037).flows[1], 0.0, 1e-10);
    assert_eq!(r.at(19.867).time_s, 19.867);
    close(
        r.at(19.867).flows[1],
        (1.0 - (-(19.867_f64 - 3.037) / 5.0).exp()) / 6.0,
        2e-7,
    );
    assert!(r.samples.windows(2).all(|w| w[0].time_s < w[1].time_s));
}
