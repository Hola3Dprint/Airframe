use airflow::{model::*, solver::steady::solve, storage};

fn fixture() -> Project {
    storage::decode(&include_bytes!("../../../examples/single-room.airflow.json")[..]).unwrap()
}
fn close(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{actual} vs {expected}, tolerance {tolerance}"
    );
}

#[test]
fn operating_point_uses_fixed_flows_and_no_timeline() {
    let mut p = fixture();
    p.scenarios.clear();
    p.network.nodes[1].kind = NodeKind::Room {
        volume_m3: 0.0,
        initial_pa: 100.0,
    };
    let mut result = solve(&p.network, || true).unwrap();
    close(result.pressures[1], -50.0, 1e-6);
    close(result.balances[1].inflow, 600.0 / M3_H, 1e-9);
    close(result.balances[1].outflow, 600.0 / M3_H, 1e-9);
    assert!(result.max_imbalance <= 1e-9);
    p.network.branches[1].kind = BranchKind::FlowFan {
        design_m3_s: 300.0 / M3_H,
        tau_s: 0.0,
    };
    result = solve(&p.network, || true).unwrap();
    close(result.pressures[1], -12.5, 1e-6);
    p.network.branches[1].kind = BranchKind::FlowFan {
        design_m3_s: 0.0,
        tau_s: 0.0,
    };
    result = solve(&p.network, || true).unwrap();
    close(result.pressures[1], 0.0, 1e-9);
    close(result.flows[1], 0.0, 1e-9);
}

#[test]
fn prescribed_flow_does_not_anchor_pressure() {
    let mut p = fixture();
    p.remove(p.network.nodes[0].id);
    let err = solve(&p.network, || true).unwrap_err();
    assert!(err.to_string().contains("fixed-pressure boundary"));
}

#[test]
fn atmosphere_is_a_zero_pressure_reference() {
    let mut p = fixture();
    p.network.nodes[0].kind = NodeKind::Atmosphere;
    p.network.nodes[2].kind = NodeKind::Atmosphere;
    let point = solve(&p.network, || true).unwrap();
    assert_eq!(point.pressures[0], 0.0);
    assert_eq!(point.pressures[2], 0.0);
    close(point.pressures[1], -50.0, 1e-6);
    assert!(matches!(
        airflow::circuit::node_element(&p.network.nodes[0], p.network.air).symbol,
        airflow::circuit::Symbol::Ground
    ));
    let bytes = serde_json::to_vec(&p).unwrap();
    let reopened = storage::decode(bytes.as_slice()).unwrap();
    assert_eq!(reopened.network.nodes[0].kind, NodeKind::Atmosphere);
}

#[test]
fn deleting_branches_removes_only_generated_orphan_nodes() {
    let mut p = Project::new("Delete inline fitting");
    let floor = p.network.floors[0].id;
    let room = p.add_node(
        "Room",
        floor,
        NodeKind::Room {
            volume_m3: 100.0,
            initial_pa: 0.0,
        },
        [0.0; 2],
    );
    let inlet = p.add_node("Inline inlet", floor, NodeKind::Junction, [0.0; 2]);
    let outlet = p.add_node("Renamed outlet", floor, NodeKind::Junction, [0.0; 2]);
    let atmosphere = p.add_node("Atmosphere", floor, NodeKind::Atmosphere, [0.0; 2]);
    let kind = BranchKind::Opening { k: 500.0 };
    let upstream = p.add_branch("Upstream", room, inlet, System::Transfer, kind);
    let fitting = p.add_branch("Fitting", inlet, outlet, System::Transfer, kind);
    let downstream = p.add_branch("Downstream", outlet, atmosphere, System::Transfer, kind);
    p.drawing.inline_nodes.insert(outlet, fitting);
    p.remove(upstream);
    p.remove(fitting);
    assert!(p.network.node(inlet).is_none());
    assert!(p.network.node(outlet).is_some());
    p.remove(downstream);
    assert!(p.network.node(outlet).is_none());
    assert!(!p.drawing.positions.contains_key(&outlet));
    assert!(p.network.node(room).is_some());
    assert!(p.network.node(atmosphere).is_some());
}

#[test]
fn splicing_passive_wire_preserves_loss_flow_and_existing_id() {
    let mut p = fixture();
    let reference = solve(&p.network, || true).unwrap();
    let old = p.network.branches[0].clone();
    let floor = p.network.floors[0].id;
    let node = p.add_node(
        "Inserted junction",
        floor,
        NodeKind::Junction,
        [-120.0, 10.0],
    );
    let downstream = p.split_branch(old.id, node, 0.25).unwrap();
    let a = p.network.branch(old.id).unwrap();
    let b = p.network.branch(downstream).unwrap();
    assert_eq!((a.from, a.to, b.from, b.to), (old.from, node, node, old.to));
    let result = solve(&p.network, || true).unwrap();
    close(result.pressures[1], reference.pressures[1], 1e-6);
    close(result.pressures[3], -12.5, 1e-6);
    close(result.flows[0], result.flows[2], 1e-9);
}

#[test]
fn reverse_flow_counts_as_inflow_at_the_correct_node() {
    let mut p = fixture();
    let inlet = &mut p.network.branches[0];
    (inlet.from, inlet.to) = (inlet.to, inlet.from);
    let r = solve(&p.network, || true).unwrap();
    close(r.flows[0], -600.0 / M3_H, 1e-9);
    close(r.balances[1].inflow, 600.0 / M3_H, 1e-9);
    close(r.balances[1].net(), 0.0, 1e-9);
}

#[test]
fn duct_additional_loss_follows_signed_airflow_and_survives_splitting() {
    let mut p = fixture();
    let duct = Duct {
        additional_loss_pa: 25.0,
        loss_airflow_m3_s: 600.0 / M3_H,
        ..Duct::default()
    };
    let air = p.network.air;
    let q = duct.loss_airflow_m3_s;
    let base = Duct {
        additional_loss_pa: 0.0,
        ..duct
    };
    let loss = |d, flow| airflow::physics::duct_loss(d, air, flow);
    close(loss(duct, q) - loss(base, q), 25.0, 1e-12);
    close(loss(duct, -q * 0.5) - loss(base, -q * 0.5), -6.25, 1e-12);
    close(loss(duct, 0.0), 0.0, 1e-12);

    let branch = &mut p.network.branches[0];
    branch.kind = BranchKind::Duct { duct, riser: false };
    let id = branch.id;
    let point = solve(&p.network, || true).unwrap();
    close(point.pressures[1], -loss(duct, q), 1e-6);
    let node = p.add_node(
        "Split",
        p.network.floors[0].id,
        NodeKind::Junction,
        [0.0; 2],
    );
    p.split_branch(id, node, 0.25).unwrap();
    let reopened = storage::decode(serde_json::to_vec(&p).unwrap().as_slice()).unwrap();
    let split = solve(&reopened.network, || true).unwrap();
    close(split.pressures[1], point.pressures[1], 1e-6);
}

#[test]
fn deleting_an_inserted_component_restores_the_opening() {
    let mut p = fixture();
    let original = p.network.branches[0].clone();
    let aperture = Aperture {
        area_m2: 0.12,
        discharge: 0.61,
        open_fraction: 0.8,
    };
    p.network.branches[0].aperture = Some(aperture);
    let expected = solve(&p.network, || true).unwrap();
    let floor = p.network.floors[0].id;
    let inlet = p.add_node("Inline inlet", floor, NodeKind::Junction, [0.0; 2]);
    let outlet = p.add_node("Inline outlet", floor, NodeKind::Junction, [0.0; 2]);
    let downstream = p.split_branch(original.id, inlet, 0.28).unwrap();
    p.network
        .branches
        .iter_mut()
        .find(|b| b.id == downstream)
        .unwrap()
        .from = outlet;
    let inserted = p.add_branch(
        "Added fitting",
        inlet,
        outlet,
        original.system,
        BranchKind::Opening { k: 500.0 },
    );
    p.drawing
        .inline_nodes
        .extend([(inlet, inserted), (outlet, inserted)]);
    assert!(p.heal_inline(inserted));
    assert_eq!(p.network.branches.len(), 2);
    assert_eq!(p.network.nodes.len(), 3);
    let rejoined = p.network.branch(original.id).unwrap();
    assert_eq!((rejoined.from, rejoined.to), (original.from, original.to));
    close(rejoined.aperture.unwrap().area_m2, aperture.area_m2, 1e-12);
    let actual = solve(&p.network, || true).unwrap();
    close(actual.pressures[1], expected.pressures[1], 1e-6);
}

#[test]
fn custom_fan_samples_set_the_loaded_operating_point() {
    let mut p = fixture();
    p.network.branches[1].enable_fan_curve();
    p.network.branches[1].custom_fan_curve = vec![[0.0, 120.0], [0.2, 72.0], [0.4, 0.0]];
    let result = solve(&p.network, || true).unwrap();
    close(result.flows[1], 0.2, 1e-9);
    close(result.pressures[1], -72.0, 1e-6);
    p.network.branches[1].fan_speed_fraction = Some(0.5);
    let result = solve(&p.network, || true).unwrap();
    close(result.flows[1], 0.1, 1e-9);
    close(result.pressures[1], -18.0, 1e-6);
    p.network.branches[1].use_fixed_airflow(Some(result.flows[1]));
    p.network.branches[1].enable_fan_curve();
    assert_eq!(p.network.branches[1].custom_fan_curve.len(), 3);
}

#[test]
fn two_floor_equilibrium_balances_rooms_and_junctions() {
    let p =
        storage::decode(&include_bytes!("../../../examples/two-floor.airflow.json")[..]).unwrap();
    let r = solve(&p.network, || true).unwrap();
    assert!(r.max_imbalance <= 1e-9);
    assert!(r.pressures.iter().all(|p| p.is_finite()));
    assert!(r.snapshot.branches.iter().enumerate().any(|(i, b)| {
        matches!(b.kind, BranchKind::Duct { riser: true, .. }) && r.flows[i].abs() > 0.01
    }));
}

#[test]
fn fan_curve_speed_changes_the_loaded_operating_point() {
    let mut p = fixture();
    let fan = &mut p.network.branches[1];
    fan.kind = BranchKind::PressureFan {
        shutoff_pa: 75.0,
        k: 1200.0,
        tau_s: 5.0,
    };
    // The fixture's inlet adds 1800 Q² Pa, so Q = s × sqrt(75 / 3000).
    let full = solve(&p.network, || true).unwrap();
    close(full.flows[1], (75.0_f64 / 3000.0).sqrt(), 1e-8);
    close(full.pressures[1], -45.0, 1e-6);
    p.network.branches[1].fan_speed_fraction = Some(0.5);
    let half = solve(&p.network, || true).unwrap();
    close(half.flows[1], full.flows[1] / 2.0, 1e-8);
    close(half.pressures[1], full.pressures[1] / 4.0, 1e-6);
    assert!(half.flows[1] < 0.125); // Half-speed airflow at zero pressure is 0.125 m³/s.
    let saved = serde_json::to_vec(&p).unwrap();
    let reopened = storage::decode(saved.as_slice()).unwrap();
    assert_eq!(reopened.network.branches[1].fan_speed_fraction, Some(0.5));
    let mut switched = reopened;
    switched.network.branches[1].use_fixed_airflow(Some(half.flows[1]));
    close(
        solve(&switched.network, || true).unwrap().flows[1],
        half.flows[1],
        1e-9,
    );
    let saved = serde_json::to_vec(&switched).unwrap();
    let mut reopened = storage::decode(saved.as_slice()).unwrap();
    reopened.network.branches[1].enable_fan_curve();
    assert_eq!(reopened.network.branches[1], p.network.branches[1]);
    p.network.branches[1].fan_speed_fraction = Some(0.0);
    close(solve(&p.network, || true).unwrap().flows[1], 0.0, 1e-9);
    p.network.branches[1].fan_speed_fraction = Some(1.01);
    assert!(solve(&p.network, || true).is_err());
}
