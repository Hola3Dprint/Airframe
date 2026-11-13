//! Circuit elements are derived from the physical model each time it changes.
use crate::{model::*, physics};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Symbol {
    Capacitor,
    Ground,
    Junction,
    Resistor,
    Inductor,
    CurrentSource,
    VoltageSource,
}
impl Symbol {
    pub fn label(self) -> &'static str {
        match self {
            Self::Capacitor => "Capacitor",
            Self::Ground => "Pressure reference",
            Self::Junction => "Junction",
            Self::Resistor => "Resistor",
            Self::Inductor => "Inductor",
            Self::CurrentSource => "Current source",
            Self::VoltageSource => "Voltage source",
        }
    }
}
pub struct Element {
    pub origin: Uuid,
    pub symbol: Symbol,
    pub equation: &'static str,
    pub value: Option<(f64, &'static str)>,
}
pub fn node_element(n: &Node, air: Air) -> Element {
    let (symbol, equation, value) = match n.kind {
        NodeKind::Room { volume_m3, .. } => (
            Symbol::Capacitor,
            "C · dP/dt = ΣQin − ΣQout",
            Some((physics::capacitance(volume_m3, air), "m³/Pa")),
        ),
        NodeKind::Boundary { gauge_pa } => (
            Symbol::Ground,
            "P = prescribed gauge pressure",
            Some((gauge_pa, "Pa")),
        ),
        NodeKind::Atmosphere => (Symbol::Ground, "P = 0 Pa (gauge)", Some((0.0, "Pa"))),
        NodeKind::Junction => (Symbol::Junction, "ΣQin − ΣQout = 0", None),
    };
    Element {
        origin: n.id,
        symbol,
        equation,
        value,
    }
}
pub fn branch_elements(b: &Branch, air: Air) -> impl Iterator<Item = Element> {
    let element = |symbol, equation, value| Element {
        origin: b.id,
        symbol,
        equation,
        value,
    };
    let resistor = |equation, value| element(Symbol::Resistor, equation, value);
    let (first, second) = match b.kind {
        BranchKind::Opening { k } => (
            resistor(
                "ΔP = k · Q · √(Q² + ε²)",
                Some((
                    b.aperture.map_or(k, |a| a.resistance(air.density)),
                    "Pa·s²/m⁶",
                )),
            ),
            None,
        ),
        BranchKind::Duct { duct, .. } => (
            resistor("ΔP = duct pressure loss", None),
            Some(element(
                Symbol::Inductor,
                "ΔP = L · dQ/dt",
                Some((physics::inertance(duct, air), "Pa·s²/m³")),
            )),
        ),
        BranchKind::FlowFan { design_m3_s, .. } => (
            element(
                Symbol::CurrentSource,
                "τ · dQ/dt = command · Qrated − Q",
                Some((design_m3_s, "m³/s")),
            ),
            None,
        ),
        BranchKind::PressureFan { shutoff_pa, k, .. } => (
            element(
                Symbol::VoltageSource,
                "Pressure rise at zero airflow = P₀ · speed²",
                Some((
                    b.custom_fan_curve.first().map_or(shutoff_pa, |p| p[1]),
                    "Pa",
                )),
            ),
            Some(resistor(
                if b.custom_fan_curve.is_empty() {
                    "ΔP = k · Q · √(Q² + ε²)"
                } else {
                    "ΔP = shutoff rise − fan curve rise"
                },
                b.custom_fan_curve.is_empty().then_some((k, "Pa·s²/m⁶")),
            )),
        ),
    };
    std::iter::once(first).chain(second)
}

/// Pressure loss of the circuit resistor alone. For a pressure fan, the
/// series source contributes the shutoff rise at the same speed.
pub fn resistor_loss(branch: &Branch, air: Air, speed: f64, flow: f64) -> Option<f64> {
    match branch.kind {
        BranchKind::Duct { duct, .. } => Some(physics::duct_loss(duct, air, flow)),
        BranchKind::Opening { k } => {
            let k = branch.aperture.map_or(k, |a| a.resistance(air.density));
            Some(physics::opening_loss(k, flow))
        }
        BranchKind::PressureFan { .. } => Some(
            physics::pressure_fan_rise(branch, speed, 0.0)
                - physics::pressure_fan_rise(branch, speed, flow),
        ),
        BranchKind::FlowFan { .. } => None,
    }
}
