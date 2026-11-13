use crate::{
    component_help,
    layout::{circuit_room_taps, circuit_transfer},
    style::*,
    workbench::{LibraryDrag, Overlay, Part, View, Workbench, primary_press_in},
};
use airflow::{
    circuit::{self, Symbol},
    model::*,
};
use eframe::egui::*;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

#[derive(Clone, Copy)]
pub struct Camera {
    pan: Vec2,
    zoom: f32,
}
impl Default for Camera {
    fn default() -> Self {
        Self {
            pan: Vec2::ZERO,
            zoom: 1.0,
        }
    }
}
impl Camera {
    pub fn center_on(&mut self, position: [f32; 2]) {
        self.pan = -vec2(position[0], position[1]) * self.zoom;
    }
    fn screen(self, rect: Rect, xy: [f32; 2]) -> Pos2 {
        rect.center() + self.pan + vec2(xy[0], xy[1]) * self.zoom
    }
    fn world(self, rect: Rect, at: Pos2, snap: bool) -> [f32; 2] {
        let v = (at - rect.center() - self.pan) / self.zoom;
        let grid = |x: f32| if snap { (x / 20.0).round() * 20.0 } else { x };
        [grid(v.x), grid(v.y)]
    }
}
struct NodeDraw {
    id: Uuid,
    rect: Rect,
    opacity: f32,
    inline: bool,
}
impl NodeDraw {
    fn inline_hit(&self, at: Pos2) -> bool {
        self.inline && self.rect.center().distance(at) <= 10.0
    }

    fn port(&self, side: PortSide) -> Pos2 {
        if self.inline {
            self.rect.center()
        } else {
            side_port(self.rect, side)
        }
    }
}
struct Wire {
    id: Uuid,
    points: [Pos2; 3],
    opacity: f32,
    badge: Rect,
    color: Color32,
    inline_card: bool,
    circuit: Option<CircuitWire>,
}

struct CircuitWire {
    points: Vec<Pos2>,
    axis: Vec2,
}

impl CircuitWire {
    fn symbol_center(&self, badge: Rect, index: usize, count: usize, zoom: f32) -> Pos2 {
        badge.center() + self.axis * ((index as f32 - (count - 1) as f32 * 0.5) * 44.0 * zoom)
    }
    fn symbol_bounds(&self, center: Pos2, zoom: f32) -> Rect {
        let size = vec2((31.0 * zoom).max(10.0), (22.0 * zoom).max(10.0));
        Rect::from_center_size(center, if self.axis.y == 0.0 { size } else { size.yx() })
    }

    fn flow_arrow(&self, badge: Rect, reversed: bool) -> Option<(Pos2, Vec2)> {
        let points = &self.points;
        let pair = if reversed {
            [points[1], points[0]]
        } else {
            [points[points.len() - 2], points[points.len() - 1]]
        };
        let (at, direction) = (pair[0].lerp(pair[1], 0.55), pair[1] - pair[0]);
        if direction.length() >= 16.0 && !badge.expand(7.0).contains(at) {
            return Some((at, direction.normalized()));
        }
        // Straight vertical links reach the destination through the center segment.
        let band = |p: Pos2| {
            let signed = (p - badge.center()).dot(self.axis);
            signed.abs()
                - if self.axis.y == 0.0 {
                    badge.width()
                } else {
                    badge.height()
                } * 0.5
        };
        let end = if reversed {
            points[0]
        } else {
            points[points.len() - 1]
        };
        let sign = if reversed { -1.0 } else { 1.0 };
        let at = end - self.axis * sign * (band(end) * 0.5);
        (band(end) >= 18.0
            && self
                .points
                .windows(2)
                .any(|p| segment_distance(at, p[0], p[1]) <= 0.5))
        .then_some((at, self.axis * sign))
    }
}

pub(crate) struct MoveDrag {
    pub before: Project,
    start: Pos2,
    anchor: Vec2,
    nodes: HashMap<Uuid, [f32; 2]>,
    bends: HashMap<Uuid, [f32; 2]>,
    containers: Vec<Uuid>,
}

impl Wire {
    fn path(&self) -> &[Pos2] {
        self.circuit
            .as_ref()
            .map_or(&self.points[..], |route| &route.points)
    }

    fn distance(&self, at: Pos2) -> f32 {
        self.path()
            .windows(2)
            .map(|p| segment_distance(at, p[0], p[1]))
            .fold(f32::INFINITY, f32::min)
    }

    fn port_hit(&self, port: Pos2, pointer: Pos2) -> bool {
        let radius = (self.points[0].distance(self.points[2]) * 0.30).min(8.0);
        pointer.distance(port) <= radius
    }
}

fn inline_card_rect(from: Pos2, to: Pos2, zoom: f32, size: Option<[f32; 2]>) -> Rect {
    let span = (to - from).abs();
    if let Some([width, height]) = size {
        return Rect::from_center_size(from.lerp(to, 0.5), vec2(width, height) * zoom);
    }
    let size = if span.x >= span.y {
        vec2(span.x, (span.y + 14.0 * zoom).max(56.0 * zoom))
    } else {
        vec2((span.x + 16.0 * zoom).max(108.0 * zoom), span.y)
    };
    Rect::from_center_size(from.lerp(to, 0.5), size)
}

#[derive(Clone, Copy)]
pub(crate) struct WireSplice {
    pub wire: Uuid,
    pub fraction: f64,
    pub points: [Pos2; 3],
}

impl WireSplice {
    fn lengths(self) -> (f32, f32) {
        let first = self.points[0].distance(self.points[1]);
        (first, first + self.points[1].distance(self.points[2]))
    }

    pub fn point(self, fraction: f64) -> [f32; 2] {
        let (first, total) = self.lengths();
        let at = fraction.clamp(0.0, 1.0) as f32 * total;
        let [start, bend, end] = self.points;
        let point = if at < first {
            start.lerp(bend, at / first)
        } else if total > first {
            bend.lerp(end, (at - first) / (total - first))
        } else {
            end
        };
        [point.x, point.y]
    }

    pub fn inline_span(self) -> (f64, f64) {
        let (_, length) = self.lengths();
        let half = f64::from(60.0 / length.max(1.0)).min(0.36);
        let margin = 0.04;
        let center = self.fraction.clamp(margin + half, 1.0 - margin - half);
        (center - half, center + half)
    }

    pub fn bend(self, from: f64, to: f64) -> Option<[f32; 2]> {
        let (first, total) = self.lengths();
        let fraction = f64::from(first / total);
        let [start, bend, end] = self.points;
        (from < fraction && fraction < to && segment_distance(bend, start, end) > 0.01)
            .then_some([bend.x, bend.y])
    }
}

impl Workbench {
    pub fn split_canvas(&mut self, ui: &mut Ui, p: Palette) {
        let available = ui.available_rect_before_wrap();
        let divider_id = ui.make_persistent_id("level-pane-divider");
        let middle = |ratio| available.left() + available.width() * ratio;
        let mut x = middle(self.split_ratio);
        let divider_rect = |x| {
            Rect::from_min_max(
                pos2(x - 6.0, available.top()),
                pos2(x + 6.0, available.bottom()),
            )
        };
        let divider = ui.interact(divider_rect(x), divider_id, Sense::drag());
        if divider.dragged()
            && let Some(at) = divider.interact_pointer_pos()
        {
            let limit = (200.0 / available.width()).clamp(0.22, 0.46);
            self.split_ratio =
                ((at.x - available.left()) / available.width()).clamp(limit, 1.0 - limit);
            x = middle(self.split_ratio);
        }
        divider
            .clone()
            .on_hover_and_drag_cursor(CursorIcon::ResizeHorizontal)
            .on_hover_text("Drag to resize level panes");
        let mut left = available;
        left.max.x = x - 6.0;
        let mut right = available;
        right.min.x = x + 6.0;
        for (index, bounds) in [left, right].into_iter().enumerate() {
            if self.view != View::Building {
                break;
            }
            let mut child = ui.new_child(
                UiBuilder::new()
                    .id_salt(("split-column", index))
                    .max_rect(bounds)
                    .layout(Layout::top_down(Align::Min)),
            );
            child.set_clip_rect(bounds.intersect(ui.clip_rect()));
            let column = &mut child;
            let right = index == 1;
            let inside = column
                .ctx()
                .pointer_interact_pos()
                .is_some_and(|at| column.max_rect().contains(at));
            let active_gesture = self.dragging.is_some()
                || self.resize.is_some()
                || self.port_drag.is_some()
                || self.box_select.is_some()
                || self.container_start.is_some();
            let activate = !active_gesture
                && (primary_press_in(column, bounds).is_some()
                    || (inside
                        && column.input(|i| i.pointer.button_released(PointerButton::Primary))
                        && DragAndDrop::has_payload_of_type::<LibraryDrag>(column.ctx())));
            if activate && right != self.secondary_active {
                if let Some((floor, camera)) = &mut self.split {
                    std::mem::swap(&mut self.active_floor, floor);
                    std::mem::swap(&mut self.cameras[0], camera);
                }
                self.secondary_active = right;
                self.selection.retain(|id| {
                    self.project
                        .network
                        .node(*id)
                        .is_some_and(|n| n.floor == self.active_floor)
                        || self
                            .project
                            .network
                            .branch(*id)
                            .and_then(|b| self.project.network.node(b.from))
                            .is_some_and(|n| n.floor == self.active_floor)
                });
                self.from = None;
                self.reconnect = None;
            }
            let active = right == self.secondary_active;
            if !active && let Some((floor, camera)) = &mut self.split {
                std::mem::swap(&mut self.active_floor, floor);
                std::mem::swap(&mut self.cameras[0], camera);
            }
            self.split_rendering = true;
            column.push_id(("floor-pane", right), |ui| {
                ui.horizontal(|ui| {
                    ui.label(tag(
                        if active { "ACTIVE LEVEL" } else { "LEVEL" },
                        if active { p.accent } else { p.muted },
                    ));
                    let prev = self.active_floor;
                    ComboBox::from_id_salt("pane-floor")
                        .selected_text(
                            self.project
                                .network
                                .floor(prev)
                                .map_or("Level", |f| f.name.as_str()),
                        )
                        .show_ui(ui, |ui| {
                            for floor in &self.project.network.floors {
                                ui.selectable_value(&mut self.active_floor, floor.id, &floor.name);
                            }
                        });
                    if prev != self.active_floor {
                        ui.ctx()
                            .data_mut(|d| d.insert_temp(Id::new(("split-fit", right)), true));
                        if active {
                            self.selection.clear();
                            self.from = None;
                            self.reconnect = None;
                        }
                    }
                });
                self.canvas_pane(ui, p, Some((right, active)));
            });
            self.split_rendering = false;
            if !active && let Some((floor, camera)) = &mut self.split {
                std::mem::swap(&mut self.active_floor, floor);
                std::mem::swap(&mut self.cameras[0], camera);
            }
        }
        let bar = divider_rect(x);
        ui.painter().rect_filled(bar, 0, p.raised);
        ui.painter().line_segment(
            [pos2(bar.left(), bar.top()), pos2(bar.left(), bar.bottom())],
            Stroke::new(1.0_f32, p.edge),
        );
        ui.painter().line_segment(
            [
                pos2(bar.right(), bar.top()),
                pos2(bar.right(), bar.bottom()),
            ],
            Stroke::new(1.0_f32, p.edge),
        );
        let color = if divider.hovered() || divider.dragged() {
            p.accent
        } else {
            p.muted
        };
        for offset in [-2.0, 2.0] {
            let center = bar.center() + vec2(offset, 0.0);
            ui.painter().line_segment(
                [center - vec2(0.0, 10.0), center + vec2(0.0, 10.0)],
                Stroke::new(1.4_f32, color),
            );
        }
        ui.advance_cursor_after_rect(available);
    }

    pub fn canvas(&mut self, ui: &mut Ui, p: Palette) {
        self.canvas_pane(ui, p, None);
    }

    fn canvas_pane(&mut self, ui: &mut Ui, p: Palette, pane: Option<(bool, bool)>) {
        let circuit_mode = self.view == View::Circuit;
        let cam_idx = usize::from(circuit_mode);
        let plan_layout = !circuit_mode
            && self.plan_visible
            && self
                .project
                .drawing
                .floor_plans
                .contains_key(&self.active_floor);
        Frame::new()
            .fill(p.surface)
            .inner_margin(Margin::symmetric(14, 9))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if pane.is_none() {
                        ui.vertical(|ui| {
                            eyebrow(
                                ui,
                                if circuit_mode {
                                    "02  /  PRESSURE–VOLTAGE ANALOGY"
                                } else {
                                    "01  /  NETWORK EDITOR"
                                },
                                p,
                            );
                            heading(ui, &self.project.name);
                        });
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .button("Fit")
                            .on_hover_text("Fit visible nodes")
                            .clicked()
                        {
                            if let Some((right, _)) = pane {
                                ui.ctx().data_mut(|d| {
                                    d.insert_temp(Id::new(("split-fit", right)), true)
                                });
                            } else {
                                self.fit_next(cam_idx, ui.ctx());
                            }
                        }
                        if ui
                            .button("Arrange")
                            .on_hover_text(if plan_layout {
                                "Arrange in Circuit to preserve floor plan alignment"
                            } else {
                                "Arrange connected components"
                            })
                            .clicked()
                        {
                            if plan_layout {
                                self.view = View::Circuit;
                                self.fit_next(1, ui.ctx());
                            }
                            self.arrange(circuit_mode || plan_layout);
                            if let Some((right, _)) = pane {
                                ui.ctx().data_mut(|d| {
                                    d.insert_temp(Id::new(("split-fit", right)), true)
                                });
                            } else {
                                self.fit_next(cam_idx, ui.ctx());
                            }
                        }
                        if circuit_mode {
                            if ui.button("Equations").clicked() {
                                self.equations = true;
                            }
                            let text = if self.focus_circuit {
                                "Room focus"
                            } else {
                                "Full network"
                            };
                            if ui
                                .button(text)
                                .on_hover_text("Switch between one room and the full circuit")
                                .clicked()
                            {
                                self.focus_circuit = !self.focus_circuit;
                                self.fit_next(1, ui.ctx());
                            }
                        } else {
                            if pane.is_none_or(|(_, active)| active)
                                && ui
                                    .add_enabled(
                                        self.floor_editable(self.active_floor),
                                        Button::new("Container").selected(self.draw_container),
                                    )
                                    .on_hover_text(
                                        "Draw a container around items to move them together",
                                    )
                                    .clicked()
                            {
                                self.draw_container = !self.draw_container;
                                self.tool = None;
                                self.from = None;
                                self.reconnect = None;
                                self.placing_preset = None;
                            }
                            if self
                                .project
                                .drawing
                                .floor_plans
                                .contains_key(&self.active_floor)
                            {
                                ui.toggle_value(&mut self.plan_visible, "Floor plan");
                            }
                            ui.menu_button("Layers", |ui| {
                                let prev = self.project.clone();
                                if ui.button("Import floor plan image…").clicked() {
                                    self.import_plan(ui.ctx());
                                    if let Some((right, _)) = pane {
                                        ui.ctx().data_mut(|d| {
                                            d.insert_temp(Id::new(("split-fit", right)), true)
                                        });
                                    }
                                    ui.close();
                                }
                                ui.checkbox(&mut self.project.drawing.grid, "Grid");
                                ui.checkbox(&mut self.project.drawing.snap, "Snap to grid");
                                ui.checkbox(&mut self.route, "Highlight connected route");
                                ui.checkbox(&mut self.isolate, "Isolate selected system");
                                ui.checkbox(&mut self.profile, "Show branch elevation profile");
                                ui.checkbox(
                                    &mut self.show_suggestions,
                                    "Show connection suggestions",
                                );
                                if self
                                    .project
                                    .drawing
                                    .floor_plans
                                    .contains_key(&self.active_floor)
                                {
                                    ui.add(
                                        Slider::new(&mut self.plan_opacity, 0.1..=1.0)
                                            .text("Plan opacity"),
                                    );
                                    if ui.button("Remove floor plan").clicked()
                                        && self.floor_editable(self.active_floor)
                                    {
                                        self.project.drawing.floor_plans.remove(&self.active_floor);
                                        self.plan_texture.remove(&self.active_floor);
                                        ui.close();
                                    }
                                }
                                ui.checkbox(&mut self.elevation_filter, "Filter branch elevation");
                                if self.elevation_filter {
                                    crate::inspector::number(
                                        ui,
                                        "Center",
                                        &mut self.elevation_m,
                                        "m",
                                        0.1,
                                    );
                                    crate::inspector::number(
                                        ui,
                                        "± Range",
                                        &mut self.elevation_band,
                                        "m",
                                        0.1,
                                    );
                                }
                                self.checkpoint(prev);
                            });
                            if pane.is_none() {
                                ui.toggle_value(&mut self.multi_select, "Multi-select")
                                    .on_hover_text("Toggle selections with a click, or hold Shift");
                            }
                        }
                        if pane.is_none() {
                            ComboBox::from_id_salt("overlay")
                                .selected_text(self.overlay.label())
                                .show_ui(ui, |ui| {
                                    for m in Overlay::ALL {
                                        ui.selectable_value(&mut self.overlay, m, m.label());
                                    }
                                });
                        }
                    });
                });
            });
        let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let rect = response.rect;
        let ctx = ui.ctx();
        let editable_pane = pane.is_none_or(|(_, active)| active);
        let on_canvas = ui.rect_contains_pointer(rect);
        let global_fit = Id::new(("fit-camera", cam_idx));
        let fit_id = pane.map_or(global_fit, |(right, _)| Id::new(("split-fit", right)));
        if ctx.data_mut(|d| {
            let local = d.remove_temp::<bool>(fit_id).unwrap_or(false);
            let global = editable_pane
                && fit_id != global_fit
                && d.remove_temp::<bool>(global_fit).unwrap_or(false);
            local || global
        }) {
            self.fit(cam_idx, rect);
        }
        let active = response.hovered() || response.dragged();
        if active {
            let (scroll, zoom, pointer, delta, space) = ctx.input(|i| {
                (
                    i.raw_scroll_delta.y,
                    i.zoom_delta(),
                    i.pointer.hover_pos(),
                    i.pointer.delta(),
                    i.key_down(Key::Space),
                )
            });
            let camera = &mut self.cameras[cam_idx];
            if let Some(at) = pointer
                && (scroll != 0.0 || zoom != 1.0)
            {
                let factor = if zoom != 1.0 {
                    zoom
                } else {
                    (scroll * 0.002).exp()
                };
                let old = camera.zoom;
                camera.zoom = (old * factor).clamp(0.25, 10.0);
                camera.pan =
                    at - rect.center() - (at - rect.center() - camera.pan) * (camera.zoom / old);
            }
            if response.dragged_by(PointerButton::Middle)
                || response.dragged_by(PointerButton::Secondary)
                || (space && response.dragged_by(PointerButton::Primary))
            {
                camera.pan += delta;
            }
        }
        let camera = self.cameras[cam_idx];
        painter.rect_filled(rect, 0, p.canvas);
        if !circuit_mode && self.plan_visible {
            self.paint_floor_plan(ctx, &painter, rect, camera);
        }
        if !circuit_mode && self.project.drawing.grid {
            grid(&painter, rect, camera, p);
        }
        let net = &self.project.network;
        let coords = |id, index| self.world_pos(id, index, circuit_mode);
        let focus = circuit_mode.then(|| self.circuit_focus()).flatten();
        let layout = circuit_mode.then(|| self.circuit_positions(focus.as_ref()));
        let visible = |floor| {
            net.floor(floor).is_some_and(|f| {
                f.visible
                    && (circuit_mode
                        || (pane.is_none() && self.all_floors)
                        || floor == self.active_floor)
            })
        };
        let containers: Vec<_> = self
            .project
            .drawing
            .containers
            .iter()
            .filter(|(_, group)| !circuit_mode && visible(group.floor))
            .map(|(&id, group)| {
                (
                    id,
                    Rect::from_center_size(
                        camera.screen(rect, group.center),
                        vec2(group.size[0], group.size[1]) * camera.zoom,
                    ),
                )
            })
            .collect();
        let group_header = |bounds: Rect| {
            Rect::from_min_max(
                bounds.min,
                pos2(bounds.right(), (bounds.top() + 23.0).min(bounds.bottom())),
            )
        };
        for &(id, bounds) in &containers {
            let group = &self.project.drawing.containers[&id];
            let opacity = if group.floor == self.active_floor {
                1.0
            } else {
                self.project.drawing.background_opacity
            };
            let color = if self.selection.contains(&id) {
                p.accent
            } else {
                p.muted
            };
            painter.rect(
                bounds,
                8,
                p.accent.gamma_multiply(0.07 * opacity),
                Stroke::new(1.0_f32, color.gamma_multiply(0.65 * opacity)),
                StrokeKind::Inside,
            );
            let header = group_header(bounds);
            painter.rect_filled(header, 7, p.accent.gamma_multiply(0.15 * opacity));
            let name = &group.name;
            painter
                .with_clip_rect(header.shrink(3.0).intersect(rect))
                .text(
                    header.left_center() + vec2(7.0, 0.0),
                    Align2::LEFT_CENTER,
                    name,
                    FontId::proportional(12.0),
                    tint(color, opacity),
                );
        }
        let mut nodes = Vec::new();
        for (i, n) in net.nodes.iter().enumerate() {
            if !visible(n.floor) || focus.as_ref().is_some_and(|(_, ids)| !ids.contains(&n.id)) {
                continue;
            }
            let inline = self.inline_port(n);
            let size = if circuit_mode {
                if inline { [14.0, 14.0] } else { [104.0, 26.0] }
            } else if inline {
                [12.0, 12.0]
            } else {
                *self
                    .project
                    .drawing
                    .sizes
                    .get(&n.id)
                    .unwrap_or(&if matches!(n.kind, NodeKind::Room { .. }) {
                        [162.0, 102.0]
                    } else {
                        [138.0, 72.0]
                    })
            };
            nodes.push(NodeDraw {
                id: n.id,
                inline,
                rect: Rect::from_center_size(
                    camera.screen(
                        rect,
                        layout
                            .as_ref()
                            .and_then(|l| l.get(&n.id).copied())
                            .unwrap_or_else(|| coords(n.id, i)),
                    ),
                    vec2(size[0], size[1]) * camera.zoom,
                ),
                opacity: if circuit_mode || n.floor == self.active_floor {
                    1.0
                } else {
                    self.project.drawing.background_opacity
                },
            });
        }
        nodes.sort_by(|a, b| a.opacity.total_cmp(&b.opacity));
        let drawn: HashMap<_, _> = nodes.iter().map(|n| (n.id, n)).collect();
        let find = |id| drawn.get(&id).copied();
        let selected_system = self
            .picked()
            .and_then(|id| net.branch(id))
            .map(|b| b.system);
        let route = if self.route {
            self.connected_route()
        } else {
            HashSet::new()
        };
        let mut routes = Vec::new();
        let mut groups = HashMap::new();
        let key = |a: Pos2, b: Pos2| {
            let a = ((a.x * 2.0).round() as i32, (a.y * 2.0).round() as i32);
            let b = ((b.x * 2.0).round() as i32, (b.y * 2.0).round() as i32);
            (a.min(b), a.max(b))
        };
        for b in &net.branches {
            if focus
                .as_ref()
                .is_some_and(|(id, _)| b.from != *id && b.to != *id)
            {
                continue;
            }
            let (Some(a), Some(z)) = (find(b.from), find(b.to)) else {
                continue;
            };
            if !circuit_mode && self.isolate && selected_system.is_some_and(|s| s != b.system) {
                continue;
            }
            if !circuit_mode && self.elevation_filter {
                let low = b.start_elevation_m.min(b.end_elevation_m);
                let high = b.start_elevation_m.max(b.end_elevation_m);
                let band = self.elevation_band.abs();
                if high < self.elevation_m - band || low > self.elevation_m + band {
                    continue;
                }
            }
            let group = key(a.rect.center(), z.rect.center());
            let count = groups.entry(group).or_insert(0_usize);
            routes.push((b, a, z, group, *count));
            *count += 1;
        }
        let [supply_drops, extract_taps] = if circuit_mode && focus.is_none() {
            [circuit_room_taps(net, true), circuit_room_taps(net, false)]
        } else {
            Default::default()
        };
        let mut header_taps = Vec::new();
        let mut wires = Vec::with_capacity(routes.len());
        for (b, a, z, group, rank) in routes {
            let a_center = a.rect.center();
            let z_center = z.rect.center();
            let map = if circuit_mode {
                &self.project.drawing.circuit
            } else {
                &self.project.drawing.bends
            };
            if circuit_mode {
                let count = circuit::branch_elements(b, net.air).count();
                let manual = focus
                    .is_none()
                    .then(|| map.get(&b.id))
                    .flatten()
                    .map(|&xy| camera.screen(rect, xy));
                let count_in_group = groups[&group];
                let mut offset = (rank as f32 - (count_in_group - 1) as f32 * 0.5) * 36.0;
                let transfer = circuit_transfer(
                    b,
                    net.node(b.from).expect("visible source"),
                    net.node(b.to).expect("visible destination"),
                );
                let delta = z_center - a_center;
                let supply_drop = supply_drops.contains(&b.id) && delta.y > camera.zoom;
                let extract_tap = extract_taps.contains(&b.id) && delta.y > camera.zoom;
                let vertical = delta.y.abs() > camera.zoom
                    && (supply_drop
                        || extract_tap
                        || transfer
                        || delta.x.abs() < 95.0 * camera.zoom);
                let bounds = Rect::from_two_pos(a_center, z_center);
                let mut at = manual.unwrap_or(bounds.center());
                if supply_drop && manual.is_none() {
                    at.x = z_center.x;
                } else if extract_tap && manual.is_none() {
                    at.x = a_center.x;
                }
                if manual.is_none()
                    && vertical
                    && nodes.iter().any(|n| {
                        let point = n.rect.center();
                        point.y > bounds.top() + camera.zoom
                            && point.y < bounds.bottom() - camera.zoom
                            && (point.x - at.x).abs() < 45.0 * camera.zoom
                    })
                {
                    at.x += 60.0 * camera.zoom;
                }
                if manual.is_none()
                    && !vertical
                    && nodes.iter().any(|n| {
                        let point = n.rect.center();
                        point.x > bounds.left() + camera.zoom
                            && point.x < bounds.right() - camera.zoom
                            && (point.y - at.y).abs() <= camera.zoom
                    })
                {
                    offset -= 54.0 + count_in_group.saturating_sub(1) as f32 * 18.0;
                }
                if manual.is_none() {
                    if vertical {
                        at.x += offset * camera.zoom;
                    } else {
                        at.y += offset * camera.zoom;
                    }
                }
                let axis = if vertical {
                    vec2(0.0, delta.y.signum())
                } else {
                    vec2(if delta.x < 0.0 { -1.0 } else { 1.0 }, 0.0)
                };
                let (route, badge) =
                    circuit_wire([a_center, z_center], count, at, axis, camera.zoom);
                if supply_drop {
                    header_taps.push((route.points[1], self.wire_color(b, p)));
                } else if extract_tap {
                    header_taps.push((route.points[4], self.wire_color(b, p)));
                }
                wires.push(Wire {
                    id: b.id,
                    points: [a_center, badge.center(), z_center],
                    opacity: 1.0,
                    badge,
                    color: self.wire_color(b, p),
                    inline_card: false,
                    circuit: Some(route),
                });
                continue;
            }
            let at = map
                .get(&b.id)
                .filter(|_| focus.is_none())
                .map(|&xy| camera.screen(rect, xy))
                .unwrap_or_else(|| {
                    if a_center.distance(z_center) < 2.0 {
                        a_center + vec2(115.0 + rank as f32 * 22.0, -45.0) * camera.zoom
                    } else {
                        let spacing = if focus.is_some() { 70.0 } else { 32.0 };
                        let off = (rank as f32 - (groups[&group] - 1) as f32 * 0.5) * spacing;
                        let d = if (a_center.x, a_center.y) < (z_center.x, z_center.y) {
                            z_center - a_center
                        } else {
                            a_center - z_center
                        }
                        .normalized();
                        a_center.lerp(z_center, 0.5) + vec2(-d.y, d.x) * off * camera.zoom
                    }
                });
            let overlap = a_center.distance(z_center) < 2.0;
            let anchors = self
                .project
                .drawing
                .ports
                .get(&b.id)
                .copied()
                .unwrap_or([None, None]);
            let straight =
                !overlap && groups[&group] == 1 && (focus.is_some() || !map.contains_key(&b.id));
            let start_anchor = anchors[0].map(|side| side_port(a.rect, side));
            let end_anchor = anchors[1].map(|side| side_port(z.rect, side));
            let start = if a.inline {
                a_center
            } else if let Some(anchor) = start_anchor {
                anchor
            } else if straight {
                port(a.rect, end_anchor.unwrap_or(z_center))
            } else if overlap {
                a.rect.right_center()
            } else {
                port(a.rect, at)
            };
            let end = if z.inline {
                z_center
            } else if let Some(anchor) = end_anchor {
                anchor
            } else if straight {
                port(z.rect, start_anchor.unwrap_or(a_center))
            } else if overlap {
                z.rect.right_top() + vec2(0.0, 11.0 * camera.zoom)
            } else {
                port(z.rect, at)
            };
            let at = if straight {
                start.lerp(end, 0.5)
            } else if !overlap && anchors.iter().any(Option::is_some) && !map.contains_key(&b.id) {
                at + ((start - port(a.rect, at)) + (end - port(z.rect, at))) * 0.5
            } else {
                at
            };
            let points = [start, at, end];
            let owns_port = |node: &NodeDraw, legacy_name: &str| {
                self.project.drawing.inline_nodes.get(&node.id).map_or_else(
                    || net.node(node.id).is_some_and(|n| n.name == legacy_name),
                    |owner| *owner == b.id,
                )
            };
            let inline_card = a.inline
                && z.inline
                && start.distance(end) >= 1.0
                && owns_port(a, "Inline inlet")
                && owns_port(z, "Inline outlet");
            let badge = if inline_card {
                inline_card_rect(
                    start,
                    end,
                    camera.zoom,
                    self.project.drawing.sizes.get(&b.id).copied(),
                )
            } else {
                Rect::from_center_size(at, vec2(36.0, 25.0) * camera.zoom.clamp(0.55, 1.3))
            };
            wires.push(Wire {
                id: b.id,
                points,
                opacity: a.opacity.min(z.opacity),
                badge,
                color: self.wire_color(b, p),
                inline_card,
                circuit: None,
            });
        }
        let card_ports: HashSet<_> = wires
            .iter()
            .filter(|w| w.inline_card)
            .filter_map(|w| net.branch(w.id))
            .flat_map(|b| [b.from, b.to])
            .collect();
        let hovered = ctx
            .pointer_interact_pos()
            .filter(|q| rect.intersect(ui.clip_rect()).contains(*q));
        let element_hit = hovered.filter(|_| circuit_mode).and_then(|at| {
            let zoom = camera.zoom;
            let shunts = nodes.iter().filter_map(|draw| {
                let element = circuit::node_element(net.node(draw.id)?, net.air);
                if element.symbol == Symbol::Junction || draw.inline {
                    return None;
                }
                let center = circuit_shunt(draw.rect.center(), zoom);
                let bounds = Rect::from_center_size(center, vec2(20.0, 23.0) * zoom.max(0.5));
                Some((bounds, center, (element.origin, element.symbol)))
            });
            let series = wires
                .iter()
                .filter_map(|w| Some((w, w.circuit.as_ref()?, net.branch(w.id)?)))
                .flat_map(|(w, wire, branch)| {
                    let count = circuit::branch_elements(branch, net.air).count();
                    circuit::branch_elements(branch, net.air).enumerate().map(
                        move |(i, element)| {
                            let center = wire.symbol_center(w.badge, i, count, zoom);
                            (
                                wire.symbol_bounds(center, zoom),
                                center,
                                (element.origin, element.symbol),
                            )
                        },
                    )
                });
            shunts
                .chain(series)
                .filter(|(bounds, _, _)| bounds.contains(at))
                .min_by(|a, b| a.1.distance_sq(at).total_cmp(&b.1.distance_sq(at)))
                .map(|(_, _, key)| key)
        });
        let mut wire_hits = Vec::new();
        if let Some(pos) = hovered {
            wire_hits.extend(
                wires
                    .iter()
                    .rev()
                    .filter(|w| w.badge.expand(4.0).contains(pos) || w.distance(pos) < 9.0)
                    .map(|w| w.id),
            );
        }
        let library_drag = (on_canvas && !circuit_mode)
            .then(|| DragAndDrop::payload::<LibraryDrag>(ctx))
            .flatten();
        let payload = match library_drag.as_deref() {
            Some(LibraryDrag::Part(part)) => Some(part),
            _ => None,
        };
        let preset_drag = match library_drag.as_deref() {
            Some(LibraryDrag::Preset(preset)) => Some(preset),
            _ => None,
        };
        let custom = preset_drag.or(self.placing_preset.as_ref()).cloned();
        if library_drag.is_some() && hovered.is_some() {
            ctx.data_mut(|d| d.insert_temp(Id::new("airframe-drag-preview-painted"), true));
        }
        let target = hovered.and_then(|pos| {
            let selected_part = self
                .port_drag
                .map(|(_, _, p)| p)
                .or(self.tool)
                .unwrap_or(Part::Duct);
            let riser_end = self.reconnect.is_some_and(|(id, from)| {
                !from
                    && net
                        .branch(id)
                        .is_some_and(|b| matches!(b.kind, BranchKind::Duct { riser: true, .. }))
            });
            let part = if riser_end {
                Part::Riser
            } else {
                selected_part
            };
            let destination = riser_end || self.from.is_some() || self.port_drag.is_some();
            nodes
                .iter()
                .rev()
                .find(|n| {
                    n.rect.expand(12.0).contains(pos)
                        && self.endpoint_available(n.id, part, destination)
                })
                .map(|n| n.id)
        });
        let node_hit = hovered
            .and_then(|pos| {
                nodes.iter().rev().find(|n| {
                    !card_ports.contains(&n.id)
                        && (n.inline_hit(pos) || n.rect.expand(5.0).contains(pos))
                })
            })
            .map(|n| n.id);
        let door_drop = (payload == Some(&Part::Door))
            .then_some(node_hit)
            .flatten()
            .filter(|&id| {
                self.endpoint_available(id, Part::Door, false)
                    && net
                        .node(id)
                        .is_some_and(|n| matches!(n.kind, NodeKind::Room { .. }))
            });
        let card_at = |pos: Pos2| {
            wires
                .iter()
                .rev()
                .find(|w| w.inline_card && w.badge.expand(8.0).contains(pos))
        };
        let badge_hit = hovered
            .and_then(|pos| {
                card_at(pos).or_else(|| wires.iter().rev().find(|w| w.badge.contains(pos)))
            })
            .map(|w| w.id);
        let container_at = |at| {
            containers
                .iter()
                .filter(|(_, bounds)| group_header(*bounds).contains(at))
                .min_by(|(a, x), (b, y)| {
                    let groups = &self.project.drawing.containers;
                    (groups[a].floor != self.active_floor)
                        .cmp(&(groups[b].floor != self.active_floor))
                        .then_with(|| x.area().total_cmp(&y.area()))
                })
                .map(|(id, _)| *id)
        };
        if on_canvas
            && editable_pane
            && library_drag.is_none()
            && self.dragging.is_none()
            && !self.draw_container
            && badge_hit.is_none()
            && node_hit.is_none()
            && hovered
                .and_then(container_at)
                .is_some_and(|id| self.component_editable(id))
        {
            ctx.set_cursor_icon(CursorIcon::Grab);
        }
        let hit = element_hit
            .map(|(id, _)| id)
            .or(badge_hit)
            .or(node_hit)
            .or_else(|| hovered.and_then(container_at))
            .or_else(|| wire_hits.first().copied());
        let primary_release = ctx.input(|i| i.pointer.button_released(PointerButton::Primary));
        let space = ctx.input(|i| i.key_down(Key::Space));
        if editable_pane
            && !space
            && library_drag.is_none()
            && let Some(at) = primary_press_in(ui, rect)
        {
            if self.draw_container && !circuit_mode && self.floor_editable(self.active_floor) {
                self.container_start = Some(at);
            } else {
                self.canvas_press = Some((response.id, at));
            }
        }
        if let Some(start) = self.container_start
            && editable_pane
            && !circuit_mode
        {
            if let Some(end) = ctx.pointer_interact_pos() {
                let bounds = Rect::from_two_pos(start, end.clamp(rect.min, rect.max));
                painter.rect(
                    bounds,
                    8,
                    p.accent.gamma_multiply(0.10),
                    Stroke::new(1.0_f32, p.accent),
                    StrokeKind::Inside,
                );
                if primary_release && bounds.width() >= 35.0 && bounds.height() >= 30.0 {
                    let previous = self.project.clone();
                    let id = Uuid::new_v4();
                    self.project.drawing.containers.insert(
                        id,
                        Container {
                            name: "Container".into(),
                            floor: self.active_floor,
                            center: camera.world(rect, bounds.center(), false),
                            size: [bounds.width() / camera.zoom, bounds.height() / camera.zoom],
                        },
                    );
                    self.selection = vec![id];
                    self.draw_container = false;
                    self.checkpoint(previous);
                }
            }
            if primary_release || !ctx.input(|i| i.pointer.primary_down() && i.focused) {
                self.container_start = None;
                ctx.request_repaint();
            }
            ctx.set_cursor_icon(CursorIcon::Crosshair);
            return;
        }
        let resize_hit = |at: Pos2| {
            if circuit_mode
                || self.tool.is_some()
                || custom.is_some()
                || payload.is_some()
                || self.port_drag.is_some()
                || !editable_pane
            {
                return None;
            }
            let inline = wires.iter().rev().find(|w| {
                w.inline_card
                    && self.component_editable(w.id)
                    && w.badge.width() >= 24.0
                    && w.badge.height() >= 20.0
                    && ![w.points[0], w.points[2]]
                        .into_iter()
                        .any(|port| w.port_hit(port, at))
                    && resize_grip(w.badge).contains(at)
            });
            if let Some(card) = inline {
                return Some((card.id, card.badge));
            }
            nodes
                .iter()
                .rev()
                .find(|n| {
                    !n.inline
                        && n.rect.width() >= 24.0
                        && n.rect.height() >= 20.0
                        && self.component_editable(n.id)
                        && resize_grip(n.rect).contains(at)
                })
                .map(|n| (n.id, n.rect))
        };
        let press = self
            .canvas_press
            .filter(|(id, _)| *id == response.id)
            .map(|(_, at)| at);
        let pressed_container = press.and_then(container_at);
        let start_drag = editable_pane
            && !space
            && library_drag.is_none()
            && press
                .zip(ctx.pointer_interact_pos())
                .is_some_and(|(start, at)| start.distance_sq(at) >= 25.0);
        let finish_drag = primary_release || !ctx.input(|i| i.pointer.primary_down() && i.focused);
        let resize_start = press.and_then(resize_hit);
        let resize_hover = hovered.and_then(resize_hit);
        if resize_hover.is_some() {
            ctx.set_cursor_icon(CursorIcon::ResizeNeSw);
        }
        if resize_hover.is_none()
            && editable_pane
            && focus.is_none()
            && on_canvas
            && library_drag.is_none()
            && custom.is_none()
            && self.tool.is_none_or(|part| part.node().is_none())
            && self.reconnect.is_none()
            && self.port_drag.is_none()
            && node_hit.is_some_and(|id| self.component_editable(id))
        {
            ctx.set_cursor_icon(CursorIcon::Grab);
        }
        let splice = if payload.is_some()
            && !circuit_mode
            && node_hit.is_none()
            && hovered.and_then(card_at).is_none()
        {
            hovered.and_then(|at| {
                wires
                    .iter()
                    .filter(|w| !w.inline_card && self.component_editable(w.id))
                    .filter_map(|w| {
                        let (distance, fraction) = wire_projection(at, w.points);
                        let distance = if w.badge.contains(at) { 0.0 } else { distance };
                        (distance <= 13.0).then_some((
                            w.id,
                            distance,
                            WireSplice {
                                wire: w.id,
                                fraction,
                                points: w.points.map(|point| {
                                    let [x, y] = camera.world(rect, point, false);
                                    pos2(x, y)
                                }),
                            },
                        ))
                    })
                    .min_by(|a, b| a.1.total_cmp(&b.1))
            })
        } else {
            None
        };
        for w in &wires {
            if !Rect::from_points(w.path()).expand(40.0).intersects(rect)
                && !w.badge.intersects(rect)
            {
                continue;
            }
            let Some(branch) = net.branch(w.id) else {
                continue;
            };
            let selected = self.selection.contains(&w.id);
            if w.circuit.is_some() {
                let active = selected || route.contains(&w.id) || badge_hit == Some(w.id);
                let symbol = self
                    .circuit_element
                    .filter(|(id, _)| selected && *id == w.id)
                    .or(element_hit.filter(|(id, _)| *id == w.id))
                    .map(|(_, symbol)| symbol);
                draw_circuit_branch(
                    &painter,
                    w,
                    branch,
                    net.air,
                    camera.zoom,
                    p,
                    CircuitHighlight {
                        branch: active,
                        symbol,
                    },
                );
                if (active || self.overlay == Overlay::Direction)
                    && let Some(q) = self.flow(w.id).filter(|q| q.abs() > 1e-7)
                    && let Some((at, direction)) = w
                        .circuit
                        .as_ref()
                        .and_then(|wire| wire.flow_arrow(w.badge, q < 0.0))
                {
                    arrow(&painter, at, direction, w.color);
                }
                continue;
            }
            if w.inline_card {
                continue;
            }
            let beside_card = card_ports.contains(&branch.from) || card_ports.contains(&branch.to);
            let compact = beside_card
                && !selected
                && w.points[0].distance(w.points[1]) + w.points[1].distance(w.points[2])
                    < w.badge.width() + 18.0;
            let overview = plan_layout && camera.zoom < 1.15 && !selected;
            let transfer = plan_layout && branch.system == System::Transfer;
            let color = tint(w.color, w.opacity);
            let hot = selected || route.contains(&w.id) || splice.is_some_and(|s| s.0 == w.id);
            let width: f32 = if splice.is_some_and(|s| s.0 == w.id) {
                4.5
            } else if selected {
                3.6
            } else if hot {
                2.8
            } else {
                1.7
            };
            if overview && transfer {
                painter.add(Shape::line(
                    w.path().to_vec(),
                    Stroke::new(1.0_f32, tint(color, 0.58)),
                ));
            } else {
                painter.add(Shape::line(
                    w.path().to_vec(),
                    Stroke::new(width + 5.0, p.canvas),
                ));
                painter.add(Shape::line(w.path().to_vec(), Stroke::new(width, color)));
            }
            let q = self.flow(w.id);
            let backward = q.is_some_and(|q| q < -1e-8);
            let (a, b) = if backward {
                (w.points[1], w.points[0])
            } else {
                (w.points[1], w.points[2])
            };
            if (b - a).length() > 20.0 && (q.is_none() || q.is_some_and(|q| q.abs() > 1e-7)) {
                arrow(&painter, a.lerp(b, 0.55), (b - a).normalized(), color);
            }
            if overview && transfer && !hovered.is_some_and(|at| w.badge.contains(at)) {
                painter.circle_filled(w.badge.center(), 2.0, tint(color, 0.7));
                continue;
            }
            for (endpoint, id) in [(w.points[0], branch.from), (w.points[2], branch.to)] {
                if !card_ports.contains(&id) {
                    painter.circle(
                        endpoint,
                        if selected { 5.0 } else { 3.0 },
                        p.canvas,
                        Stroke::new(1.4_f32, color),
                    );
                }
            }
            if compact {
                continue;
            }
            let stroke = Stroke::new(if selected { 2.0 } else { 1.0_f32 }, color);
            painter.rect(w.badge, 5, p.surface, stroke, StrokeKind::Inside);
            let label = if branch.kind.tau().is_some() {
                "FAN"
            } else if matches!(branch.kind, BranchKind::Opening { .. }) {
                "OP"
            } else if matches!(branch.kind, BranchKind::Duct { riser: true, .. }) {
                "VR"
            } else {
                "DX"
            };
            painter.text(
                w.badge.center(),
                Align2::CENTER_CENTER,
                label,
                FontId::monospace((10.0 * camera.zoom).clamp(7.0, 14.0)),
                color,
            );
            if camera.zoom > 0.62 && !overview {
                let font = FontId::proportional((11.5 * camera.zoom).clamp(8.0, 15.0));
                let galley = painter.layout(
                    branch.name.clone(),
                    font,
                    tint(p.text, w.opacity),
                    150.0 * camera.zoom,
                );
                let at = w.badge.center_top() - vec2(galley.size().x * 0.5, galley.size().y + 4.0);
                painter.rect_filled(
                    Rect::from_min_size(at, galley.size()).expand2(vec2(4.0, 1.0)),
                    4,
                    tint(p.canvas, w.opacity),
                );
                painter.galley(at, galley, tint(p.text, w.opacity));
                if let Some(q) = q {
                    let (unit, factor) = self.flow_unit();
                    painter.text(
                        w.badge.center_bottom() + vec2(0.0, 4.0),
                        Align2::CENTER_TOP,
                        format!("{:+.1} {unit}", q * factor),
                        FontId::monospace((10.0 * camera.zoom).clamp(8.0, 14.0)),
                        tint(p.muted, w.opacity),
                    );
                }
            }
        }
        for (at, color) in header_taps {
            painter.circle_filled(at, (3.0 * camera.zoom).max(2.0), color);
        }
        for w in wires
            .iter()
            .filter(|w| w.inline_card && w.badge.expand(8.0).intersects(rect))
        {
            let branch = net.branch(w.id).expect("visible branch");
            let selected = self.selection.contains(&w.id);
            let editable = editable_pane && self.component_editable(w.id);
            let hot = on_canvas
                && editable
                && library_drag.is_none()
                && self.placing_preset.is_none()
                && self.tool.is_none_or(|part| part.node().is_none())
                && hovered.is_some_and(|at| w.badge.contains(at));
            if hot && resize_hover.is_none() && self.dragging.is_none() && self.port_drag.is_none()
            {
                ctx.set_cursor_icon(CursorIcon::Grab);
            }
            let color = tint(w.color, w.opacity);
            let card = w.badge;
            painter.rect(
                card,
                9,
                tint(
                    if selected || hot { p.raised } else { p.surface },
                    w.opacity.max(0.76),
                ),
                Stroke::new(if selected { 1.8 } else { 1.0_f32 }, color),
                StrokeKind::Inside,
            );
            let inner = painter.with_clip_rect(rect.intersect(card.shrink(3.0)));
            if card.width() >= 60.0 && card.height() >= 39.0 {
                let font = FontId::proportional((12.0 * camera.zoom).clamp(9.0, 16.0));
                inner.text(
                    card.center_top() + vec2(0.0, 5.0),
                    Align2::CENTER_TOP,
                    branch.kind.label().to_uppercase(),
                    FontId::monospace(9.0),
                    color,
                );
                let name = inner.layout(
                    branch.name.clone(),
                    font,
                    tint(p.text, w.opacity),
                    (card.width() - 12.0).max(8.0),
                );
                let at = card.center() - vec2(name.size().x * 0.5, name.size().y * 0.5);
                let name_clip = Rect::from_min_max(
                    pos2(card.left() + 5.0, card.top() + 17.0),
                    pos2(
                        card.right() - 5.0,
                        card.bottom() - if card.height() >= 49.0 { 15.0 } else { 2.0 },
                    ),
                );
                inner
                    .with_clip_rect(name_clip.intersect(inner.clip_rect()))
                    .galley(at, name, tint(p.text, w.opacity));
                if card.height() >= 49.0
                    && let Some(q) = self.flow(w.id)
                {
                    let (unit, factor) = self.flow_unit();
                    inner.text(
                        card.center_bottom() - vec2(0.0, 5.0),
                        Align2::CENTER_BOTTOM,
                        format!("{:+.1} {unit}", q * factor),
                        FontId::monospace(10.0),
                        color,
                    );
                }
            } else {
                let mark = match branch.kind {
                    BranchKind::Opening { .. } => "OP",
                    BranchKind::Duct { riser: true, .. } => "VR",
                    BranchKind::Duct { .. } => "DX",
                    _ => "FAN",
                };
                inner.text(
                    card.center(),
                    Align2::CENTER_CENTER,
                    mark,
                    FontId::monospace(10.0),
                    color,
                );
            }
            for port in [w.points[0], w.points[2]] {
                let hot = editable
                    && resize_hover.is_none()
                    && library_drag.is_none()
                    && self.port_drag.is_none()
                    && hovered.is_some_and(|at| w.port_hit(port, at));
                if hot {
                    ctx.set_cursor_icon(CursorIcon::Crosshair);
                }
                painter.circle(
                    port,
                    if hot { 5.5 } else { 4.0 },
                    p.canvas,
                    Stroke::new(1.4_f32, if hot { p.accent } else { color }),
                );
            }
            if editable && card.width() >= 24.0 && card.height() >= 20.0 {
                draw_resize_grip(
                    &painter,
                    card,
                    if resize_hover.is_some_and(|(id, _)| id == w.id) {
                        p.accent
                    } else {
                        p.muted
                    },
                );
            }
        }
        for item in &nodes {
            if card_ports.contains(&item.id) {
                continue;
            }
            if !item.rect.intersects(rect) {
                continue;
            }
            let n = net.node(item.id).unwrap();
            let landing = self
                .project
                .drawing
                .riser_landings
                .get(&n.id)
                .and_then(|id| net.branch(*id));
            let selected = self.selection.contains(&n.id) || self.from == Some(n.id);
            let pressure = self.pressure(n.id);
            let color = if self.overlay == Overlay::Floor {
                self.floor_color(n.floor)
            } else if self.overlay == Overlay::Pressure {
                pressure.map_or(p.accent, |v| diverge(v, p))
            } else if n.kind.fixed_gauge_pa().is_some() {
                p.muted
            } else {
                p.accent
            };
            let r = item.rect;
            if circuit_mode {
                draw_circuit_node(&painter, item, n, p, color, camera.zoom, selected);
                if (selected && self.circuit_element.is_some_and(|(id, _)| id == item.id))
                    || element_hit.is_some_and(|(id, _)| id == item.id)
                {
                    let shunt = circuit_shunt(r.center(), camera.zoom);
                    highlight_symbol(
                        &painter,
                        Rect::from_center_size(shunt, vec2(21.0, 26.0) * camera.zoom.max(0.5)),
                        p.accent,
                    );
                }
                if let Some(pa) = pressure.filter(|_| selected) {
                    painter.text(
                        r.center() + vec2(5.0, 7.0),
                        Align2::LEFT_TOP,
                        format!("{pa:+.1} Pa"),
                        FontId::monospace(10.0),
                        p.muted,
                    );
                }
                continue;
            }
            if item.inline {
                painter.circle(
                    r.center(),
                    r.width().clamp(5.0, 14.0) * 0.5,
                    p.surface,
                    Stroke::new(
                        if selected { 2.0 } else { 1.0_f32 },
                        tint(color, item.opacity),
                    ),
                );
                continue;
            }
            let fill = if selected { p.raised } else { p.surface };
            painter.rect(
                r,
                9,
                tint(fill, item.opacity.max(0.76)),
                Stroke::new(
                    if selected { 1.8 } else { 1.0_f32 },
                    tint(
                        if selected || self.overlay == Overlay::Floor {
                            color
                        } else {
                            p.edge
                        },
                        item.opacity,
                    ),
                ),
                StrokeKind::Inside,
            );
            if plan_layout || r.height() < 54.0 {
                let name = painter.layout(
                    n.name.clone(),
                    FontId::monospace((10.0 * camera.zoom).clamp(7.0, 15.0)),
                    tint(p.text, item.opacity),
                    (r.width() - 3.0).max(12.0),
                );
                painter.with_clip_rect(r.intersect(rect)).galley(
                    r.center() - name.size() * 0.5,
                    name,
                    tint(p.text, item.opacity),
                );
                if selected && let Some(value) = pressure {
                    painter.text(
                        r.center_bottom() + vec2(0.0, 3.0),
                        Align2::CENTER_TOP,
                        format!("{value:+.1} Pa"),
                        FontId::monospace(10.0),
                        tint(p.accent, item.opacity),
                    );
                }
            } else if camera.zoom > 0.45 {
                let offset = (10.0 * camera.zoom).clamp(6.0, 15.0);
                let fs = |f: f32| (f * camera.zoom).clamp(7.0, 20.0);
                let at = r.left_top() + vec2(offset, offset);
                painter.text(
                    at,
                    Align2::LEFT_TOP,
                    if landing.is_some() {
                        "VERTICAL RISER".to_owned()
                    } else {
                        n.kind.label().to_uppercase()
                    },
                    FontId::monospace(fs(9.5)),
                    tint(color, item.opacity),
                );
                let name = painter.layout(
                    n.name.clone(),
                    FontId::proportional(fs(14.0)),
                    tint(p.text, item.opacity),
                    (r.width() - offset * 2.0).max(8.0),
                );
                let title_at = at + vec2(0.0, fs(14.0));
                let bottom = r.bottom() - fs(20.0);
                painter
                    .with_clip_rect(Rect::from_min_max(
                        r.min,
                        pos2(r.right(), bottom.max(title_at.y)),
                    ))
                    .galley(title_at, name, tint(p.text, item.opacity));
                let detail = if let Some(riser) = landing {
                    let other = if riser.from == n.id {
                        riser.to
                    } else {
                        riser.from
                    };
                    net.node(other)
                        .and_then(|node| net.floor(node.floor))
                        .map_or_else(
                            || "Riser landing".into(),
                            |f| {
                                let own = net
                                    .floor(n.floor)
                                    .map_or("Floor", |floor| floor.name.as_str());
                                if n.floor == net.node(other).unwrap().floor {
                                    format!(
                                        "{own} · rise {:.1} m",
                                        (riser.end_elevation_m - riser.start_elevation_m).abs()
                                    )
                                } else {
                                    format!("{own} → {}", f.name)
                                }
                            },
                        )
                } else if let Some(p) = pressure {
                    format!("{p:+.2} Pa")
                } else {
                    match n.kind {
                        NodeKind::Room { volume_m3, .. } => format!("{volume_m3:.1} m³"),
                        NodeKind::Boundary { gauge_pa } => format!("{gauge_pa:+.1} Pa"),
                        NodeKind::Atmosphere => "0.0 Pa gauge".into(),
                        NodeKind::Junction => "ΣQ = 0".into(),
                    }
                };
                painter.text(
                    r.left_bottom() + vec2(offset, -offset),
                    Align2::LEFT_BOTTOM,
                    detail,
                    FontId::monospace(fs(11.0)),
                    tint(color, item.opacity),
                );
            }
            if !circuit_mode
                && editable_pane
                && self.component_editable(n.id)
                && r.width() >= 24.0
                && r.height() >= 20.0
            {
                draw_resize_grip(
                    &painter,
                    r,
                    if resize_hover.is_some_and(|(id, _)| id == n.id) {
                        p.accent
                    } else {
                        p.muted
                    },
                );
            }
            if plan_layout
                && camera.zoom < 0.85
                && !selected
                && self.tool.is_none()
                && !hovered.is_some_and(|at| r.expand(12.0).contains(at))
            {
                continue;
            }
            for dot in [r.left_center(), r.right_center()] {
                let hot = !circuit_mode
                    && resize_hover.is_none()
                    && on_canvas
                    && editable_pane
                    && library_drag.is_none()
                    && self.component_editable(n.id)
                    && hovered.is_some_and(|at| at.distance(dot) <= 11.0);
                painter.circle(
                    dot,
                    if hot { 6.0 } else { 4.0 },
                    if hot { p.accent } else { p.canvas },
                    Stroke::new(1.4_f32, tint(color, item.opacity)),
                );
                if hot && self.tool.is_none() {
                    ctx.set_cursor_icon(CursorIcon::Crosshair);
                }
            }
        }
        let suggestion = if !circuit_mode
            && self.show_suggestions
            && self.tool.is_none()
            && self.reconnect.is_none()
            && self.port_drag.is_none()
            && self.dragging.is_none()
            && self.box_select.is_none()
            && library_drag.is_none()
        {
            let mut hits = Vec::new();
            let connected: HashSet<_> = net
                .branches
                .iter()
                .map(|b| (b.from.min(b.to), b.from.max(b.to)))
                .collect();
            let editable: Vec<_> = nodes
                .iter()
                .filter(|n| self.component_editable(n.id))
                .filter(|n| {
                    net.node(n.id)
                        .is_some_and(|node| !node.name.starts_with("Inline "))
                })
                .collect();
            let mut seen = HashSet::new();
            for a in &editable {
                let Some(z) = editable
                    .iter()
                    .copied()
                    .filter(|z| {
                        let distance = z.rect.center().distance(a.rect.center());
                        z.id != a.id
                            && distance <= 360.0 * camera.zoom
                            && !connected.contains(&(a.id.min(z.id), a.id.max(z.id)))
                            && !self
                                .project
                                .drawing
                                .dismissed_connections
                                .contains(&[a.id.min(z.id), a.id.max(z.id)])
                            && !a.rect.expand(26.0).intersects(z.rect)
                    })
                    .min_by(|z1, z2| {
                        z1.rect
                            .center()
                            .distance(a.rect.center())
                            .total_cmp(&z2.rect.center().distance(a.rect.center()))
                    })
                else {
                    continue;
                };
                if !seen.insert((a.id.min(z.id), a.id.max(z.id))) {
                    continue;
                }
                let (a, z) = if (a.rect.center().x, a.rect.center().y)
                    < (z.rect.center().x, z.rect.center().y)
                {
                    (*a, z)
                } else {
                    (z, *a)
                };
                let start = port(a.rect, z.rect.center());
                let end = port(z.rect, a.rect.center());
                let middle = start.lerp(end, 0.5);
                if !rect.shrink(14.0).contains(middle)
                    || nodes.iter().any(|n| n.rect.expand(14.0).contains(middle))
                    || wires.iter().any(|w| w.badge.expand(14.0).contains(middle))
                {
                    continue;
                }
                let hot = response.contains_pointer()
                    && hovered.is_some_and(|at| at.distance(middle) <= 15.0);
                let color = if hot {
                    p.accent
                } else {
                    p.muted.gamma_multiply(0.62)
                };
                dashed(&painter, start, end, color);
                painter.circle(middle, 11.0, p.surface, Stroke::new(1.2_f32, color));
                for d in [vec2(4.0, 0.0), vec2(0.0, 4.0)] {
                    painter.line_segment([middle - d, middle + d], Stroke::new(1.4_f32, color));
                }
                if hot {
                    hits.push((a.id, z.id, middle));
                }
            }
            hits.into_iter().min_by(|a, b| {
                let at = hovered.expect("hovered suggestion");
                at.distance(a.2).total_cmp(&at.distance(b.2))
            })
        } else {
            None
        };
        if suggestion.is_some() {
            ctx.set_cursor_icon(CursorIcon::PointingHand);
            response
                .clone()
                .on_hover_text_at_pointer("Click to connect · Right-click to dismiss");
        }
        if !circuit_mode
            && let Some(part) = payload.filter(|part| part.node().is_none())
            && let Some(at) = hovered
        {
            let valid = if *part == Part::Riser {
                self.riser_drop_available()
            } else {
                splice.is_some() || door_drop.is_some()
            };
            let color = if valid {
                if part.system() == System::Unassigned {
                    p.accent
                } else {
                    self.color(part.system())
                }
            } else {
                Color32::from_rgb(237, 102, 111)
            };
            let label = format!("{}   {}", part.mark(), part.name());
            let font = FontId::monospace(11.0);
            let text = painter.layout_no_wrap(label, font, color);
            let ghost = Rect::from_center_size(at, text.size() + vec2(20.0, 14.0));
            painter.rect(
                ghost,
                6,
                if valid {
                    p.raised
                } else {
                    p.surface.gamma_multiply(0.74)
                },
                Stroke::new(1.4_f32, color),
                StrokeKind::Inside,
            );
            painter.galley(ghost.center() - text.size() * 0.5, text, color);
        }
        let source = self.port_drag.map(|(id, _, _)| id).or(self.from);
        let eligible = target.filter(|id| Some(*id) != source).and_then(find);
        if let Some(from) = source.and_then(find)
            && let Some(at) = hovered
        {
            let origin = self
                .port_drag
                .map(|(_, side, _)| from.port(side))
                .unwrap_or_else(|| port(from.rect, at));
            let destination = eligible.map_or(at, |n| {
                n.port(hit_side(n.rect, at).unwrap_or_else(|| facing_side(n.rect, origin)))
            });
            painter.line_segment([origin, destination], Stroke::new(2.0_f32, p.accent));
            painter.circle_filled(origin, 4.0, p.accent);
        }
        if !circuit_mode && let Some(at) = hovered {
            let placing = payload.copied().or(self.tool);
            if let Some(preset) = custom.as_ref() {
                let [w, h] =
                    preset
                        .size
                        .unwrap_or(if matches!(preset.kind, NodeKind::Room { .. }) {
                            [162.0, 102.0]
                        } else {
                            [138.0, 72.0]
                        });
                let color = if self.floor_editable(self.active_floor) {
                    p.accent
                } else {
                    p.warning
                };
                let ghost = Rect::from_center_size(at, vec2(w, h) * camera.zoom);
                painter.rect(
                    ghost,
                    9,
                    p.raised,
                    Stroke::new(1.6_f32, color),
                    StrokeKind::Inside,
                );
                painter.text(
                    at,
                    Align2::CENTER_CENTER,
                    &preset.name,
                    FontId::proportional(13.0),
                    color,
                );
            } else if let Some(part) = placing.filter(|part| part.node().is_some()) {
                let allowed = self.floor_editable(self.active_floor);
                let xy = splice.map_or_else(
                    || camera.world(rect, at, self.project.drawing.snap),
                    |(_, _, route)| route.point(route.fraction),
                );
                let size = if part == Part::Room {
                    vec2(162.0, 102.0)
                } else {
                    vec2(138.0, 72.0)
                };
                let placement = camera.screen(rect, xy);
                let preview = Rect::from_center_size(
                    if payload.is_some() { at } else { placement },
                    size * camera.zoom,
                );
                let color = if allowed { p.accent } else { p.warning };
                painter.rect(
                    preview,
                    9,
                    p.raised.gamma_multiply(0.86),
                    Stroke::new(1.6_f32, color),
                    StrokeKind::Inside,
                );
                painter.text(
                    preview.center(),
                    Align2::CENTER_CENTER,
                    part.name(),
                    FontId::proportional(13.0),
                    color,
                );
                if payload.is_some() && placement.distance(at) > 3.0 {
                    painter.circle_stroke(placement, 4.0, Stroke::new(1.0_f32, color));
                }
                ctx.set_cursor_icon(if allowed {
                    CursorIcon::Copy
                } else {
                    CursorIcon::NotAllowed
                });
            } else if placing.is_some() || self.port_drag.is_some() || self.reconnect.is_some() {
                if let Some(node) = door_drop
                    .and_then(find)
                    .or(eligible.filter(|_| payload.is_none()))
                {
                    painter.rect_stroke(
                        node.rect.expand(3.0),
                        9,
                        Stroke::new(2.0_f32, p.good),
                        StrokeKind::Outside,
                    );
                }
                ctx.set_cursor_icon(
                    if payload.is_some()
                        && if placing == Some(Part::Riser) {
                            self.riser_drop_available()
                        } else {
                            splice.is_some() || door_drop.is_some()
                        }
                    {
                        CursorIcon::Copy
                    } else if payload.is_none() && eligible.is_some() {
                        CursorIcon::Crosshair
                    } else {
                        CursorIcon::NotAllowed
                    },
                );
            }
        }
        let help = if self.draw_container && !circuit_mode && editable_pane {
            "Drag a box around the items to group".into()
        } else if let Some((center, _)) = focus.as_ref() {
            format!(
                "{}  ·  Direct connections",
                net.node(*center).map_or("Room", |n| n.name.as_str())
            )
        } else if payload == Some(&Part::Riser) && !self.riser_drop_available() {
            "Unlock the active floor to place a riser".into()
        } else if let Some((wire, ..)) = splice {
            format!(
                "Release to connect to {}",
                net.branch(wire).map_or("branch", |b| b.name.as_str())
            )
        } else if let Some((_, _, part)) = self.port_drag {
            format!("{}  ·  Release over the destination box", part.name())
        } else if let Some(part) = payload {
            if part.node().is_some() {
                "Release to place on the active floor".into()
            } else if *part == Part::Riser {
                "Release to place a same-level rise".into()
            } else if *part == Part::Door {
                if door_drop.is_some() {
                    "Release here, then click the second room".into()
                } else {
                    "Drop the door onto the first room".into()
                }
            } else {
                format!("{}  ·  Drop on an existing wire to insert", part.name())
            }
        } else if self.reconnect.is_some() {
            "Pick the replacement pressure node".into()
        } else if let Some(part) = self.tool {
            if part.node().is_some() {
                format!("Place {}  ·  Click canvas", part.name())
            } else if self.from.is_some() {
                format!("{}  ·  Pick TO node", part.name())
            } else {
                format!("{}  ·  Pick FROM node", part.name())
            }
        } else if circuit_mode {
            "Drag symbols to arrange the circuit".into()
        } else {
            format!(
                "{}  ·  {}  /  {} nodes  /  {} branches",
                net.floor(self.active_floor)
                    .map_or("Floor", |f| f.name.as_str()),
                if editable_pane {
                    self.active_floor_status()
                } else {
                    "CLICK TO EDIT"
                },
                nodes.len(),
                wires.len()
            )
        };
        let plate = |at: Pos2, align: Align2, text: String| {
            let g = painter.layout_no_wrap(text, FontId::monospace(10.5), p.muted);
            let r = align.anchor_size(at, g.size());
            painter.rect(
                r.expand2(vec2(8.0, 5.0)),
                5,
                p.surface,
                Stroke::new(1.0_f32, p.edge),
                StrokeKind::Inside,
            );
            painter.galley(r.min, g, p.muted);
        };
        plate(rect.left_top() + vec2(16.0, 14.0), Align2::LEFT_TOP, help);
        plate(
            rect.right_bottom() - vec2(16.0, 14.0),
            Align2::RIGHT_BOTTOM,
            format!(
                "{}%  ·  Scroll zoom  /  Right-drag pan",
                (camera.zoom * 100.0).round()
            ),
        );
        if circuit_mode && self.overlay == Overlay::System {
            let mut systems = [false; System::ALL.len()];
            for w in &wires {
                if let Some(branch) = net.branch(w.id) {
                    systems[branch.system as usize] = true;
                }
            }
            let mut at = rect.left_bottom() + vec2(16.0, -15.0);
            let font = FontId::proportional(11.0);
            for system in System::ALL.into_iter().filter(|s| systems[*s as usize]) {
                let label = painter.layout_no_wrap(system.label().into(), font.clone(), p.muted);
                painter.line_segment(
                    [at, at + vec2(13.0, 0.0)],
                    Stroke::new(1.7_f32, self.color(system)),
                );
                painter.galley(
                    at + vec2(19.0, -label.size().y * 0.5),
                    label.clone(),
                    p.muted,
                );
                at.x += label.size().x + 32.0;
            }
        } else if !circuit_mode {
            let text = match self.overlay {
                Overlay::Floor => "FLOORS  •  Colors match the floor list",
                Overlay::System => "SYSTEM  •  Supply / exhaust / transfer",
                Overlay::Pressure => "PRESSURE  •  Blue −  /  Orange +  ·  ±50 Pa scale",
                Overlay::Magnitude => "MAGNITUDE  •  Dim to bright  ·  0–900 m³/h",
                Overlay::Direction => "DIRECTION  •  Green forward  /  Orange reverse",
                Overlay::Fan => "FANS  •  Green running  /  Gray idle",
            };
            if pane.is_none() {
                plate(
                    rect.left_bottom() + vec2(16.0, -14.0),
                    Align2::LEFT_BOTTOM,
                    text.into(),
                );
            }
        }
        if self.profile
            && !circuit_mode
            && let Some(b) = self.picked().and_then(|id| net.branch(id))
            && self.component_editable(b.id)
        {
            elevation_profile(&painter, rect, b, p);
        }
        if on_canvas
            && suggestion.is_none()
            && library_drag.is_none()
            && self.port_drag.is_none()
            && self.dragging.is_none()
            && self.resize.is_none()
            && self.box_select.is_none()
            && !ctx.input(|i| i.pointer.any_down() || i.pointer.any_released())
            && let Some(id) = hit
        {
            response.clone().on_hover_ui_at_pointer(|ui| {
                if self.project.drawing.containers.contains_key(&id) {
                    if self.component_editable(id) && editable_pane {
                        ui.label("Drag this header to move everything inside.");
                    } else {
                        ui.label("Switch to this container’s level to move it.");
                    }
                } else if let Some(node) = net.node(id) {
                    let summary = if self.project.drawing.riser_landings.contains_key(&id) {
                        "A connection point at one end of a vertical duct."
                    } else {
                        component_help::node_summary(node.kind)
                    };
                    let usage = if node.floor != self.active_floor {
                        "Switch to this node’s level to edit it."
                    } else if !self.floor_editable(node.floor) {
                        "Unlock this node’s level to edit it."
                    } else if !editable_pane {
                        "Click this pane to make its level active."
                    } else if circuit_mode {
                        "Select the node to inspect it. Use Building to connect nodes."
                    } else if find(id).is_some_and(|n| n.inline) {
                        "Drag this dot to move the junction and its attached wires."
                    } else {
                        "Drag the card to move it. Drag a side port onto another node to connect it."
                    };
                    component_help::tooltip(ui, &node.name, summary, usage);
                } else if let Some(branch) = net.branch(id) {
                    let (summary, usage) = component_help::branch_summary(branch);
                    component_help::tooltip(ui, &branch.name, summary, usage);
                    if hovered.and_then(card_at).is_some_and(|card| card.id == id) {
                        ui.small("Drag the card to move it. Drag a port to connect it to another node.");
                    }
                    if wire_hits.len() > 1 {
                        ui.small("Alt-click to cycle through overlapping branches.");
                    }
                }
            });
        }
        if !editable_pane {
            return;
        }
        let mut handled = false;
        if let Some(drag) = library_drag.as_deref()
            && let Some(at) = hovered
            && primary_release
        {
            DragAndDrop::clear_payload(ctx);
            ctx.request_repaint();
            handled = true;
            let xy = camera.world(rect, at, self.project.drawing.snap);
            match drag {
                LibraryDrag::Preset(preset) => self.place_preset(preset, xy),
                LibraryDrag::Part(_) if !self.floor_editable(self.active_floor) => {
                    self.notice("Choose a visible, unlocked floor.", true);
                }
                LibraryDrag::Part(Part::Riser) => {
                    let route = splice.map(|(_, _, route)| route);
                    let xy = route.map_or(xy, |route| route.point(route.fraction));
                    self.place_riser_at(xy, route, target);
                }
                LibraryDrag::Part(part) => {
                    if let Some(id) = door_drop {
                        self.set_tool(*part);
                        self.from = Some(id);
                    } else if let Some((_, _, route)) = splice {
                        self.splice(*part, route);
                    } else if part.node().is_some() {
                        self.place(*part, xy);
                    } else if *part == Part::Door {
                        self.set_tool(*part);
                        self.from = None;
                        self.notice("Click the two rooms to connect them with a door.", false);
                    } else {
                        self.notice("Drop onto an editable wire to insert. To create a new branch, click its library item first.", true);
                    }
                }
            }
        }
        if !handled
            && !start_drag
            && response.clicked_by(PointerButton::Primary)
            && let Some(at) = hovered
        {
            if !circuit_mode && let Some(preset) = custom {
                self.place_preset(&preset, camera.world(rect, at, self.project.drawing.snap));
            } else if let Some((a, z, _)) = suggestion {
                self.connect(Part::Duct, a, z);
            } else if let Some((branch, start)) = self.reconnect {
                if let Some(id) = target {
                    self.replace_end(branch, start, id);
                }
            } else if let Some(part) = self.tool {
                if part.node().is_some() {
                    self.place(part, camera.world(rect, at, self.project.drawing.snap));
                } else if let Some(id) = target {
                    self.pick_connection_node(part, id);
                } else if part == Part::Riser && self.from.is_none() {
                    self.place_riser(camera.world(rect, at, self.project.drawing.snap));
                }
            } else {
                let mut candidates = wire_hits;
                candidates.extend(
                    nodes
                        .iter()
                        .rev()
                        .filter(|n| n.rect.expand(5.0).contains(at))
                        .map(|n| n.id),
                );
                let (alt, shift) =
                    ctx.input(|i| (i.modifiers.alt, i.modifiers.shift || self.multi_select));
                if !alt {
                    self.circuit_element = element_hit;
                    if let Some((id, _)) = element_hit {
                        self.selection = vec![id];
                        ctx.request_repaint();
                        return;
                    }
                } else {
                    self.circuit_element = None;
                }
                let hit = if alt && !candidates.is_empty() {
                    let next = self
                        .picked()
                        .and_then(|id| candidates.iter().position(|&h| h == id))
                        .map_or(0, |i| i + 1);
                    Some(candidates[next % candidates.len()])
                } else {
                    hit
                };
                if let Some(id) = hit {
                    if shift {
                        if self.selection.contains(&id) {
                            self.selection.retain(|x| *x != id);
                        } else {
                            self.selection.push(id);
                        }
                    } else {
                        self.selection = vec![id];
                    }
                    if circuit_mode
                        && focus.as_ref().is_some_and(|(center, _)| *center != id)
                        && self
                            .project
                            .network
                            .node(id)
                            .is_some_and(|n| matches!(n.kind, NodeKind::Room { .. }))
                    {
                        self.fit_next(1, ctx);
                    }
                } else if !shift {
                    self.selection.clear();
                }
            }
        }
        if response.clicked_by(PointerButton::Secondary)
            && let Some((a, b, _)) = suggestion
        {
            let prev = self.project.clone();
            self.project
                .drawing
                .dismissed_connections
                .push([a.min(b), a.max(b)]);
            self.checkpoint(prev);
        }
        if start_drag {
            self.canvas_press = None;
            let inline_card = press.and_then(card_at);
            let container = pressed_container;
            // Junctions outside a component card still move as individual dots.
            let inline_node = press.and_then(|at| {
                nodes
                    .iter()
                    .rev()
                    .find(|n| !card_ports.contains(&n.id) && n.inline_hit(at))
            });
            let source_port = if !circuit_mode
                && inline_node.is_none()
                && self.reconnect.is_none()
                && self.placing_preset.is_none()
                && self.tool.is_none_or(|part| part.node().is_none())
            {
                press.and_then(|at| {
                    if let Some(card) = inline_card {
                        let branch = self.project.network.branch(card.id)?;
                        return [
                            (branch.from, PortSide::Left, card.points[0]),
                            (branch.to, PortSide::Right, card.points[2]),
                        ]
                        .into_iter()
                        .filter(|(id, _, port)| {
                            self.component_editable(*id) && card.port_hit(*port, at)
                        })
                        .min_by(|a, b| a.2.distance_sq(at).total_cmp(&b.2.distance_sq(at)))
                        .map(|(id, side, _)| (id, side));
                    }
                    nodes
                        .iter()
                        .rev()
                        .filter(|n| !n.inline && self.component_editable(n.id))
                        .find_map(|n| hit_side(n.rect, at).map(|side| (n.id, side)))
                })
            } else {
                None
            };
            let start_node = inline_node
                .or_else(|| {
                    press.and_then(|q| {
                        nodes
                            .iter()
                            .rev()
                            .find(|n| !card_ports.contains(&n.id) && n.rect.contains(q))
                    })
                })
                .map(|n| n.id);
            let start_branch = inline_card
                .or_else(|| {
                    press.and_then(|q| wires.iter().rev().find(|w| w.badge.expand(4.0).contains(q)))
                })
                .map(|w| w.id);
            let start_port = press.and_then(|q| {
                wires.iter().find_map(|w| {
                    if circuit_mode
                        || inline_card.is_some()
                        || inline_node.is_some()
                        || self.picked() != Some(w.id)
                    {
                        return None;
                    }
                    if q.distance(w.points[0]) < 8.0 {
                        Some((w.id, true))
                    } else if q.distance(w.points[2]) < 8.0 {
                        Some((w.id, false))
                    } else {
                        None
                    }
                })
            });
            if let Some(at) = press
                && let Some((id, card)) = resize_start
            {
                self.selection = vec![id];
                self.resize = Some((id, at, card.size() / camera.zoom, self.project.clone()));
            } else if let Some((id, side)) = source_port {
                self.from = None;
                self.port_drag = Some((id, side, self.tool.unwrap_or(Part::Duct)));
                ctx.request_repaint();
            } else if let Some((id, side)) = start_port
                && self.component_editable(id)
                && self.tool.is_none()
            {
                self.reconnect = Some((id, side));
            } else if self.reconnect.is_none()
                && focus.is_none()
                && self.tool.is_none_or(|part| part.node().is_none())
                && self.placing_preset.is_none()
                && let Some(id) = inline_card
                    .map(|w| w.id)
                    .or(start_node)
                    .or(start_branch)
                    .or(container)
                && self.component_editable(id)
            {
                let initial = find(id)
                    .map(|n| n.rect.center())
                    .or_else(|| wires.iter().find(|w| w.id == id).map(|w| w.badge.center()))
                    .or_else(|| {
                        containers
                            .iter()
                            .find(|(key, _)| *key == id)
                            .map(|(_, bounds)| bounds.center())
                    })
                    .unwrap();
                if !self.selection.contains(&id) {
                    self.selection = vec![id];
                }
                let mut members: HashSet<_> = self
                    .selection
                    .iter()
                    .copied()
                    .filter(|&id| self.component_editable(id))
                    .collect();
                let groups: Vec<_> = containers
                    .iter()
                    .filter(|(id, _)| members.contains(id))
                    .map(|(_, bounds)| *bounds)
                    .collect();
                let contains = |point| groups.iter().any(|bounds| bounds.contains(point));
                for &(id, bounds) in &containers {
                    if self.component_editable(id)
                        && groups.iter().any(|group| group.contains_rect(bounds))
                    {
                        members.insert(id);
                    }
                }
                let mut moving_nodes = HashMap::new();
                let mut moving_bends = HashMap::new();
                let world = |at| camera.world(rect, at, false);
                for n in &nodes {
                    if self.component_editable(n.id)
                        && (members.contains(&n.id)
                            || (!card_ports.contains(&n.id) && contains(n.rect.center())))
                    {
                        moving_nodes.insert(n.id, world(n.rect.center()));
                    }
                }
                for w in &wires {
                    if !self.component_editable(w.id) {
                        continue;
                    }
                    if members.contains(&w.id) || contains(w.badge.center()) {
                        if w.inline_card {
                            let branch = self.project.network.branch(w.id).unwrap();
                            for (id, point) in
                                [(branch.from, w.points[0]), (branch.to, w.points[2])]
                            {
                                if self.component_editable(id) {
                                    moving_nodes.insert(id, world(point));
                                }
                            }
                            if let Some(&bend) = self.project.drawing.bends.get(&w.id) {
                                moving_bends.insert(w.id, bend);
                            }
                        } else if members.contains(&w.id)
                            || self.project.drawing.bends.contains_key(&w.id)
                        {
                            moving_bends.insert(w.id, world(w.badge.center()));
                        }
                    }
                }
                if !circuit_mode {
                    for branch in &self.project.network.branches {
                        if moving_nodes.contains_key(&branch.from)
                            && moving_nodes.contains_key(&branch.to)
                            && let Some(&bend) = self.project.drawing.bends.get(&branch.id)
                        {
                            moving_bends.insert(branch.id, bend);
                        }
                    }
                }
                let [x, y] = world(press.unwrap_or(initial));
                let [anchor_x, anchor_y] = world(initial);
                self.dragging = Some(MoveDrag {
                    before: self.project.clone(),
                    start: pos2(x, y),
                    anchor: vec2(anchor_x, anchor_y),
                    nodes: moving_nodes,
                    bends: moving_bends,
                    containers: containers
                        .iter()
                        .filter(|(id, _)| members.contains(id))
                        .map(|(id, _)| *id)
                        .collect(),
                });
            } else if self.tool.is_none()
                && self.reconnect.is_none()
                && start_node.is_none()
                && start_branch.is_none()
                && container.is_none()
                && let Some(start) = press
                && wires.iter().all(|w| w.distance(start) > 8.0)
            {
                let add = ctx.input(|i| i.modifiers.shift) || self.multi_select;
                self.box_select = Some((start, add));
                ctx.request_repaint();
            }
        }
        if let Some((start, add)) = self.box_select
            && let Some(at) = response
                .interact_pointer_pos()
                .or_else(|| ctx.pointer_interact_pos())
        {
            let area = Rect::from_two_pos(start, at).intersect(rect);
            painter.rect(
                area,
                0,
                p.accent.gamma_multiply(0.13),
                Stroke::new(1.0_f32, p.accent),
                StrokeKind::Inside,
            );
            let inside: Vec<_> = nodes
                .iter()
                .filter(|n| {
                    !card_ports.contains(&n.id)
                        && area.intersects(n.rect)
                        && self.component_editable(n.id)
                })
                .map(|n| n.id)
                .chain(
                    wires
                        .iter()
                        .filter(|w| area.intersects(w.badge) && self.component_editable(w.id))
                        .map(|w| w.id),
                )
                .collect();
            if primary_release {
                if !add {
                    self.selection.clear();
                }
                for id in inside {
                    if !self.selection.contains(&id) {
                        self.selection.push(id);
                    }
                }
            } else {
                for id in inside {
                    if let Some(n) = find(id) {
                        painter.rect_stroke(
                            n.rect.expand(2.0),
                            7,
                            Stroke::new(1.5_f32, p.accent),
                            StrokeKind::Outside,
                        );
                    } else if let Some(w) = wires.iter().find(|w| w.id == id) {
                        painter.rect_stroke(
                            w.badge.expand(2.0),
                            5,
                            Stroke::new(1.5_f32, p.accent),
                            StrokeKind::Outside,
                        );
                    }
                }
            }
        }
        if let Some(drag) = &self.dragging
            && let Some(at) = ctx.input(|i| i.pointer.interact_pos())
        {
            let [x, y] = camera.world(rect, at, false);
            let mut delta = pos2(x, y) - drag.start;
            if !circuit_mode && self.project.drawing.snap {
                let target = drag.anchor + delta;
                delta = (target / 20.0).round() * 20.0 - drag.anchor;
            }
            let translate = |[x, y]: [f32; 2]| [x + delta.x, y + delta.y];
            let drawing = &mut self.project.drawing;
            if circuit_mode {
                drawing.circuit.extend(
                    drag.nodes
                        .iter()
                        .chain(&drag.bends)
                        .map(|(&id, &pos)| (id, translate(pos))),
                );
            } else {
                drawing
                    .positions
                    .extend(drag.nodes.iter().map(|(&id, &pos)| (id, translate(pos))));
                drawing
                    .bends
                    .extend(drag.bends.iter().map(|(&id, &pos)| (id, translate(pos))));
                for id in &drag.containers {
                    if let Some(group) = drawing.containers.get_mut(id) {
                        group.center = translate(drag.before.drawing.containers[id].center);
                    }
                }
            }
            ctx.set_cursor_icon(CursorIcon::Grabbing);
            ctx.request_repaint();
        }
        if let Some((id, start, initial, before)) = &self.resize
            && let Some(at) = ctx.pointer_interact_pos()
        {
            let delta = at - *start;
            let size = *initial + vec2(delta.x, -delta.y) / camera.zoom * 2.0;
            let size = vec2(size.x.clamp(36.0, 900.0), size.y.clamp(30.0, 700.0));
            if let Some(branch) = before.network.branch(*id)
                && let (Some(from), Some(to)) = (
                    before.drawing.positions.get(&branch.from),
                    before.drawing.positions.get(&branch.to),
                )
            {
                let center = (vec2(from[0], from[1]) + vec2(to[0], to[1])) * 0.5;
                let scale = size / *initial;
                let stretch = |[x, y]: [f32; 2]| {
                    let point = center + (vec2(x, y) - center) * scale;
                    [point.x, point.y]
                };
                self.project
                    .drawing
                    .positions
                    .insert(branch.from, stretch(*from));
                self.project
                    .drawing
                    .positions
                    .insert(branch.to, stretch(*to));
                if let Some(&bend) = before.drawing.bends.get(id) {
                    self.project.drawing.bends.insert(*id, stretch(bend));
                }
            }
            self.project.drawing.sizes.insert(*id, [size.x, size.y]);
            ctx.set_cursor_icon(CursorIcon::ResizeNeSw);
            ctx.request_repaint();
        }
        if finish_drag {
            self.canvas_press = None;
            self.box_select = None;
            if let Some((from, side, part)) = self.port_drag.take() {
                let origin = find(from).map(|n| n.port(side));
                let destination = hovered.filter(|_| on_canvas).and_then(|at| {
                    nodes
                        .iter()
                        .rev()
                        .find(|n| {
                            n.id != from
                                && n.rect.expand(12.0).contains(at)
                                && self.endpoint_available(n.id, part, true)
                        })
                        .map(|n| {
                            (
                                n.id,
                                hit_side(n.rect, at)
                                    .unwrap_or_else(|| facing_side(n.rect, origin.unwrap_or(at))),
                            )
                        })
                });
                if primary_release && let Some((to, end)) = destination {
                    self.connect_ports(part, from, to, [Some(side), Some(end)]);
                } else {
                    self.notice(
                        "Connection cancelled. Drag from a port to a different pressure node.",
                        false,
                    );
                }
            }
            if let Some(drag) = self.dragging.take() {
                self.checkpoint(drag.before);
                ctx.request_repaint();
            }
            if let Some((_, _, _, prev)) = self.resize.take() {
                self.checkpoint(prev);
            }
            if primary_release
                && (start_drag || response.drag_stopped_by(PointerButton::Primary))
                && let Some((id, start)) = self.reconnect
                && let Some(target) = target
            {
                self.replace_end(id, start, target);
            }
        }
    }
    pub fn drag_preview_outside_canvas(&self, ctx: &Context, p: Palette) {
        let Some(drag) = DragAndDrop::payload::<LibraryDrag>(ctx) else {
            return;
        };
        let Some(at) = ctx.pointer_interact_pos() else {
            return;
        };
        if ctx.data(|d| {
            d.get_temp::<bool>(Id::new("airframe-drag-preview-painted"))
                .unwrap_or(false)
        }) {
            return;
        }
        let painter = ctx.layer_painter(LayerId::new(
            Order::Tooltip,
            Id::new("airframe-library-ghost"),
        ));
        let (label, color) = match drag.as_ref() {
            LibraryDrag::Part(part) => (
                format!("{}   {}", part.mark(), part.name()),
                if part.node().is_none() {
                    Color32::from_rgb(237, 102, 111)
                } else {
                    p.accent
                },
            ),
            LibraryDrag::Preset(preset) => (preset.name.clone(), p.accent),
        };
        let title = painter.layout_no_wrap(label, FontId::monospace(11.0), color);
        let rect = Rect::from_center_size(at, title.size() + vec2(20.0, 14.0));
        painter.rect(
            rect,
            6,
            p.raised.gamma_multiply(0.8),
            Stroke::new(1.4_f32, color),
            StrokeKind::Inside,
        );
        painter.galley(rect.center() - title.size() * 0.5, title, color);
    }
    fn world_pos(&self, id: Uuid, i: usize, circuit: bool) -> [f32; 2] {
        let xy = if circuit {
            &self.project.drawing.circuit
        } else {
            &self.project.drawing.positions
        };
        xy.get(&id).copied().unwrap_or_else(|| {
            if circuit {
                if self
                    .project
                    .drawing
                    .floor_plans
                    .contains_key(&self.project.network.nodes[i].floor)
                {
                    return [
                        (i % 9) as f32 * 180.0 - 720.0,
                        (i / 9) as f32 * 110.0 - 520.0,
                    ];
                }
                let mut xy = self.world_pos(id, i, false);
                let floor = self.project.network.nodes[i].floor;
                let level = self
                    .project
                    .network
                    .floors
                    .iter()
                    .position(|f| f.id == floor)
                    .unwrap_or(0);
                let center = self.project.network.floors.len().saturating_sub(1) as f32 * 0.5;
                xy[1] += (level as f32 - center) * 220.0;
                xy
            } else {
                [
                    (i % 4) as f32 * 200.0 - 300.0,
                    (i / 4) as f32 * 130.0 - 70.0,
                ]
            }
        })
    }
    pub(crate) fn fit_next(&self, idx: usize, ctx: &Context) {
        ctx.data_mut(|d| d.insert_temp(Id::new(("fit-camera", idx)), true));
    }
    fn fit(&mut self, idx: usize, rect: Rect) {
        let mut bounds = Rect::NOTHING;
        if idx == 0
            && self.plan_visible
            && let Some(plan) = self.project.drawing.floor_plans.get(&self.active_floor)
        {
            bounds.extend_with(pos2(plan.origin[0], plan.origin[1]));
            bounds.extend_with(pos2(
                plan.origin[0] + plan.size[0],
                plan.origin[1] + plan.size[1],
            ));
        }
        if idx == 0 {
            for container in self.project.drawing.containers.values().filter(|group| {
                self.project
                    .network
                    .floor(group.floor)
                    .is_some_and(|floor| {
                        floor.visible
                            && (group.floor == self.active_floor
                                || (!self.split_rendering && self.all_floors))
                    })
            }) {
                let [x, y] = container.center;
                let [width, height] = container.size;
                bounds = bounds.union(Rect::from_center_size(pos2(x, y), vec2(width, height)));
            }
        }
        let focus = (idx == 1).then(|| self.circuit_focus()).flatten();
        let layout = (idx == 1).then(|| self.circuit_positions(focus.as_ref()));
        let visible = |floor| {
            self.project.network.floor(floor).is_some_and(|f| {
                f.visible
                    && (idx == 1
                        || (!self.split_rendering && self.all_floors)
                        || f.id == self.active_floor)
            })
        };
        for (i, n) in self.project.network.nodes.iter().enumerate() {
            if !visible(n.floor) || focus.as_ref().is_some_and(|(_, ids)| !ids.contains(&n.id)) {
                continue;
            }
            let xy = layout
                .as_ref()
                .and_then(|l| l.get(&n.id).copied())
                .unwrap_or_else(|| self.world_pos(n.id, i, idx == 1));
            bounds.extend_with(pos2(xy[0], xy[1]));
        }
        if !bounds.is_finite() {
            self.cameras[idx] = Camera::default();
            return;
        }
        if layout.is_some() && focus.is_none() {
            for b in &self.project.network.branches {
                if [b.from, b.to].into_iter().all(|id| {
                    self.project
                        .network
                        .node(id)
                        .is_some_and(|n| visible(n.floor))
                }) && let Some(&[x, y]) = self.project.drawing.circuit.get(&b.id)
                {
                    bounds.extend_with(pos2(x, y));
                }
            }
        }
        bounds = bounds.expand2(if idx == 1 {
            vec2(132.0, 76.0)
        } else {
            vec2(106.0, 76.0)
        });
        let zoom = (rect.width() / bounds.width())
            .min(rect.height() / bounds.height())
            .clamp(0.25, 1.25);
        self.cameras[idx] = Camera {
            pan: -bounds.center().to_vec2() * zoom,
            zoom,
        };
    }
    fn paint_floor_plan(&mut self, ctx: &Context, painter: &Painter, rect: Rect, camera: Camera) {
        let Some(plan) = self.project.drawing.floor_plans.get(&self.active_floor) else {
            return;
        };
        let path = self.plan_path(&plan.image);
        let key = path.to_string_lossy().into_owned();
        let cache_key = Id::new(("missing-floor-plan", key.as_str()));
        if self
            .plan_texture
            .get(&self.active_floor)
            .is_none_or(|(file, _)| *file != key)
            && !ctx.data(|d| d.get_temp::<bool>(cache_key).unwrap_or(false))
        {
            match image::open(&path) {
                Ok(decoded) => {
                    let rgba = decoded.to_rgba8();
                    let pixels = ColorImage::from_rgba_unmultiplied(
                        [rgba.width() as usize, rgba.height() as usize],
                        rgba.as_raw(),
                    );
                    let texture = ctx.load_texture("floor-plan", pixels, TextureOptions::LINEAR);
                    self.plan_texture
                        .insert(self.active_floor, (key.clone(), texture));
                }
                Err(error) => {
                    ctx.data_mut(|d| d.insert_temp(cache_key, true));
                    self.notice(format!("Could not open floor plan image: {error}"), true);
                    return;
                }
            }
        }
        if let Some((file, texture)) = self.plan_texture.get(&self.active_floor)
            && *file == key
        {
            let bounds = Rect::from_min_size(
                camera.screen(rect, plan.origin),
                vec2(plan.size[0], plan.size[1]) * camera.zoom,
            );
            painter.image(
                texture.id(),
                bounds,
                Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)),
                Color32::WHITE.gamma_multiply(self.plan_opacity),
            );
        }
    }
    fn connected_route(&self) -> HashSet<Uuid> {
        let net = &self.project.network;
        let Some(seed) = self.picked().and_then(|id| net.branch(id)) else {
            return HashSet::new();
        };
        let mut nodes = HashSet::from([seed.from, seed.to]);
        let mut branches = HashSet::from([seed.id]);
        loop {
            let count = branches.len();
            for b in &net.branches {
                if b.system == seed.system && (nodes.contains(&b.from) || nodes.contains(&b.to)) {
                    branches.insert(b.id);
                    nodes.extend([b.from, b.to]);
                }
            }
            if count == branches.len() {
                break;
            }
        }
        branches
    }
    fn wire_color(&self, b: &Branch, p: Palette) -> Color32 {
        let base = self.color(b.system);
        match self.overlay {
            Overlay::Floor => self
                .project
                .network
                .node(b.from)
                .map_or(base, |n| self.floor_color(n.floor)),
            Overlay::Magnitude => self.flow(b.id).map_or(base, |q| {
                p.grid
                    .lerp_to_gamma(p.accent, (q.abs() * 4.0).min(1.0) as f32)
            }),
            Overlay::Direction => self.flow(b.id).map_or(base, |q| {
                if q < -1e-7 {
                    p.warning
                } else if q > 1e-7 {
                    p.good
                } else {
                    p.muted
                }
            }),
            Overlay::Fan => match b.kind {
                BranchKind::FlowFan { design_m3_s, .. } if design_m3_s > 0.0 => p.good,
                BranchKind::PressureFan { .. } if b.fan_speed_fraction.unwrap_or(1.0) > 0.0 => {
                    p.good
                }
                _ => p.muted,
            },
            _ => base,
        }
    }
}

fn resize_grip(rect: Rect) -> Rect {
    Rect::from_min_size(rect.right_top() + vec2(-17.0, 0.0), vec2(17.0, 17.0))
}

fn draw_resize_grip(painter: &Painter, rect: Rect, color: Color32) {
    let corner = rect.right_top() + vec2(-4.0, 4.0);
    for length in [3.0, 6.0, 9.0] {
        painter.line_segment(
            [corner - vec2(length, 0.0), corner + vec2(0.0, length)],
            Stroke::new(1.0_f32, color),
        );
    }
}

fn port(r: Rect, target: Pos2) -> Pos2 {
    let d = target - r.center();
    if d.length_sq() < 1e-6 {
        return r.right_center();
    }
    let f = (r.width() * 0.5 / d.x.abs()).min(r.height() * 0.5 / d.y.abs());
    r.center() + d * f
}
fn side_port(rect: Rect, side: PortSide) -> Pos2 {
    match side {
        PortSide::Left => rect.left_center(),
        PortSide::Right => rect.right_center(),
    }
}

fn circuit_wire(
    [from, to]: [Pos2; 2],
    count: usize,
    at: Pos2,
    axis: Vec2,
    zoom: f32,
) -> (CircuitWire, Rect) {
    let vertical = axis.y != 0.0;
    let length = (32.0 + (count - 1) as f32 * 44.0) * zoom;
    let points = if vertical {
        let extent = axis * (length * 0.5 + 7.0 * zoom);
        let entry = (axis.y * from.y).min(axis.y * (at.y - extent.y)) * axis.y;
        let exit = (axis.y * to.y).max(axis.y * (at.y + extent.y)) * axis.y;
        vec![
            from,
            pos2(at.x, from.y),
            pos2(at.x, entry),
            pos2(at.x, exit),
            pos2(at.x, to.y),
            to,
        ]
    } else {
        let extent = axis * (length * 0.5 + 7.0 * zoom);
        let entry = (axis.x * from.x).min(axis.x * (at.x - extent.x)) * axis.x;
        let exit = (axis.x * to.x).max(axis.x * (at.x + extent.x)) * axis.x;
        let lead = (from.x - to.x).abs().min(46.0 * zoom) * 0.5 * axis.x;
        let entry = if axis.x * (from.x + lead) < axis.x * (at.x - extent.x) {
            from.x + lead
        } else {
            entry
        };
        let exit = if axis.x * (to.x - lead) > axis.x * (at.x + extent.x) {
            to.x - lead
        } else {
            exit
        };
        vec![
            from,
            pos2(entry, from.y),
            pos2(entry, at.y),
            pos2(exit, at.y),
            pos2(exit, to.y),
            to,
        ]
    };
    let size = if vertical {
        vec2(24.0 * zoom, length)
    } else {
        vec2(length, 24.0 * zoom)
    };
    (
        CircuitWire { points, axis },
        Rect::from_center_size(at, size),
    )
}

struct CircuitHighlight {
    branch: bool,
    symbol: Option<Symbol>,
}

fn highlight_symbol(p: &Painter, bounds: Rect, color: Color32) {
    p.rect_stroke(
        bounds.expand(2.0),
        3,
        Stroke::new(1.35_f32, color),
        StrokeKind::Outside,
    );
}

fn circuit_shunt(at: Pos2, zoom: f32) -> Pos2 {
    at + vec2(14.0, 29.0) * zoom
}

fn draw_circuit_branch(
    p: &Painter,
    w: &Wire,
    branch: &Branch,
    air: Air,
    zoom: f32,
    palette: Palette,
    highlight: CircuitHighlight,
) {
    let route = w.circuit.as_ref().expect("circuit wire");
    let hot = highlight.branch;
    let stroke = Stroke::new(if hot { 2.1 } else { 1.25_f32 }, w.color);
    p.add(Shape::line(route.points.clone(), stroke));
    let elements = circuit::branch_elements(branch, air);
    let count = circuit::branch_elements(branch, air).count();
    for (i, el) in elements.enumerate() {
        let center = route.symbol_center(w.badge, i, count, zoom);
        let span = 14.5 * zoom;
        p.line_segment(
            [center - route.axis * span, center + route.axis * span],
            Stroke::new(stroke.width + 2.0, palette.canvas),
        );
        draw_symbol(p, center, el.symbol, w.color, zoom, route.axis);
        if highlight.symbol == Some(el.symbol) {
            highlight_symbol(p, route.symbol_bounds(center, zoom), palette.accent);
        }
    }
    let vertical = route.axis.y != 0.0;
    let (point, anchor) = if vertical {
        (w.badge.right_center() + vec2(5.0, 0.0), Align2::LEFT_CENTER)
    } else {
        (w.badge.center_bottom() + vec2(0.0, 2.0), Align2::CENTER_TOP)
    };
    if zoom >= 0.38 || hot {
        let label = p.layout(
            branch.name.clone(),
            FontId::proportional((12.0 * zoom).clamp(8.0, 16.0)),
            if hot { palette.text } else { palette.muted },
            (155.0 * zoom).max(70.0),
        );
        let text_rect = anchor.anchor_size(point, label.size());
        p.rect_filled(text_rect.expand(1.0), 0, palette.canvas);
        p.galley(text_rect.min, label, palette.text);
    }
}

fn draw_circuit_node(
    p: &Painter,
    item: &NodeDraw,
    node: &Node,
    palette: Palette,
    color: Color32,
    zoom: f32,
    selected: bool,
) {
    let at = item.rect.center();
    let scale = zoom.max(0.4);
    let ground = vec2(0.0, 1.0);
    if selected {
        p.circle_filled(at, 7.0, palette.raised);
        p.circle_stroke(at, 7.0, Stroke::new(1.2_f32, palette.accent));
    }
    p.circle_filled(at, (3.0 * zoom).max(2.0), color);
    let symbol = match node.kind {
        NodeKind::Room { .. } => Symbol::Capacitor,
        NodeKind::Boundary { .. } | NodeKind::Atmosphere => Symbol::Ground,
        NodeKind::Junction => Symbol::Junction,
    };
    if !matches!(symbol, Symbol::Junction) {
        let terminal = at + ground * (12.0 * zoom);
        let shunt = circuit_shunt(at, zoom);
        p.add(Shape::line(
            vec![
                at,
                terminal,
                pos2(shunt.x, terminal.y),
                shunt - ground * (8.0 * scale),
            ],
            Stroke::new(1.2_f32, color),
        ));
        draw_symbol(p, shunt, symbol, color, scale, Vec2::X);
    }
    if !item.inline || selected {
        let font = FontId::proportional((14.0 * zoom).clamp(9.0, 17.0));
        let text = p.layout(
            node.name.clone(),
            font,
            palette.text,
            (146.0 * zoom).max(64.0),
        );
        let point = at - vec2(5.0 * zoom, 8.0 * zoom);
        let bounds = Align2::RIGHT_BOTTOM.anchor_size(point, text.size());
        p.rect_filled(
            bounds.expand(2.0),
            2,
            if selected {
                palette.raised
            } else {
                palette.canvas
            },
        );
        p.galley(bounds.min, text, palette.text);
    }
}
fn facing_side(rect: Rect, target: Pos2) -> PortSide {
    if target.x < rect.center().x {
        PortSide::Left
    } else {
        PortSide::Right
    }
}
fn hit_side(rect: Rect, at: Pos2) -> Option<PortSide> {
    let side = facing_side(rect, at);
    (side_port(rect, side).distance(at) <= 11.0).then_some(side)
}
fn segment_distance(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let d = b - a;
    let scale = (p - a).dot(d) / d.length_sq().max(1e-8);
    p.distance(a + d * scale.clamp(0.0, 1.0))
}
fn wire_projection(at: Pos2, points: [Pos2; 3]) -> (f32, f64) {
    let lengths = [points[0].distance(points[1]), points[1].distance(points[2])];
    let total = (lengths[0] + lengths[1]).max(0.01);
    let project = |i: usize| {
        let delta = points[i + 1] - points[i];
        let t = ((at - points[i]).dot(delta) / delta.length_sq().max(1e-8)).clamp(0.0, 1.0);
        let point = points[i] + t * delta;
        let fraction = (if i == 0 { 0.0 } else { lengths[0] } + t * lengths[i]) / total;
        (at.distance(point), f64::from(fraction))
    };
    let a = project(0);
    let b = project(1);
    if a.0 <= b.0 { a } else { b }
}
fn dashed(painter: &Painter, from: Pos2, to: Pos2, color: Color32) {
    let length = from.distance(to);
    let direction = (to - from).normalized();
    let mut at = 0.0;
    while at < length {
        painter.line_segment(
            [
                from + at * direction,
                from + (at + 5.0).min(length) * direction,
            ],
            Stroke::new(1.2_f32, color),
        );
        at += 10.0;
    }
}
fn tint(color: Color32, alpha: f32) -> Color32 {
    color.gamma_multiply(alpha)
}
fn diverge(v: f64, p: Palette) -> Color32 {
    p.muted.lerp_to_gamma(
        if v >= 0.0 { p.warning } else { p.accent },
        (v.abs() / 50.0).min(1.0) as f32,
    )
}
fn arrow(p: &Painter, at: Pos2, dir: Vec2, color: Color32) {
    let normal = vec2(-dir.y, dir.x);
    p.add(Shape::convex_polygon(
        vec![
            at + dir * 5.0,
            at - dir * 4.0 + normal * 3.5,
            at - dir * 4.0 - normal * 3.5,
        ],
        color,
        Stroke::NONE,
    ));
}
fn grid(painter: &Painter, r: Rect, cam: Camera, p: Palette) {
    let step = if cam.zoom < 0.58 { 40.0 } else { 20.0 } * cam.zoom;
    let c = r.center() + cam.pan;
    let mut x = r.left() + (c.x - r.left()).rem_euclid(step);
    while x < r.right() {
        let mut y = r.top() + (c.y - r.top()).rem_euclid(step);
        while y < r.bottom() {
            painter.circle_filled(pos2(x, y), 0.95, p.grid);
            y += step;
        }
        x += step;
    }
}
fn elevation_profile(painter: &Painter, r: Rect, b: &Branch, p: Palette) {
    let box_r = Rect::from_min_size(r.left_top() + vec2(16.0, 45.0), vec2(200.0, 76.0));
    painter.rect(
        box_r,
        7,
        p.surface,
        Stroke::new(1.0_f32, p.edge),
        StrokeKind::Inside,
    );
    painter.text(
        box_r.left_top() + vec2(10.0, 8.0),
        Align2::LEFT_TOP,
        "BRANCH ELEVATION  /  m",
        FontId::monospace(10.0),
        p.muted,
    );
    let mut a = box_r.left_bottom() + vec2(15.0, -22.0);
    let mut z = box_r.right_bottom() + vec2(-15.0, -22.0);
    if b.start_elevation_m > b.end_elevation_m {
        a.y -= 19.0;
    } else if b.end_elevation_m > b.start_elevation_m {
        z.y -= 19.0;
    }
    painter.line_segment([a, z], Stroke::new(2.0_f32, p.accent));
    for (xy, value, align) in [
        (a, b.start_elevation_m, Align2::LEFT_TOP),
        (z, b.end_elevation_m, Align2::RIGHT_TOP),
    ] {
        painter.circle_filled(xy, 3.0, p.accent);
        painter.text(
            xy + vec2(0.0, 5.0),
            align,
            format!("{value:.2}"),
            FontId::monospace(10.0),
            p.text,
        );
    }
}
fn draw_symbol(p: &Painter, at: Pos2, symbol: Symbol, color: Color32, z: f32, axis: Vec2) {
    let normal = vec2(-axis.y, axis.x);
    let pt = |x, y| at + (axis * x + normal * y) * z;
    let st = Stroke::new(1.4_f32, color);
    let line = |x, y, a, b| {
        p.line_segment([pt(x, y), pt(a, b)], st);
    };
    match symbol {
        Symbol::Capacitor => {
            line(0.0, -8.0, 0.0, -3.0);
            line(-8.0, -3.0, 8.0, -3.0);
            line(-8.0, 1.0, 8.0, 1.0);
            line(0.0, 1.0, 0.0, 8.0);
            line(-6.0, 8.0, 6.0, 8.0);
            line(-3.0, 11.0, 3.0, 11.0);
        }
        Symbol::Ground => {
            line(0.0, -8.0, 0.0, 0.0);
            line(-8.0, 0.0, 8.0, 0.0);
            line(-5.0, 3.0, 5.0, 3.0);
            line(-2.0, 6.0, 2.0, 6.0);
        }
        Symbol::Junction => {
            p.circle_filled(at, 3.0 * z, color);
        }
        Symbol::Resistor => {
            p.add(Shape::line(
                vec![
                    pt(-14., 0.),
                    pt(-10., 0.),
                    pt(-7., -5.),
                    pt(-4., 5.),
                    pt(-1., -5.),
                    pt(2., 5.),
                    pt(5., -5.),
                    pt(8., 5.),
                    pt(11., 0.),
                    pt(14., 0.),
                ],
                st,
            ));
        }
        Symbol::Inductor => {
            line(-14.0, 0.0, -11.6, 0.0);
            line(11.6, 0.0, 14.0, 0.0);
            for i in 0..4 {
                let left = -11.6 + i as f32 * 5.8;
                let arc = (0..=10)
                    .map(|j| {
                        let theta = std::f32::consts::PI * j as f32 / 10.0;
                        pt(left + 2.9 * (1.0 - theta.cos()), -4.5 * theta.sin())
                    })
                    .collect::<Vec<_>>();
                p.add(Shape::line(arc, st));
            }
        }
        Symbol::CurrentSource => {
            line(-14., 0., -8., 0.);
            line(8., 0., 14., 0.);
            p.circle_stroke(at, 8.0 * z, st);
            line(-5., 0., 5., 0.);
            line(5., 0., 2., -3.);
            line(5., 0., 2., 3.);
        }
        Symbol::VoltageSource => {
            p.circle_stroke(at, 8.0 * z, st);
            line(-14., 0., -8., 0.);
            line(8., 0., 14., 0.);
            line(-5., 0., -1., 0.);
            line(0., 0., 5., 0.);
            line(2.5, -2.5, 2.5, 2.5);
        }
    }
}
