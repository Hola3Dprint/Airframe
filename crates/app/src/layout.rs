use crate::workbench::Workbench;
use airflow::model::{Branch, BranchKind, Network, Node, NodeKind, System};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use uuid::Uuid;

type Positions = BTreeMap<Uuid, [f32; 2]>;
type Focus = (Uuid, HashSet<Uuid>);
const CIRCUIT_COLUMN: f32 = 208.0;
const CIRCUIT_ROW: f32 = 154.0;

pub(crate) fn circuit_transfer(b: &Branch, from: &Node, to: &Node) -> bool {
    from.floor != to.floor
        || matches!(b.kind, BranchKind::Duct { riser: true, .. })
        || (matches!(from.kind, NodeKind::Room { .. })
            && matches!(to.kind, NodeKind::Room { .. })
            && matches!(b.kind, BranchKind::Opening { .. })
            && (b.aperture.is_some() || b.system == System::Transfer))
}

/// Find the room branches with a shared supply or extract node.
pub(crate) fn circuit_room_taps(net: &Network, supply: bool) -> HashSet<Uuid> {
    let mut rooms = HashMap::<Uuid, HashSet<Uuid>>::new();
    let mut branches = Vec::new();
    for b in &net.branches {
        let (header, room) = if supply {
            (b.from, b.to)
        } else {
            (b.to, b.from)
        };
        let (Some(header), Some(room)) = (net.node(header), net.node(room)) else {
            continue;
        };
        let system = if supply {
            matches!(
                b.system,
                System::Supply | System::Unassigned | System::Transfer
            )
        } else {
            matches!(b.system, System::Exhaust | System::Return)
        };
        if header.floor == room.floor
            && matches!(room.kind, NodeKind::Room { .. })
            && !matches!(header.kind, NodeKind::Room { .. })
            && system
            && !circuit_transfer(b, header, room)
        {
            rooms.entry(header.id).or_default().insert(room.id);
            branches.push((b.id, header.id));
        }
    }
    branches
        .into_iter()
        .filter(|(_, header)| rooms[header].len() > 1)
        .map(|(branch, _)| branch)
        .collect()
}

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
    let supply_drops = circuit_room_taps(net, true);
    let extract_taps = circuit_room_taps(net, false);
    let supplied_rooms: HashSet<_> = net
        .branches
        .iter()
        .filter(|b| supply_drops.contains(&b.id))
        .map(|b| b.to)
        .collect();
    let indices: HashMap<_, _> = net
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id, i))
        .collect();
    let mut outgoing = vec![Vec::new(); net.nodes.len()];
    for b in &net.branches {
        if let (Some(&a), Some(&z)) = (indices.get(&b.from), indices.get(&b.to)) {
            let transfer = circuit_transfer(b, &net.nodes[a], &net.nodes[z]);
            outgoing[a].push((
                z,
                transfer || extract_taps.contains(&b.id),
                supply_drops.contains(&b.id),
            ));
        }
    }
    // A boundary feeding the network stays at the supply end, including when
    // an extract branch also connects back to it.
    let sources: Vec<_> = net
        .nodes
        .iter()
        .zip(&outgoing)
        .map(|(n, next)| n.kind.fixed_gauge_pa().is_some() && !next.is_empty())
        .collect();
    let mut incoming = vec![0; net.nodes.len()];
    for &(z, ..) in outgoing.iter().flatten() {
        if !sources[z] {
            incoming[z] += 1;
        }
    }
    let mut pending: BTreeSet<_> = incoming
        .iter()
        .copied()
        .enumerate()
        .map(|(i, count)| (count, i))
        .collect();
    let mut order = Vec::with_capacity(net.nodes.len());
    let mut order_index = vec![0; net.nodes.len()];
    // Place merges after their feeds. For cycles, route the remaining edges
    // back to the already placed nodes.
    while let Some((_, a)) = pending.pop_first() {
        order_index[a] = order.len();
        order.push(a);
        for &(z, ..) in &outgoing[a] {
            if !sources[z] && pending.remove(&(incoming[z], z)) {
                incoming[z] -= 1;
                pending.insert((incoming[z], z));
            }
        }
    }
    let mut row = vec![None; net.nodes.len()];
    let mut column = vec![0; net.nodes.len()];
    let mut rows = 0_usize;
    for &a in &order {
        let lane = *row[a].get_or_insert_with(|| {
            let next = rows;
            rows += 1;
            next
        });
        // Continue one path along the current row. Doors, risers and
        // secondary branches join other rows at the current column.
        let mut continuation = None;
        let mut room_row = None;
        let mut room_column = column[a];
        for &(z, transfer, drop) in &outgoing[a] {
            if order_index[z] <= order_index[a] {
                continue;
            }
            if drop && row[z].is_none() {
                let drop_row = *room_row.get_or_insert_with(|| {
                    let next = rows;
                    rows += 1;
                    next
                });
                row[z] = Some(drop_row);
                room_column += 1;
                column[z] = column[z].max(room_column);
                room_column = column[z];
                continue;
            }
            if !transfer
                && !supplied_rooms.contains(&net.nodes[z].id)
                && !supplied_rooms.contains(&net.nodes[a].id)
                && continuation.is_none()
                && row[z].is_none()
            {
                row[z] = Some(lane);
                continuation = Some(z);
            }
            column[z] = column[z].max(column[a] + usize::from(!transfer));
        }
    }
    let middle_x = column.iter().copied().max().unwrap_or(0) as f32 * 0.5;
    let middle_y = rows.saturating_sub(1) as f32 * 0.5;
    net.nodes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            (
                n.id,
                [
                    (column[i] as f32 - middle_x) * CIRCUIT_COLUMN,
                    (row[i].unwrap_or(0) as f32 - middle_y) * CIRCUIT_ROW,
                ],
            )
        })
        .collect()
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
    use airflow::{model::Project, storage};

    #[test]
    fn circuit_keeps_parallel_rooms_on_separate_rows() {
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
            let system = if from == outside {
                System::Outdoor
            } else {
                System::Transfer
            };
            p.add_branch("Path", from, to, system, BranchKind::Opening { k: 1.0 });
        }
        let layout = circuit_layout(&p.network);
        assert_eq!(layout[&outside][1], layout[&a][1]);
        assert!(layout[&outside][0] < layout[&a][0]);
        assert_eq!(layout[&a][0], layout[&merge][0]);
        assert_ne!(layout[&a][1], layout[&b][1]);
        assert!(layout[&merge][1] > layout[&a][1] && layout[&merge][1] > layout[&b][1]);
    }

    #[test]
    fn shared_supply_places_rooms_side_by_side_below_the_header() {
        let mut p = Project::new("Parallel supply");
        let floor = p.network.floors[0].id;
        let room = NodeKind::Room {
            volume_m3: 100.0,
            initial_pa: 0.0,
        };
        let outside = p.add_node("Air intake", floor, NodeKind::Atmosphere, [0.0; 2]);
        let supply = p.add_node("Supply header", floor, NodeKind::Junction, [0.0; 2]);
        let a = p.add_node("Room A", floor, room, [0.0; 2]);
        let b = p.add_node("Room B", floor, room, [0.0; 2]);
        let extract = p.add_node("Extract", floor, NodeKind::Junction, [0.0; 2]);
        let duct = BranchKind::Duct {
            duct: Default::default(),
            riser: false,
        };
        for (from, to) in [(outside, supply), (supply, a), (supply, b)] {
            p.add_branch("Supply", from, to, System::Supply, duct);
        }
        for from in [a, b] {
            p.add_branch("Extract", from, extract, System::Exhaust, duct);
        }
        let drops = circuit_room_taps(&p.network, true);
        assert_eq!(
            drops,
            p.network.branches[1..3].iter().map(|b| b.id).collect()
        );
        assert_eq!(
            circuit_room_taps(&p.network, false),
            p.network.branches[3..].iter().map(|b| b.id).collect()
        );
        let layout = circuit_layout(&p.network);
        assert_eq!(layout[&outside][1], layout[&supply][1]);
        assert!(layout[&supply][1] < layout[&a][1]);
        assert_eq!(layout[&a][1], layout[&b][1]);
        assert!(layout[&supply][0] < layout[&a][0] && layout[&a][0] < layout[&b][0]);
        assert_eq!(layout[&b][0], layout[&extract][0]);
        assert!(layout[&b][1] < layout[&extract][1]);
        assert_eq!(p.network.branches.len(), 5);
    }

    #[test]
    fn two_floor_circuit_has_horizontal_runs_and_vertical_door_and_riser() {
        let p = storage::decode(&include_bytes!("../../../examples/two-floor.airflow.json")[..])
            .unwrap();
        let layout = circuit_layout(&p.network);
        let at = |name| {
            let node = p.network.nodes.iter().find(|n| n.name == name).unwrap();
            layout[&node.id]
        };
        let intake = at("Outdoor intake");
        let corridor = at("Main corridor");
        let first = at("Studio 01");
        let second = at("Studio 02");
        let third = at("Studio 03");
        let last = at("Studio 04");
        let extract = at("Outdoor discharge");
        assert!(intake[0] < corridor[0] && corridor[0] < first[0]);
        assert_eq!([intake[1], corridor[1]], [first[1]; 2]);
        assert_eq!([first[0], second[0]], [third[0]; 2]);
        assert!(first[1] < second[1] && second[1] < third[1]);
        assert!(third[0] < last[0] && last[0] < extract[0]);
        assert_eq!([last[1], extract[1]], [third[1]; 2]);
    }
}
