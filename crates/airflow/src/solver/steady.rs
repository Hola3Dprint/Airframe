//! Solve a single operating point: room and junction mass balance, branch losses,
//! and prescribed fan flows. No integration or simulation clock.
use super::{SolveError, jacobian_product, prepare, rhs};
use crate::{model::*, physics::*};
use nalgebra::{DMatrix, DVector};
use uuid::Uuid;

#[derive(Clone, Copy, Default, Debug)]
pub struct FlowBalance {
    pub inflow: f64,
    pub outflow: f64,
}
impl FlowBalance {
    pub fn net(self) -> f64 {
        self.inflow - self.outflow
    }
}

#[derive(Debug)]
pub struct OperatingPoint {
    pub snapshot: Network,
    /// Gauge pressures, in the same order as the network nodes.
    pub pressures: Vec<f64>,
    /// Signed volumetric flows, in the same order as the network branches.
    pub flows: Vec<f64>,
    pub balances: Vec<FlowBalance>,
    pub max_imbalance: f64,
    pub warnings: Vec<String>,
}
impl OperatingPoint {
    pub fn balance(&self, id: Uuid) -> Option<FlowBalance> {
        self.snapshot
            .nodes
            .iter()
            .position(|n| n.id == id)
            .map(|i| self.balances[i])
    }
    pub fn matches(&self, net: &Network) -> bool {
        self.snapshot.air == net.air
            && self.snapshot.nodes == net.nodes
            && self.snapshot.branches == net.branches
    }
}

/// Solve the nonlinear steady equations with a damped Newton iteration.
/// Returns an error for pressure islands, unbalanced nodes, or a failed iteration.
pub fn solve(
    net: &Network,
    mut keep_going: impl FnMut() -> bool,
) -> Result<OperatingPoint, SolveError> {
    if !keep_going() {
        return Err(SolveError::Cancelled);
    }
    let data = prepare(net, None)?;
    let n = net.nodes.len();
    let b = net.branches.len();
    let mut y = DVector::from_vec(data.initial.clone());
    let size = y.len();
    let mut command = vec![1.0; b];
    let mut known = y
        .iter()
        .take(n)
        .copied()
        .zip(&net.nodes)
        .filter_map(|(p, node)| node.kind.fixed_gauge_pa().map(|_| p));
    let mean = known.clone().sum::<f64>() / known.by_ref().count() as f64;
    let mut tolerance = vec![1e-6; size]; // Pa for pressure equations
    for (i, node) in net.nodes.iter().enumerate() {
        if node.kind.fixed_gauge_pa().is_none() {
            y[i] = mean;
            tolerance[i] = 1e-9; // m³/s for mass balance
        }
    }
    for (i, branch) in net.branches.iter().enumerate() {
        let (a, z) = data.ends[i];
        let dp = y[a] - y[z];
        y[n + i] = match branch.kind {
            BranchKind::Opening { k } => quadratic_flow(
                branch.aperture.map_or(k, |a| a.resistance(net.air.density)),
                dp,
            ),
            BranchKind::Duct { duct, .. } => duct_flow(duct, net.air, dp),
            BranchKind::FlowFan { design_m3_s, .. } => {
                tolerance[n + i] = 1e-9;
                design_m3_s
            }
            BranchKind::PressureFan { shutoff_pa, k, .. } => {
                let speed = data.speed[i].expect("pressure fan state");
                let fraction = branch.fan_speed_fraction.unwrap_or(1.0);
                if !(0.0..=1.0).contains(&fraction) {
                    return Err(SolveError::Invalid(format!(
                        "{} needs a fan speed between 0% and 100%.",
                        branch.name
                    )));
                }
                command[i] = fraction;
                y[speed] = fraction;
                tolerance[speed] = 1e-9;
                quadratic_flow(k, dp + shutoff_pa * fraction.powi(2))
            }
        };
    }

    let error = |message: &str| SolveError::Steady(message.into());
    let norm = |r: &DVector<f64>| {
        if !r.iter().all(|v| v.is_finite()) {
            return f64::INFINITY;
        }
        r.iter()
            .zip(&tolerance)
            .map(|(v, t)| (v / t).abs())
            .fold(0.0, f64::max)
    };
    let mut residual = DVector::zeros(size);
    let mut trial_residual = DVector::zeros(size);
    let mut basis = vec![0.0; size];
    for _ in 0..100 {
        if !keep_going() {
            return Err(SolveError::Cancelled);
        }
        rhs(net, &data, &command, y.as_slice(), residual.as_mut_slice());
        let initial = norm(&residual);
        if initial <= 1.0 && y.iter().all(|v| v.is_finite()) {
            let pressures = y.as_slice()[..n].to_vec();
            let flows = y.as_slice()[n..n + b].to_vec();
            let mut balances = vec![FlowBalance::default(); n];
            for (&q, &(a, z)) in flows.iter().zip(&data.ends) {
                let (from, to) = if q >= 0.0 { (a, z) } else { (z, a) };
                balances[from].outflow += q.abs();
                balances[to].inflow += q.abs();
            }
            let max_imbalance = net
                .nodes
                .iter()
                .zip(&balances)
                .filter(|(node, _)| node.kind.fixed_gauge_pa().is_none())
                .map(|(_, balance)| balance.net().abs())
                .fold(0.0, f64::max);
            let mut warnings = Vec::new();
            if pressures
                .iter()
                .any(|p| p.abs() > net.air.absolute_pa * 0.05)
            {
                warnings.push("Gauge pressure exceeds 5% of the absolute reference. Check the inputs and constant-density assumption.".into());
            }
            return Ok(OperatingPoint {
                snapshot: net.clone(),
                pressures,
                flows,
                balances,
                max_imbalance,
                warnings,
            });
        }
        if !initial.is_finite() {
            return Err(error(
                "Non-finite residual. Check dimensions, pressure and flow inputs.",
            ));
        }
        let mut jacobian = DMatrix::zeros(size, size);
        for j in 0..size {
            basis[j] = 1.0;
            jacobian_product(
                net,
                &data,
                y.as_slice(),
                &basis,
                jacobian.column_mut(j).as_mut_slice(),
            );
            basis[j] = 0.0;
        }
        let delta = jacobian.lu().solve(&(-&residual)).ok_or_else(|| {
            error("No unique pressure solution. Check boundary connections and branch resistances.")
        })?;
        let mut scale = 1.0;
        let mut accepted = false;
        for _ in 0..32 {
            let trial = &y + scale * &delta;
            rhs(
                net,
                &data,
                &command,
                trial.as_slice(),
                trial_residual.as_mut_slice(),
            );
            if norm(&trial_residual) < initial {
                y = trial;
                accepted = true;
                break;
            }
            scale *= 0.5;
        }
        if !accepted {
            return Err(error(
                "The flow balance did not converge. Check the branch sizes, fan flows and boundary pressures.",
            ));
        }
    }
    Err(error(
        "Reached the 100-iteration limit before the flow and pressure equations balanced.",
    ))
}
