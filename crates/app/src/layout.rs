use crate::workbench::Workbench;
use airflow::model::{Network, Node, NodeKind};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use uuid::Uuid;

type Positions = BTreeMap<Uuid, [f32; 2]>;
type Focus = (Uuid, HashSet<Uuid>);
const CIRCUIT_COLUMN: f32 = 340.0;
const CIRCUIT_ROW: f32 = 150.0;

impl Workbench {
    pub(crate) fn inline_port(&self, node: &Node) -> bool {
        self.project.drawing.inline_nodes.contains_key(&node.id)
            || (node.kind == NodeKind::Junction
                && matches!(node.name.as_str(), "Inline inlet" | "Inline outlet"))
    }

    pub(crate) fn circuit_focus(&self) -> Option<Focus> {
        let net = &self.project.network;
        if !self.focus_circuit {
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
            for (column, ids) in [(-CIRCUIT_COLUMN, left), (CIRCUIT_COLUMN, right)] {
                let center = ids.len().saturating_sub(1) as f32 * 0.5;
                for (i, id) in ids.into_iter().enumerate() {
                    pos.insert(id, [column, (i as f32 - center) * CIRCUIT_ROW]);
                }
            }
            return pos;
        }
        let mut positions = circuit_layout(net);
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
            self.project.drawing.circuit.clear();
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

fn circuit_layout(net: &Network) -> Positions {
    let indices: HashMap<_, _> = net
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id, i))
        .collect();
    let mut outgoing = vec![Vec::new(); net.nodes.len()];
    for b in &net.branches {
        if let (Some(&a), Some(&z)) = (indices.get(&b.from), indices.get(&b.to)) {
            outgoing[a].push(z);
        }
    }
    // Fixed-pressure supplies start a stage even when a relief path returns to them.
    let sources: Vec<_> = net
        .nodes
        .iter()
        .zip(&outgoing)
        .map(|(n, next)| n.kind.fixed_gauge_pa().is_some() && !next.is_empty())
        .collect();
    let mut incoming = vec![0; net.nodes.len()];
    for &z in outgoing.iter().flatten().filter(|&&z| !sources[z]) {
        incoming[z] += 1;
    }
    let mut pending: BTreeSet<_> = incoming
        .iter()
        .copied()
        .enumerate()
        .map(|(i, count)| (count, i))
        .collect();
    let mut rank = vec![0; net.nodes.len()];
    let mut columns = BTreeMap::<usize, Vec<Uuid>>::new();
    // Process merges after their predecessors. If a loop remains, start with the
    // node with the fewest unresolved incoming paths and draw those paths back.
    while let Some((_, a)) = pending.pop_first() {
        columns.entry(rank[a]).or_default().push(net.nodes[a].id);
        for &z in &outgoing[a] {
            if !sources[z] && pending.remove(&(incoming[z], z)) {
                incoming[z] -= 1;
                rank[z] = rank[z].max(rank[a] + 1);
                pending.insert((incoming[z], z));
            }
        }
    }
    let mid = columns.len().saturating_sub(1) as f32 * 0.5;
    let mut result = Positions::new();
    for (column, ids) in columns {
        let center = ids.len().saturating_sub(1) as f32 * 0.5;
        for (row, id) in ids.into_iter().enumerate() {
            result.insert(
                id,
                [
                    (column as f32 - mid) * CIRCUIT_COLUMN,
                    (row as f32 - center) * CIRCUIT_ROW,
                ],
            );
        }
    }
    result
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

#[cfg(test)]
mod tests {
    use super::*;
    use airflow::model::{BranchKind, Project, System};

    #[test]
    fn circuit_keeps_parallel_rows_before_their_merge_and_routes_relief_back() {
        let mut p = Project::new("Circuit stages");
        let floor = p.network.floors[0].id;
        let room = NodeKind::Room {
            volume_m3: 100.0,
            initial_pa: 0.0,
        };
        let outside = p.add_node("Atmosphere", floor, NodeKind::Atmosphere, [0.0; 2]);
        let a = p.add_node("Zone 1A", floor, room, [0.0; 2]);
        let b = p.add_node("Zone 1B", floor, room, [0.0; 2]);
        let merge = p.add_node("Zone 2", floor, room, [0.0; 2]);
        for (from, to) in [
            (outside, a),
            (outside, b),
            (a, merge),
            (b, merge),
            (merge, outside),
        ] {
            p.add_branch(
                "Path",
                from,
                to,
                System::Transfer,
                BranchKind::Opening { k: 1.0 },
            );
        }
        let layout = circuit_layout(&p.network);
        assert!(layout[&outside][0] < layout[&a][0]);
        assert!(layout[&a][0] < layout[&merge][0]);
        assert_eq!(layout[&a][0], layout[&b][0]);
        assert_ne!(layout[&a][1], layout[&b][1]);
    }
}
