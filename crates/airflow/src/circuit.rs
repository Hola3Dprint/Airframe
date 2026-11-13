//! Circuit elements are derived from the physical model each time it changes.
use crate::{model::*, physics::capacitance};
use uuid::Uuid;

#[derive(Clone, Copy)]
pub enum Symbol {
    Capacitor,
    Ground,
    Junction,
    Resistor,
    ResistorInductor,
    CurrentSource,
    VoltageSourceResistor,
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
pub fn branch_element(b: &Branch) -> Element {
    let (symbol, equation, value) = match b.kind {
        BranchKind::Opening { k } => (
            Symbol::Resistor,
            "P₁ − P₂ = k · Q · |Q|",
            Some((k, "Pa·s²/m⁶")),
        ),
        BranchKind::Duct { .. } => (
            Symbol::ResistorInductor,
            "P₁ − P₂ = ΔP_loss + L · dQ/dt",
            None,
        ),
        BranchKind::FlowFan { design_m3_s, .. } => (
            Symbol::CurrentSource,
            "τ · dQ/dt = command · Qrated − Q",
            Some((design_m3_s, "m³/s")),
        ),
        BranchKind::PressureFan { shutoff_pa, .. } => (
            Symbol::VoltageSourceResistor,
            "P₁ − P₂ + P₀ · speed² = k · Q · |Q|",
            Some((shutoff_pa, "Pa")),
        ),
    };
    Element {
        origin: b.id,
        symbol,
        equation,
        value,
    }
}
