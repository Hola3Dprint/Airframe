//! Assemble one mass-matrix system from the network, then integrate each event interval.
pub mod steady;

use crate::{model::*, physics::*};
use diffsol::{NalgebraLU, NalgebraMat, OdeBuilder, OdeSolverMethod, OdeSolverStopReason};
use petgraph::{graph::UnGraph, visit::Dfs};
use std::collections::{HashMap, HashSet};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SolveError {
    #[error("{0}")]
    Invalid(String),
    #[error("Solver at {time:.4} s: {message}")]
    Numerical { time: f64, message: String },
    #[error("Steady airflow: {0}")]
    Steady(String),
    #[error("Simulation cancelled")]
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct Sample {
    pub time_s: f64,
    pub pressures: Vec<f64>,
    pub flows: Vec<f64>,
    pub fan_fraction: Vec<Option<f64>>,
}
pub struct Run {
    pub snapshot: Network,
    pub scenario: Scenario,
    pub samples: Vec<Sample>,
    pub warnings: Vec<String>,
    pub max_junction_residual: f64,
    pub steps: usize,
}
impl Run {
    pub fn at(&self, t: f64) -> &Sample {
        let i = self.samples.partition_point(|s| s.time_s < t);
        if i == 0 {
            return &self.samples[0];
        }
        if i == self.samples.len() || t - self.samples[i - 1].time_s <= self.samples[i].time_s - t {
            &self.samples[i - 1]
        } else {
            &self.samples[i]
        }
    }
}
struct SystemData {
    ends: Vec<(usize, usize)>,
    speed: Vec<Option<usize>>,
    mass: Vec<f64>,
    initial: Vec<f64>,
}

fn prepare(net: &Network, scenario: Option<&Scenario>) -> Result<SystemData, SolveError> {
    let invalid = |s: &str| SolveError::Invalid(s.into());
    let positive = |x: f64| x.is_finite() && x > 0.0;
    if !positive(net.air.absolute_pa) || !positive(net.air.density) || !positive(net.air.viscosity)
    {
        return Err(invalid(
            "Air pressure, density and viscosity must be positive.",
        ));
    }
    if scenario.is_some_and(|s| !positive(s.duration_s) || s.duration_s > 86_400.0) {
        return Err(invalid(
            "Simulation duration must be above 0 and at most 86,400 s.",
        ));
    }
    if net.nodes.is_empty() || net.branches.is_empty() {
        return Err(invalid(
            "Connect at least two pressure nodes with a branch.",
        ));
    }
    let mut ids = HashSet::new();
    for f in &net.floors {
        if !ids.insert(f.id) || !f.elevation_m.is_finite() {
            return Err(invalid("Floor IDs must be unique and elevations finite."));
        }
    }
    let mut index = HashMap::new();
    let mut graph = UnGraph::<(), ()>::new_undirected();
    let mut graph_nodes = Vec::new();
    let mut mass = Vec::new();
    let mut initial = Vec::new();
    let mut anchors = Vec::new();
    let mut connected = vec![false; net.nodes.len()];
    for (i, node) in net.nodes.iter().enumerate() {
        if !ids.insert(node.id) || net.floor(node.floor).is_none() {
            return Err(invalid("Each node needs a unique ID and a valid floor."));
        }
        index.insert(node.id, i);
        let g = graph.add_node(());
        graph_nodes.push(g);
        let (m, p) = match node.kind {
            NodeKind::Room { .. } if scenario.is_none() => (0.0, 0.0),
            NodeKind::Room {
                volume_m3,
                initial_pa,
            } if positive(volume_m3)
                && initial_pa.is_finite()
                && net.air.absolute_pa + initial_pa > 0.0 =>
            {
                (capacitance(volume_m3, net.air), initial_pa)
            }
            NodeKind::Boundary { gauge_pa }
                if gauge_pa.is_finite() && net.air.absolute_pa + gauge_pa > 0.0 =>
            {
                (0.0, gauge_pa)
            }
            NodeKind::Junction | NodeKind::Atmosphere => (0.0, 0.0),
            _ => {
                return Err(SolveError::Invalid(format!(
                    "{} has an invalid volume or pressure.",
                    node.name
                )));
            }
        };
        if node.kind.fixed_gauge_pa().is_some()
            || (scenario.is_some() && matches!(node.kind, NodeKind::Room { .. }))
        {
            anchors.push(g);
        }
        mass.push(m);
        initial.push(p);
    }
    let mut ends = Vec::new();
    let mut speed = vec![None; net.branches.len()];
    let mut fans = HashSet::new();
    for branch in &net.branches {
        let (Some(&a), Some(&b)) = (index.get(&branch.from), index.get(&branch.to)) else {
            return Err(SolveError::Invalid(format!(
                "{} has a missing endpoint.",
                branch.name
            )));
        };
        if !ids.insert(branch.id)
            || a == b
            || !branch.start_elevation_m.is_finite()
            || !branch.end_elevation_m.is_finite()
        {
            return Err(SolveError::Invalid(format!(
                "{} needs a unique ID, two different endpoints and finite elevations.",
                branch.name
            )));
        }
        let valid = match branch.kind {
            BranchKind::Opening { k } => branch.aperture.map_or_else(
                || positive(k),
                |a| {
                    positive(a.area_m2)
                        && (0.0..=1.0).contains(&a.open_fraction)
                        && positive(a.discharge)
                        && a.discharge <= 1.0
                        && positive(a.resistance(net.air.density))
                },
            ),
            BranchKind::Duct { duct, .. } => {
                let (area, diameter) = area_and_diameter(duct.section);
                let dimensions = match duct.section {
                    Section::Round { diameter_m } => positive(diameter_m),
                    Section::Rectangular { width_m, height_m } => {
                        positive(width_m) && positive(height_m)
                    }
                };
                dimensions
                    && positive(area)
                    && positive(diameter)
                    && positive(duct.length_m)
                    && duct.roughness_m.is_finite()
                    && (0.0..diameter).contains(&duct.roughness_m)
                    && duct.minor_k.is_finite()
                    && duct.minor_k >= 0.0
                    && duct.additional_loss_pa.is_finite()
                    && duct.additional_loss_pa >= 0.0
                    && positive(duct.loss_airflow_m3_s)
            }
            BranchKind::FlowFan { design_m3_s, tau_s } => {
                design_m3_s.is_finite()
                    && design_m3_s >= 0.0
                    && (scenario.is_none() || positive(tau_s))
            }
            BranchKind::PressureFan {
                shutoff_pa,
                k,
                tau_s,
            } => positive(shutoff_pa) && positive(k) && (scenario.is_none() || positive(tau_s)),
        };
        if !valid
            || (matches!(branch.kind, BranchKind::PressureFan { .. })
                && !branch.custom_fan_curve.is_empty()
                && !valid_fan_curve(&branch.custom_fan_curve))
        {
            return Err(SolveError::Invalid(format!(
                "{} has invalid physical parameters.",
                branch.name
            )));
        }
        if branch.kind.tau().is_some() {
            fans.insert(branch.id);
        }
        if !matches!(branch.kind, BranchKind::FlowFan { .. }) {
            graph.add_edge(graph_nodes[a], graph_nodes[b], ());
        }
        connected[a] = true;
        connected[b] = true;
        ends.push((a, b));
        mass.push(if let BranchKind::FlowFan { tau_s, .. } = branch.kind {
            tau_s
        } else {
            0.0
        });
        initial.push(0.0);
    }
    let mut reachable = HashSet::new();
    for root in anchors {
        if reachable.contains(&root) {
            continue;
        }
        let mut dfs = Dfs::new(&graph, root);
        while let Some(node) = dfs.next(&graph) {
            reachable.insert(node);
        }
    }
    for (i, node) in net.nodes.iter().enumerate() {
        if !connected[i] {
            return Err(SolveError::Invalid(format!(
                "{} is unconnected.",
                node.name
            )));
        }
        if !reachable.contains(&graph_nodes[i]) {
            let message = if scenario.is_some() {
                format!(
                    "{} needs a pressure-dependent path to a room or fixed boundary.",
                    node.name
                )
            } else {
                format!(
                    "{} needs a duct or opening path to a fixed-pressure boundary. A prescribed-flow fan sets flow but does not set pressure.",
                    node.name
                )
            };
            return Err(SolveError::Invalid(message));
        }
    }
    for (i, branch) in net.branches.iter().enumerate() {
        if let BranchKind::PressureFan { tau_s, .. } = branch.kind {
            speed[i] = Some(initial.len());
            mass.push(tau_s);
            initial.push(0.0);
        }
    }
    let mut times = HashSet::new();
    for e in scenario.into_iter().flat_map(|s| &s.events) {
        if !fans.contains(&e.fan)
            || !e.time_s.is_finite()
            || !(0.0..=scenario.expect("event scenario").duration_s).contains(&e.time_s)
            || !e.command.is_finite()
            || !(0.0..=1.0).contains(&e.command)
            || !times.insert((
                e.fan,
                if e.time_s == 0.0 {
                    0
                } else {
                    e.time_s.to_bits()
                },
            ))
        {
            return Err(invalid(
                "Each fan event must name a fan, have a unique time for that fan within the run, and have a command from 0 to 100%.",
            ));
        }
    }
    Ok(SystemData {
        ends,
        speed,
        mass,
        initial,
    })
}

fn rhs(net: &Network, data: &SystemData, command: &[f64], y: &[f64], out: &mut [f64]) {
    let n = net.nodes.len();
    out.fill(0.0);
    for (i, branch) in net.branches.iter().enumerate() {
        let (a, b) = data.ends[i];
        let q = y[n + i];
        let dp = y[a] - y[b];
        out[a] -= q;
        out[b] += q;
        out[n + i] = match branch.kind {
            BranchKind::Opening { k } => {
                dp - opening_loss(
                    branch.aperture.map_or(k, |a| a.resistance(net.air.density)),
                    q,
                )
            }
            BranchKind::Duct { duct, .. } => dp - duct_loss(duct, net.air, q),
            BranchKind::FlowFan { design_m3_s, .. } => command[i] * design_m3_s - q,
            BranchKind::PressureFan { .. } => {
                let s = data.speed[i].expect("pressure fan state");
                out[s] = command[i] - y[s];
                dp + pressure_fan_rise(branch, y[s], q)
            }
        };
    }
    for (i, node) in net.nodes.iter().enumerate() {
        if let Some(gauge_pa) = node.kind.fixed_gauge_pa() {
            out[i] = gauge_pa - y[i];
        }
    }
}

fn jacobian_product(net: &Network, data: &SystemData, y: &[f64], v: &[f64], out: &mut [f64]) {
    let n = net.nodes.len();
    out.fill(0.0);
    for (i, branch) in net.branches.iter().enumerate() {
        let (a, b) = data.ends[i];
        let q = y[n + i];
        let dq = v[n + i];
        let dp = v[a] - v[b];
        out[a] -= dq;
        out[b] += dq;
        let h = q.hypot(LOW_FLOW);
        let slope = |k: f64| k * (h + q * (q / h));
        out[n + i] = match branch.kind {
            BranchKind::Opening { k } => {
                dp - slope(branch.aperture.map_or(k, |a| a.resistance(net.air.density))) * dq
            }
            BranchKind::Duct { duct, .. } => {
                let eps = 1e-7 * (1.0 + q.abs());
                let derivative = (duct_loss(duct, net.air, q + eps)
                    - duct_loss(duct, net.air, q - eps))
                    / (2.0 * eps);
                dp - derivative * dq
            }
            BranchKind::FlowFan { .. } => -dq,
            BranchKind::PressureFan { shutoff_pa, k, .. } => {
                let s = data.speed[i].expect("pressure fan state");
                out[s] = -v[s];
                if branch.custom_fan_curve.is_empty() {
                    dp + 2.0 * shutoff_pa * y[s] * v[s] - slope(k) * dq
                } else {
                    let eps = 1e-7;
                    let speed = y[s];
                    let dq_slope = (pressure_fan_rise(branch, speed, q + eps)
                        - pressure_fan_rise(branch, speed, q - eps))
                        / (2.0 * eps);
                    let ds_slope = (pressure_fan_rise(branch, speed + eps, q)
                        - pressure_fan_rise(branch, speed - eps, q))
                        / (2.0 * eps);
                    dp + dq_slope * dq + ds_slope * v[s]
                }
            }
        };
    }
    for (i, node) in net.nodes.iter().enumerate() {
        if node.kind.fixed_gauge_pa().is_some() {
            out[i] = -v[i];
        }
    }
}

fn sample(net: &Network, data: &SystemData, t: f64, y: &[f64]) -> Sample {
    let n = net.nodes.len();
    Sample {
        time_s: t,
        pressures: y[..n].to_vec(),
        flows: y[n..n + net.branches.len()].to_vec(),
        fan_fraction: net
            .branches
            .iter()
            .enumerate()
            .map(|(i, b)| match b.kind {
                BranchKind::FlowFan { design_m3_s, .. } => Some(if design_m3_s == 0.0 {
                    0.0
                } else {
                    y[n + i] / design_m3_s
                }),
                BranchKind::PressureFan { .. } => data.speed[i].map(|s| y[s]),
                _ => None,
            })
            .collect(),
    }
}

/// The callback receives progress in [0, 1]; return false to cancel.
pub fn simulate(
    net: &Network,
    scenario: &Scenario,
    mut progress: impl FnMut(f64) -> bool,
) -> Result<Run, SolveError> {
    let data = prepare(net, Some(scenario))?;
    let mut events = scenario.events.clone();
    events.sort_by(|a, b| a.time_s.total_cmp(&b.time_s));
    let mut cmd = vec![0.0; net.branches.len()];
    let mut event = 0;
    let mut t = 0.0;
    let mut y = data.initial.clone();
    let mut samples = Vec::new();
    let mut output_tick = 1;
    let mut steps = 0;
    let output_dt = (scenario.duration_s / 1200.0).max(0.02);
    let n = net.nodes.len();
    let anchors = net
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| node.kind != NodeKind::Junction);
    let mean = anchors.clone().map(|(i, _)| y[i]).sum::<f64>() / anchors.count() as f64;
    for (i, node) in net.nodes.iter().enumerate() {
        if node.kind == NodeKind::Junction {
            y[i] = mean;
        }
    }
    for (i, branch) in net.branches.iter().enumerate() {
        let (a, b) = data.ends[i];
        let dp = y[a] - y[b];
        y[n + i] = match branch.kind {
            BranchKind::Opening { k } | BranchKind::PressureFan { k, .. } => quadratic_flow(
                branch.aperture.map_or(k, |a| a.resistance(net.air.density)),
                dp,
            ),
            BranchKind::Duct { duct, .. } => duct_flow(duct, net.air, dp),
            BranchKind::FlowFan { .. } => 0.0,
        };
    }
    let tolerances: Vec<_> = (0..y.len())
        .map(|i| if i < n { 1e-6 } else { 1e-9 })
        .collect();
    while t < scenario.duration_s {
        if !progress(t / scenario.duration_s) {
            return Err(SolveError::Cancelled);
        }
        while event < events.len() && events[event].time_s <= t {
            let e = &events[event];
            let i = net
                .branches
                .iter()
                .position(|b| b.id == e.fan)
                .expect("validated fan");
            cmd[i] = e.command;
            event += 1;
        }
        let end = events.get(event).map_or(scenario.duration_s, |e| e.time_s);
        let numerical = |time: f64, message: String| SolveError::Numerical { time, message };
        let problem = OdeBuilder::<NalgebraMat<f64>>::new()
            .t0(t)
            .h0(1e-5_f64.min(end - t))
            .rtol(1e-7)
            .atol(tolerances.clone())
            .init(|_, _, out| out.copy_from_slice(&y), y.len())
            .mass(|v, _, _, beta, out| {
                for (i, m) in data.mass.iter().enumerate() {
                    out[i] = m * v[i] + beta * out[i];
                }
            })
            .rhs_implicit(
                |state, _, _, out| rhs(net, &data, &cmd, state, out),
                |state, _, _, v, out| jacobian_product(net, &data, state, v, out),
            )
            .build()
            .map_err(|e| numerical(t, e.to_string()))?;
        let mut solver = problem
            .bdf::<NalgebraLU<f64>>()
            .map_err(|e| numerical(t, e.to_string()))?;
        if samples.is_empty() {
            samples.push(sample(net, &data, t, solver.state().y.batch_as_slice(0)));
        }
        solver
            .set_stop_time(end)
            .map_err(|e| numerical(t, e.to_string()))?;
        loop {
            let reason = solver
                .step()
                .map_err(|e| numerical(solver.state().t, e.to_string()))?;
            steps += 1;
            let now = solver.state().t;
            if !now.is_finite() || steps > 250_000 {
                return Err(numerical(now, "Step limit or non-finite time.".into()));
            }
            if !progress((now / scenario.duration_s).min(1.0)) {
                return Err(SolveError::Cancelled);
            }
            while (output_tick as f64 * output_dt) <= now
                && (output_tick as f64 * output_dt) < end - 1e-10
            {
                let at = output_tick as f64 * output_dt;
                let state = solver
                    .interpolate(at)
                    .map_err(|e| numerical(at, e.to_string()))?;
                samples.push(sample(net, &data, at, state.batch_as_slice(0)));
                output_tick += 1;
            }
            if reason == OdeSolverStopReason::TstopReached {
                break;
            }
        }
        let next = solver.state().y.batch_as_slice(0).to_vec();
        samples.push(sample(net, &data, end, &next));
        while (output_tick as f64 * output_dt) <= end + 1e-10 {
            output_tick += 1;
        }
        drop(solver);
        drop(problem);
        y = next;
        t = end;
    }
    let mut warnings = Vec::new();
    let mut max_junction_residual: f64 = 0.0;
    for s in &samples {
        if s.pressures.iter().chain(&s.flows).any(|v| !v.is_finite()) {
            return Err(SolveError::Numerical {
                time: s.time_s,
                message: "Non-finite solution.".into(),
            });
        }
        let mut balance = vec![0.0; n];
        for (&(a, b), &q) in data.ends.iter().zip(&s.flows) {
            balance[a] -= q;
            balance[b] += q;
        }
        for (i, node) in net.nodes.iter().enumerate() {
            if node.kind == NodeKind::Junction {
                max_junction_residual = max_junction_residual.max(balance[i].abs());
            }
        }
    }
    if max_junction_residual > 1e-6 {
        return Err(SolveError::Invalid(format!(
            "Junction imbalance {:.3e} m³/s exceeds 1e-6 m³/s.",
            max_junction_residual
        )));
    }
    if samples.iter().any(|s| {
        s.pressures
            .iter()
            .any(|p| p.abs() > net.air.absolute_pa * 0.05)
    }) {
        warnings.push("Gauge pressure exceeded 5% of absolute reference pressure; the constant-density model may be inaccurate.".into());
    }
    Ok(Run {
        snapshot: net.clone(),
        scenario: scenario.clone(),
        samples,
        warnings,
        max_junction_residual,
        steps,
    })
}
