use crate::workbench::Workbench;
use airflow::model::{Network, Node, NodeKind};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use uuid::Uuid;

type Positions = BTreeMap<Uuid, [f32; 2]>;
type Focus = (Uuid, HashSet<Uuid>);

impl Workbench {
    pub(crate) fn inline_port(&self, node: &Node) -> bool {
        self.project.drawing.inline_nodes.contains_key(&node.id)
            || (node.kind == NodeKind::Junction
                && matches!(node.name.as_str(), "Inline inlet" | "Inline outlet"))
    }

    pub(crate) fn circuit_focus(&self) -> Option<Focus> {
        let net = &self.project.network;
        if !self.focus_circuit || net.nodes.len() < 12 {
            return None;
        }
        let room =
            |n: &&Node| n.floor == self.active_floor && matches!(n.kind, NodeKind::Room { .. });
        let center = self
            .selection
            .iter()
            .rev()
            .find_map(|id| net.node(*id).filter(room))
            .or_else(|| net.nodes.iter().find(room))?;
        let mut ids = HashSet::from([center.id]);
        for b in &net.branches {
            if b.from == center.id || b.to == center.id {
                ids.extend([b.from, b.to]);
            }
        }
        Some((center.id, ids))
    }

    pub(crate) fn circuit_positions(&self, focus: Option<&Focus>) -> Positions {
        let net = &self.project.network;
        if let Some((center, _)) = focus {
            let mut pos = Positions::from([(*center, [0.0, 0.0])]);
            let mut left = Vec::new();
            let mut right = Vec::new();
            for b in &net.branches {
                if b.to == *center && !left.contains(&b.from) {
                    left.push(b.from);
                }
                if b.from == *center && !right.contains(&b.to) {
                    right.push(b.to);
                }
            }
            right.retain(|id| !left.contains(id));
            for (column, ids) in [(-350.0, left), (350.0, right)] {
                let center = ids.len().saturating_sub(1) as f32 * 0.5;
                for (i, id) in ids.into_iter().enumerate() {
                    pos.insert(id, [column, (i as f32 - center) * 135.0]);
                }
            }
            return pos;
        }
        let mut positions = connected_layout(net, |_| true, 270.0, 125.0);
        positions.extend(
            self.project
                .drawing
                .circuit
                .iter()
                .map(|(&id, &xy)| (id, xy)),
        );
        positions
    }

    pub(crate) fn arrange(&mut self, circuit: bool) {
        let before = self.project.clone();
        let net = &self.project.network;
        if circuit {
            self.project.drawing.circuit = connected_layout(net, |_| true, 270.0, 125.0);
        } else if self.floor_editable(self.active_floor) {
            let sizes = &self.project.drawing.sizes;
            let mut spacing = [260.0_f32, 145.0_f32];
            for n in net.nodes.iter().filter(|n| n.floor == self.active_floor) {
                if let Some(size) = sizes.get(&n.id) {
                    spacing[0] = spacing[0].max(size[0] + 92.0);
                    spacing[1] = spacing[1].max(size[1] + 60.0);
                }
            }
            self.project.drawing.positions.extend(connected_layout(
                net,
                |n| n.floor == self.active_floor,
                spacing[0],
                spacing[1],
            ));
            for b in net.branches.iter().filter(|b| {
                net.node(b.from)
                    .is_some_and(|n| n.floor == self.active_floor)
                    && net.node(b.to).is_some_and(|n| n.floor == self.active_floor)
            }) {
                self.project.drawing.bends.remove(&b.id);
            }
        }
        self.checkpoint(before);
    }
}

fn connected_layout(net: &Network, show: impl Fn(&Node) -> bool, dx: f32, dy: f32) -> Positions {
    let nodes: Vec<_> = net.nodes.iter().filter(|n| show(n)).collect();
    let ids: HashSet<_> = nodes.iter().map(|n| n.id).collect();
    let mut neighbors = HashMap::<Uuid, Vec<Uuid>>::new();
    let mut incoming = HashSet::new();
    for b in &net.branches {
        if ids.contains(&b.from) && ids.contains(&b.to) {
            neighbors.entry(b.from).or_default().push(b.to);
            neighbors.entry(b.to).or_default().push(b.from);
            incoming.insert(b.to);
        }
    }
    let mut roots = nodes.clone();
    roots.sort_by_key(|n| (incoming.contains(&n.id), n.kind.fixed_gauge_pa().is_none()));
    let mut result = Positions::new();
    let mut seen = HashSet::new();
    let mut offset = 0.0;
    for root in roots {
        if !seen.insert(root.id) {
            continue;
        }
        let mut queue = VecDeque::from([(root.id, 0)]);
        let mut levels: Vec<Vec<Uuid>> = Vec::new();
        while let Some((id, rank)) = queue.pop_front() {
            if levels.len() == rank {
                levels.push(Vec::new());
            }
            levels[rank].push(id);
            for &adj in neighbors.get(&id).into_iter().flatten() {
                if seen.insert(adj) {
                    queue.push_back((adj, rank + 1));
                }
            }
        }
        let height = levels.iter().map(Vec::len).max().unwrap_or(1);
        let width = levels.len().saturating_sub(1) as f32;
        for (x, level) in levels.into_iter().enumerate() {
            let top = (height - level.len()) as f32 * 0.5;
            for (y, id) in level.into_iter().enumerate() {
                result.insert(
                    id,
                    [
                        (x as f32 - width * 0.5) * dx,
                        offset + (top + y as f32) * dy,
                    ],
                );
            }
        }
        offset += (height as f32 + 0.5) * dy;
    }
    let mid = (offset - 1.5 * dy).max(0.0) * 0.5;
    for xy in result.values_mut() {
        xy[1] -= mid;
    }
    result
}
