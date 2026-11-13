//! Circuit elements are derived from the physical model each time it changes.
use crate::{model::*, physics::capacitance};
use uuid::Uuid;

#[derive(Clone, Copy)]
pub enum Symbol {
    Capacitor,
    Ground,
    Junction,
    Resistor,
    Inductor,
    CurrentSource,
    VoltageSource,
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
            Some((capacitance(volume_m3, air), "m³/Pa")),
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
                "P₁ − P₂ = k · Q · |Q|",
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
                Some((crate::physics::inertance(duct, air), "Pa·s²/m³")),
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
                "ΔP = fan pressure rise",
                Some((
                    b.custom_fan_curve.first().map_or(shutoff_pa, |p| p[1]),
                    "Pa",
                )),
            ),
            Some(resistor(
                "ΔP = k · Q · |Q|",
                b.custom_fan_curve.is_empty().then_some((k, "Pa·s²/m⁶")),
            )),
        ),
    };
    std::iter::once(first).chain(second)
}
