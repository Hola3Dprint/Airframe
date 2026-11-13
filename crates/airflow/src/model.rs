//! Physical connectivity and SI values are separate from drawing coordinates.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

pub const SCHEMA: u32 = 1;
pub const CFM: f64 = 0.000_471_947_443_2;
pub const M3_H: f64 = 3600.0;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Air {
    pub absolute_pa: f64,
    pub density: f64,
    pub viscosity: f64,
}
impl Default for Air {
    fn default() -> Self {
        Self {
            absolute_pa: 101325.0,
            density: 1.204,
            viscosity: 1.81e-5,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Floor {
    pub id: Uuid,
    pub name: String,
    pub elevation_m: f64,
    pub visible: bool,
    pub locked: bool,
}
impl Floor {
    pub fn new(name: &str, elevation_m: f64) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            elevation_m,
            visible: true,
            locked: false,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NodeKind {
    Room { volume_m3: f64, initial_pa: f64 },
    Boundary { gauge_pa: f64 },
    Atmosphere,
    Junction,
}
impl NodeKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Room { .. } => "Room",
            Self::Boundary { .. } => "Boundary",
            Self::Atmosphere => "Atmosphere",
            Self::Junction => "Junction",
        }
    }

    pub fn fixed_gauge_pa(self) -> Option<f64> {
        match self {
            Self::Boundary { gauge_pa } => Some(gauge_pa),
            Self::Atmosphere => Some(0.0),
            _ => None,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub id: Uuid,
    pub name: String,
    pub floor: Uuid,
    pub kind: NodeKind,
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum Section {
    Round { diameter_m: f64 },
    Rectangular { width_m: f64, height_m: f64 },
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Duct {
    pub section: Section,
    pub length_m: f64,
    pub roughness_m: f64,
    pub minor_k: f64,
}
impl Default for Duct {
    fn default() -> Self {
        Self {
            section: Section::Round { diameter_m: 0.25 },
            length_m: 5.0,
            roughness_m: 0.00009,
            minor_k: 1.0,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BranchKind {
    Opening { k: f64 },
    Duct { duct: Duct, riser: bool },
    FlowFan { design_m3_s: f64, tau_s: f64 },
    PressureFan { shutoff_pa: f64, k: f64, tau_s: f64 },
}
impl BranchKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Opening { .. } => "Opening",
            Self::Duct { riser: true, .. } => "Riser",
            Self::Duct { .. } => "Duct",
            Self::FlowFan { .. } => "Flow fan",
            Self::PressureFan { .. } => "Pressure fan",
        }
    }
    pub fn tau(self) -> Option<f64> {
        match self {
            Self::FlowFan { tau_s, .. } | Self::PressureFan { tau_s, .. } => Some(tau_s),
            _ => None,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum System {
    Supply,
    Return,
    Exhaust,
    Transfer,
    Outdoor,
    Unassigned,
}
impl System {
    pub const ALL: [Self; 6] = [
        Self::Supply,
        Self::Return,
        Self::Exhaust,
        Self::Transfer,
        Self::Outdoor,
        Self::Unassigned,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Supply => "Supply",
            Self::Return => "Return",
            Self::Exhaust => "Exhaust",
            Self::Transfer => "Transfer",
            Self::Outdoor => "Outdoor",
            Self::Unassigned => "Unassigned",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Branch {
    pub id: Uuid,
    pub name: String,
    pub from: Uuid,
    pub to: Uuid,
    pub system: System,
    pub start_elevation_m: f64,
    pub end_elevation_m: f64,
    pub kind: BranchKind,
    /// Steady speed fraction for a pressure fan. Omitted means full speed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fan_speed_fraction: Option<f64>,
    /// Rated (flow in m³/s, pressure rise in Pa) samples.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_fan_curve: Vec<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aperture: Option<Aperture>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    saved_fan_curve: Option<FanCurveSettings>,
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Aperture {
    pub area_m2: f64,
    pub discharge: f64,
    pub open_fraction: f64,
}
impl Aperture {
    pub fn resistance(self, density: f64) -> f64 {
        density / (2.0 * (self.area_m2 * self.discharge * self.open_fraction.max(1e-6)).powi(2))
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
struct FanCurveSettings {
    shutoff_pa: f64,
    k: f64,
    speed_fraction: f64,
}
impl Branch {
    pub fn enable_fan_curve(&mut self) {
        let BranchKind::FlowFan { design_m3_s, tau_s } = self.kind else {
            return;
        };
        if let Some(settings) = self.saved_fan_curve.take() {
            self.kind = BranchKind::PressureFan {
                shutoff_pa: settings.shutoff_pa,
                k: settings.k,
                tau_s,
            };
            self.fan_speed_fraction = Some(settings.speed_fraction);
            return;
        }
        let shutoff_pa = 75.0;
        let rated = if design_m3_s > 0.0 {
            design_m3_s
        } else {
            600.0 / M3_H
        };
        self.kind = BranchKind::PressureFan {
            shutoff_pa,
            k: shutoff_pa / rated.powi(2),
            tau_s,
        };
        self.fan_speed_fraction = Some(if design_m3_s > 0.0 { 1.0 } else { 0.0 });
    }
    pub fn use_fixed_airflow(&mut self, solved_flow: Option<f64>) {
        let BranchKind::PressureFan {
            shutoff_pa,
            k,
            tau_s,
        } = self.kind
        else {
            return;
        };
        let speed_fraction = self.fan_speed_fraction.unwrap_or(1.0);
        let free_flow = speed_fraction * self.fan_rated_flow().unwrap_or_default();
        self.saved_fan_curve = Some(FanCurveSettings {
            shutoff_pa,
            k,
            speed_fraction,
        });
        let design_m3_s = solved_flow.unwrap_or(free_flow).max(0.0);
        self.kind = BranchKind::FlowFan { design_m3_s, tau_s };
        self.fan_speed_fraction = None;
    }
    pub fn fan_rated_flow(&self) -> Option<f64> {
        let BranchKind::PressureFan { shutoff_pa, k, .. } = self.kind else {
            return None;
        };
        Some(self.custom_fan_curve.last().map_or_else(
            || crate::physics::rated_free_airflow(shutoff_pa, k),
            |point| point[0],
        ))
    }
}
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Network {
    pub air: Air,
    pub floors: Vec<Floor>,
    pub nodes: Vec<Node>,
    pub branches: Vec<Branch>,
}
impl Network {
    pub fn node(&self, id: Uuid) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }
    pub fn branch(&self, id: Uuid) -> Option<&Branch> {
        self.branches.iter().find(|b| b.id == id)
    }
    pub fn floor(&self, id: Uuid) -> Option<&Floor> {
        self.floors.iter().find(|f| f.id == id)
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub fan: Uuid,
    pub time_s: f64,
    /// Fraction of design flow (flow fan) or rated speed (pressure fan).
    pub command: f64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scenario {
    pub name: String,
    pub duration_s: f64,
    pub events: Vec<Event>,
}
impl Default for Scenario {
    fn default() -> Self {
        Self {
            name: "Startup / shutdown".into(),
            duration_s: 120.0,
            events: Vec::new(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Drawing {
    pub positions: BTreeMap<Uuid, [f32; 2]>,
    pub sizes: BTreeMap<Uuid, [f32; 2]>,
    pub circuit: BTreeMap<Uuid, [f32; 2]>,
    pub bends: BTreeMap<Uuid, [f32; 2]>,
    /// Explicit left/right handles for the start and end of a building wire.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ports: BTreeMap<Uuid, [Option<PortSide>; 2]>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub floor_colors: BTreeMap<Uuid, [u8; 3]>,
    /// Generated junction IDs mapped to the riser they belong to.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub riser_landings: BTreeMap<Uuid, Uuid>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub floor_plans: BTreeMap<Uuid, FloorPlan>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dismissed_connections: Vec<[Uuid; 2]>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub inline_nodes: BTreeMap<Uuid, Uuid>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub containers: BTreeMap<Uuid, Container>,
    pub grid: bool,
    pub snap: bool,
    pub background_opacity: f32,
    pub colors: [[u8; 3]; 6],
    pub light: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Container {
    pub name: String,
    pub floor: Uuid,
    pub center: [f32; 2],
    pub size: [f32; 2],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FloorPlan {
    /// Absolute image path, or relative to the project file.
    pub image: String,
    pub origin: [f32; 2],
    pub size: [f32; 2],
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortSide {
    Left,
    Right,
}
impl Default for Drawing {
    fn default() -> Self {
        Self {
            positions: BTreeMap::new(),
            sizes: BTreeMap::new(),
            circuit: BTreeMap::new(),
            bends: BTreeMap::new(),
            ports: BTreeMap::new(),
            floor_colors: BTreeMap::new(),
            riser_landings: BTreeMap::new(),
            floor_plans: BTreeMap::new(),
            dismissed_connections: Vec::new(),
            inline_nodes: BTreeMap::new(),
            containers: BTreeMap::new(),
            grid: true,
            snap: true,
            background_opacity: 0.38,
            colors: [
                [104, 171, 255],
                [79, 205, 188],
                [255, 165, 110],
                [191, 145, 255],
                [82, 219, 159],
                [157, 173, 192],
            ],
            light: false,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub schema: u32,
    pub name: String,
    pub notes: String,
    pub network: Network,
    pub drawing: Drawing,
    pub scenarios: Vec<Scenario>,
}
impl Project {
    /// Remove a series component and rejoin the two surviving sections.
    pub fn heal_inline(&mut self, id: Uuid) -> bool {
        let Some(component) = self.network.branch(id).cloned() else {
            return false;
        };
        let is_inline_port = |port| {
            self.drawing.inline_nodes.get(&port) == Some(&id)
                || self.network.node(port).is_some_and(|n| {
                    n.kind == NodeKind::Junction
                        && matches!(n.name.as_str(), "Inline inlet" | "Inline outlet")
                })
        };
        if !is_inline_port(component.from) || !is_inline_port(component.to) {
            return false;
        }
        let attached = |port| {
            self.network
                .branches
                .iter()
                .filter(move |b| b.id != id && (b.from == port || b.to == port))
        };
        let (mut incoming, mut outgoing) = (attached(component.from), attached(component.to));
        let (Some(before), Some(after)) = (incoming.next(), outgoing.next()) else {
            return false;
        };
        if incoming.next().is_some()
            || outgoing.next().is_some()
            || before.to != component.from
            || after.from != component.to
            || before.from == after.to
        {
            return false;
        }
        let Some(kind) = join_series(before.kind, after.kind) else {
            // Keep both loss laws when the two branches cannot be combined.
            return self.keep_series_connection(&component, after.id);
        };
        let aperture = joined_aperture(before.aperture, after.aperture);
        if aperture.is_none() && (before.aperture.is_some() || after.aperture.is_some()) {
            return self.keep_series_connection(&component, after.id);
        }
        let (head, tail, destination, elevation) =
            (before.id, after.id, after.to, after.end_elevation_m);
        let from_port = self.drawing.ports.get(&head).and_then(|p| p[0]);
        let to_port = self.drawing.ports.get(&tail).and_then(|p| p[1]);
        let b = self
            .network
            .branches
            .iter_mut()
            .find(|b| b.id == head)
            .unwrap();
        b.to = destination;
        b.end_elevation_m = elevation;
        b.kind = kind;
        b.aperture = aperture;
        self.drawing.ports.insert(head, [from_port, to_port]);
        self.remove(component.from);
        self.remove(component.to);
        true
    }

    pub fn heal_node(&mut self, id: Uuid) -> bool {
        if !self
            .network
            .node(id)
            .is_some_and(|n| n.kind.fixed_gauge_pa().is_none())
            || self.drawing.riser_landings.contains_key(&id)
        {
            return false;
        }
        let mut links = self
            .network
            .branches
            .iter()
            .filter(|b| b.from == id || b.to == id);
        let (Some(a), Some(b), None) = (links.next(), links.next(), links.next()) else {
            return false;
        };
        let (a, b) = if a.to == id { (a, b) } else { (b, a) };
        if a.to != id || b.from != id || a.from == b.to {
            return false;
        }
        let aperture = joined_aperture(a.aperture, b.aperture);
        let kind = join_series(a.kind, b.kind)
            .filter(|_| aperture.is_some() || (a.aperture.is_none() && b.aperture.is_none()));
        let Some(kind) = kind else {
            let node = self.network.nodes.iter_mut().find(|n| n.id == id).unwrap();
            node.kind = NodeKind::Junction;
            node.name = "Inline inlet".into();
            self.drawing.sizes.remove(&id);
            return true;
        };
        let (head, tail, destination, elevation) = (a.id, b.id, b.to, b.end_elevation_m);
        let ports = [
            self.drawing.ports.get(&head).and_then(|p| p[0]),
            self.drawing.ports.get(&tail).and_then(|p| p[1]),
        ];
        let branch = self
            .network
            .branches
            .iter_mut()
            .find(|branch| branch.id == head)
            .unwrap();
        branch.to = destination;
        branch.kind = kind;
        branch.aperture = aperture;
        branch.end_elevation_m = elevation;
        self.drawing.ports.insert(head, ports);
        self.remove(id);
        true
    }
    fn keep_series_connection(&mut self, component: &Branch, tail: Uuid) -> bool {
        self.network
            .branches
            .iter_mut()
            .find(|b| b.id == tail)
            .unwrap()
            .from = component.from;
        self.remove(component.id);
        self.remove(component.to);
        if let Some(n) = self
            .network
            .nodes
            .iter_mut()
            .find(|n| n.id == component.from)
        {
            n.name = "Inline inlet".into();
        }
        true
    }
    pub fn add_riser_landing(&mut self, riser: Uuid, floor: Uuid, pos: [f32; 2]) -> Uuid {
        let id = self.add_node("Riser landing", floor, NodeKind::Junction, pos);
        self.drawing.riser_landings.insert(id, riser);
        id
    }

    /// Insert a pressure node on a branch. A passive branch keeps the same
    /// combined loss law; a fan keeps its ID and gets a new downstream opening.
    pub fn split_branch(&mut self, id: Uuid, node: Uuid, fraction: f64) -> Option<Uuid> {
        let original = self.network.branch(id)?.clone();
        self.network.node(node)?;
        if node == original.from || node == original.to || !fraction.is_finite() {
            return None;
        }
        let f = fraction.clamp(0.05, 0.95);
        let scale = |kind, factor| match kind {
            BranchKind::Opening { k } => BranchKind::Opening { k: k * factor },
            BranchKind::Duct { mut duct, riser } => {
                duct.length_m *= factor;
                duct.minor_k *= factor;
                BranchKind::Duct { duct, riser }
            }
            fan => fan,
        };
        let (up, down) = if original.kind.tau().is_some() {
            (original.kind, BranchKind::Opening { k: 1800.0 })
        } else {
            (scale(original.kind, f), scale(original.kind, 1.0 - f))
        };
        let middle = original.start_elevation_m
            + f * (original.end_elevation_m - original.start_elevation_m);
        let tail = self.add_branch(
            &format!("{} · downstream", original.name),
            node,
            original.to,
            original.system,
            down,
        );
        let upstream = self.network.branches.iter_mut().find(|b| b.id == id)?;
        upstream.to = node;
        upstream.end_elevation_m = middle;
        upstream.kind = up;
        let scale_aperture = |factor: f64| {
            original.aperture.map(|mut a| {
                a.area_m2 /= factor.sqrt();
                a
            })
        };
        upstream.aperture = scale_aperture(f);
        let downstream = self.network.branches.last_mut()?;
        downstream.start_elevation_m = middle;
        downstream.end_elevation_m = original.end_elevation_m;
        downstream.aperture = scale_aperture(1.0 - f);
        self.drawing.bends.remove(&id);
        self.drawing.circuit.remove(&id);
        if let Some([from, to]) = self.drawing.ports.remove(&id) {
            self.drawing.ports.insert(id, [from, None]);
            self.drawing.ports.insert(tail, [None, to]);
        }
        Some(tail)
    }

    pub fn new(name: &str) -> Self {
        Self {
            schema: SCHEMA,
            name: name.into(),
            notes: String::new(),
            network: Network {
                floors: vec![Floor::new("Level 01", 0.0)],
                ..Network::default()
            },
            drawing: Drawing::default(),
            scenarios: vec![Scenario::default()],
        }
    }
    pub fn add_node(&mut self, name: &str, floor: Uuid, kind: NodeKind, pos: [f32; 2]) -> Uuid {
        let id = Uuid::new_v4();
        self.network.nodes.push(Node {
            id,
            name: name.into(),
            floor,
            kind,
        });
        self.drawing.positions.insert(id, pos);
        id
    }
    pub fn add_branch(
        &mut self,
        name: &str,
        from: Uuid,
        to: Uuid,
        system: System,
        kind: BranchKind,
    ) -> Uuid {
        let id = Uuid::new_v4();
        let elevation = |node| {
            self.network
                .node(node)
                .and_then(|n| self.network.floor(n.floor))
                .map_or(0.0, |f| f.elevation_m)
        };
        self.network.branches.push(Branch {
            id,
            name: name.into(),
            from,
            to,
            system,
            start_elevation_m: elevation(from),
            end_elevation_m: elevation(to),
            kind,
            fan_speed_fraction: None,
            custom_fan_curve: Vec::new(),
            aperture: None,
            saved_fan_curve: None,
        });
        id
    }
    pub fn remove(&mut self, id: Uuid) {
        let mut removed = vec![id];
        let mut affected = Vec::new();
        self.network.nodes.retain(|n| n.id != id);
        self.network.branches.retain(|b| {
            if b.id == id || b.from == id || b.to == id {
                removed.push(b.id);
                affected.extend([b.from, b.to]);
                false
            } else {
                true
            }
        });
        for node in affected {
            let generated = self.drawing.inline_nodes.contains_key(&node)
                || self.network.node(node).is_some_and(|n| {
                    n.kind == NodeKind::Junction
                        && matches!(n.name.as_str(), "Inline inlet" | "Inline outlet")
                });
            if generated
                && !self
                    .network
                    .branches
                    .iter()
                    .any(|b| b.from == node || b.to == node)
            {
                self.network.nodes.retain(|n| n.id != node);
                removed.push(node);
            }
        }
        self.drawing.riser_landings.retain(|node, riser| {
            if removed.contains(riser) {
                if !self
                    .network
                    .branches
                    .iter()
                    .any(|b| b.from == *node || b.to == *node)
                {
                    self.network.nodes.retain(|n| n.id != *node);
                    removed.push(*node);
                }
                false
            } else {
                !removed.contains(node)
            }
        });
        for scenario in &mut self.scenarios {
            scenario.events.retain(|e| !removed.contains(&e.fan));
        }
        self.drawing.inline_nodes.retain(|node, owner| {
            if removed.contains(node) {
                return false;
            }
            if removed.contains(owner) {
                // A port that still has a wire becomes a standalone junction.
                *owner = Uuid::nil();
            }
            true
        });
        for id in removed {
            self.drawing
                .dismissed_connections
                .retain(|pair| !pair.contains(&id));
            self.drawing.positions.remove(&id);
            self.drawing.sizes.remove(&id);
            self.drawing.circuit.remove(&id);
            self.drawing.bends.remove(&id);
            self.drawing.ports.remove(&id);
        }
    }
}

fn join_series(a: BranchKind, b: BranchKind) -> Option<BranchKind> {
    match (a, b) {
        (BranchKind::Opening { k: a }, BranchKind::Opening { k: b }) => {
            Some(BranchKind::Opening { k: a + b })
        }
        (
            BranchKind::Duct { duct: mut a, riser },
            BranchKind::Duct {
                duct: b,
                riser: other,
            },
        ) if a.section == b.section && a.roughness_m == b.roughness_m && riser == other => {
            a.length_m += b.length_m;
            a.minor_k += b.minor_k;
            Some(BranchKind::Duct { duct: a, riser })
        }
        (fan, BranchKind::Opening { k: 1800.0 }) if fan.tau().is_some() => Some(fan),
        _ => None,
    }
}

fn joined_aperture(a: Option<Aperture>, b: Option<Aperture>) -> Option<Aperture> {
    let (mut a, b) = (a?, b?);
    if a.discharge != b.discharge || a.open_fraction != b.open_fraction {
        return None;
    }
    a.area_m2 = 1.0 / (a.area_m2.recip().powi(2) + b.area_m2.recip().powi(2)).sqrt();
    Some(a)
}
