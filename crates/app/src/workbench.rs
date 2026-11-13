use crate::{
    canvas::{Camera, MoveDrag, WireSplice},
    component_help,
    style::*,
};
use airflow::{
    model::*,
    solver::{
        SolveError,
        steady::{self, OperatingPoint},
    },
    storage,
};
use eframe::egui::Event as UiEvent;
use eframe::egui::*;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use uuid::Uuid;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum View {
    Building,
    Circuit,
    Results,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Part {
    Room,
    Boundary,
    Atmosphere,
    Junction,
    Inlet,
    Door,
    Leakage,
    TransferAir,
    SupplyAir,
    Duct,
    Riser,
    Supply,
    Exhaust,
    Transfer,
    Pressure,
}

#[derive(Clone)]
pub(crate) enum LibraryDrag {
    Part(Part),
    Preset(crate::presets::NodePreset),
}
impl Part {
    pub fn name(self) -> &'static str {
        match self {
            Self::Room => "Room / pressure zone",
            Self::Boundary => "Fixed pressure",
            Self::Atmosphere => "Atmosphere",
            Self::Junction => "Junction",
            Self::Inlet => "Passive opening",
            Self::Door => "Door",
            Self::Leakage => "Air leakage",
            Self::TransferAir => "Transfer grille",
            Self::SupplyAir => "Supply air terminal",
            Self::Duct => "Duct segment",
            Self::Riser => "Vertical riser",
            Self::Supply => "Supply fan",
            Self::Exhaust => "Exhaust fan",
            Self::Transfer => "Transfer fan",
            Self::Pressure => "Pressure fan",
        }
    }
    pub fn mark(self) -> &'static str {
        match self {
            Self::Room => "RM",
            Self::Boundary => "P₀",
            Self::Atmosphere => "ATM",
            Self::Junction => "JN",
            Self::Inlet => "OP",
            Self::Door => "DR",
            Self::Leakage => "LK",
            Self::TransferAir => "TA",
            Self::SupplyAir => "SA",
            Self::Duct => "DX",
            Self::Riser => "VR",
            Self::Supply => "SF",
            Self::Exhaust => "EF",
            Self::Transfer => "TF",
            Self::Pressure => "PF",
        }
    }
    pub fn system(self) -> System {
        match self {
            Self::Inlet => System::Outdoor,
            Self::Supply | Self::SupplyAir => System::Supply,
            Self::Exhaust => System::Exhaust,
            Self::Transfer | Self::Door | Self::Leakage | Self::TransferAir => System::Transfer,
            _ => System::Unassigned,
        }
    }
    pub fn node(self) -> Option<NodeKind> {
        match self {
            Self::Room => Some(NodeKind::Room {
                volume_m3: 100.0,
                initial_pa: 0.0,
            }),
            Self::Boundary => Some(NodeKind::Boundary { gauge_pa: 0.0 }),
            Self::Atmosphere => Some(NodeKind::Atmosphere),
            Self::Junction => Some(NodeKind::Junction),
            _ => None,
        }
    }
    pub fn aperture(self) -> Option<Aperture> {
        let area_m2 = match self {
            Self::Door => 1.8,
            Self::Leakage => 0.003,
            Self::TransferAir => 0.10,
            _ => return None,
        };
        Some(Aperture {
            area_m2,
            discharge: 0.65,
            open_fraction: 1.0,
        })
    }
    pub fn branch(self) -> BranchKind {
        match self {
            Self::Inlet | Self::Door | Self::Leakage | Self::TransferAir => {
                BranchKind::Opening { k: 1800.0 }
            }
            Self::Duct | Self::Riser => BranchKind::Duct {
                duct: Duct::default(),
                riser: self == Self::Riser,
            },
            Self::Pressure => BranchKind::PressureFan {
                shutoff_pa: 75.0,
                k: 1800.0,
                tau_s: 5.0,
            },
            _ => BranchKind::FlowFan {
                design_m3_s: 600.0 / M3_H,
                tau_s: 5.0,
            },
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Overlay {
    Floor,
    System,
    Pressure,
    Magnitude,
    Direction,
    Fan,
}
impl Overlay {
    pub const ALL: [Self; 6] = [
        Self::Floor,
        Self::System,
        Self::Pressure,
        Self::Magnitude,
        Self::Direction,
        Self::Fan,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Floor => "Floor colors",
            Self::System => "System colors",
            Self::Pressure => "Room pressure",
            Self::Magnitude => "Flow magnitude",
            Self::Direction => "Flow direction",
            Self::Fan => "Fan status",
        }
    }
}
struct Worker {
    inbox: mpsc::Receiver<Result<OperatingPoint, SolveError>>,
    cancel: Arc<AtomicBool>,
}
pub struct Workbench {
    pub project: Project,
    pub active_floor: Uuid,
    pub view: View,
    pub selection: Vec<Uuid>,
    pub multi_select: bool,
    pub tool: Option<Part>,
    pub from: Option<Uuid>,
    pub port_drag: Option<(Uuid, PortSide, Part)>,
    pub reconnect: Option<(Uuid, bool)>,
    pub cameras: [Camera; 2],
    pub split: Option<(Uuid, Camera)>,
    pub split_ratio: f32,
    pub secondary_active: bool,
    pub split_rendering: bool,
    pub resize: Option<(Uuid, Pos2, Vec2, Project)>,
    pub pending_floor_delete: Option<Uuid>,
    pub(crate) presets: Vec<crate::presets::NodePreset>,
    pub(crate) preset_draft: Option<crate::presets::NodePreset>,
    pub(crate) placing_preset: Option<crate::presets::NodePreset>,
    pub(crate) library_press: Option<(LibraryDrag, Pos2)>,
    pub focus_circuit: bool,
    pub(crate) dragging: Option<MoveDrag>,
    pub(crate) draw_container: bool,
    pub(crate) container_start: Option<Pos2>,
    pub canvas_press: Option<(Id, Pos2)>,
    pub box_select: Option<(Pos2, bool)>,
    pub search: String,
    pub all_floors: bool,
    pub isolate: bool,
    pub route: bool,
    pub elevation_filter: bool,
    pub elevation_m: f64,
    pub elevation_band: f64,
    pub profile: bool,
    pub show_suggestions: bool,
    pub plan_visible: bool,
    pub plan_opacity: f32,
    pub plan_texture: HashMap<Uuid, (String, TextureHandle)>,
    pub overlay: Overlay,
    pub results: Option<OperatingPoint>,
    pub(crate) startup: crate::startup::Startup,
    pub flow_cfm: bool,
    pub settings: bool,
    pub equations: bool,
    pub status: String,
    pub status_error: bool,
    pub clipboard: Option<(String, NodeKind, [f32; 2])>,
    undo: Vec<Project>,
    redo: Vec<Project>,
    path: Option<PathBuf>,
    saved: Option<Project>,
    autosave_at: Instant,
    recovery_session: Option<crate::recovery::RecoverySession>,
    autosaved: Option<Project>,
    recovery_entries: Option<Vec<crate::recovery::RecoveryEntry>>,
    #[cfg(windows)]
    updater: crate::update::Updater,
    worker: Option<Worker>,
    pub(crate) solve_requested: bool,
    pub(crate) fan_slider_edit: Option<Project>,
    close_pending: bool,
    allow_close: bool,
}
const SINGLE: &[u8] = include_bytes!("../../../examples/single-room.airflow.json");
const TWO: &[u8] = include_bytes!("../../../examples/two-floor.airflow.json");

impl Workbench {
    pub fn new(ctx: &Context, path: Option<PathBuf>) -> Self {
        let mut warning = None;
        let (project, path) = match path {
            Some(path) => match storage::load(&path) {
                Ok(p) => (p, Some(path)),
                Err(e) => {
                    warning = Some(e.to_string());
                    (Project::new("Untitled project"), None)
                }
            },
            None => (Project::new("Untitled project"), None),
        };
        install(ctx, project.drawing.light);
        let selection = project
            .network
            .nodes
            .iter()
            .find(|n| matches!(n.kind, NodeKind::Room { .. }))
            .map(|n| n.id)
            .into_iter()
            .collect();
        let recoverable = crate::recovery::available();
        let mut this = Self {
            active_floor: project.network.floors[0].id,
            saved: Some(project.clone()),
            project,
            view: View::Building,
            selection,
            multi_select: false,
            tool: None,
            from: None,
            port_drag: None,
            reconnect: None,
            cameras: [Camera::default(); 2],
            split: None,
            split_ratio: 0.5,
            secondary_active: false,
            split_rendering: false,
            resize: None,
            pending_floor_delete: None,
            presets: crate::presets::load(),
            preset_draft: None,
            placing_preset: None,
            library_press: None,
            focus_circuit: false,
            dragging: None,
            draw_container: false,
            container_start: None,
            canvas_press: None,
            box_select: None,
            search: String::new(),
            all_floors: true,
            isolate: false,
            route: true,
            elevation_filter: false,
            elevation_m: 0.0,
            elevation_band: 0.8,
            profile: false,
            show_suggestions: true,
            plan_visible: true,
            plan_opacity: 0.85,
            plan_texture: HashMap::new(),
            overlay: Overlay::Floor,
            results: None,
            startup: crate::startup::Startup::default(),
            flow_cfm: true,
            settings: false,
            equations: false,
            status: if path.is_some() {
                "Project opened"
            } else {
                "Blank workspace · open a project or start building"
            }
            .into(),
            status_error: false,
            clipboard: None,
            undo: Vec::new(),
            redo: Vec::new(),
            path,
            autosave_at: Instant::now(),
            recovery_session: None,
            autosaved: None,
            recovery_entries: (!recoverable.is_empty()).then_some(recoverable),
            #[cfg(windows)]
            updater: crate::update::Updater::default(),
            worker: None,
            solve_requested: false,
            fan_slider_edit: None,
            close_pending: false,
            allow_close: false,
        };
        if let Some(warning) = warning {
            this.notice(warning, true);
        } else if this.path.is_some() {
            if !this.project.drawing.floor_plans.is_empty() {
                this.overlay = Overlay::System;
                this.show_suggestions = false;
                this.fit_next(0, ctx);
                this.fit_next(1, ctx);
            }
            if !this.project.network.branches.is_empty() {
                this.run(ctx);
            }
        }
        this
    }
    pub fn picked(&self) -> Option<Uuid> {
        self.selection.last().copied()
    }
    pub fn notice(&mut self, text: impl Into<String>, error: bool) {
        self.status = text.into();
        self.status_error = error;
    }
    pub fn checkpoint(&mut self, previous: Project) {
        if previous == self.project {
            return;
        }
        self.undo.push(previous);
        if self.undo.len() > 60 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }
    fn history(&mut self, redo: bool) {
        let (source, target) = if redo {
            (&mut self.redo, &mut self.undo)
        } else {
            (&mut self.undo, &mut self.redo)
        };
        if let Some(p) = source.pop() {
            target.push(std::mem::replace(&mut self.project, p));
            self.reconcile();
        }
    }
    fn reconcile(&mut self) {
        if self.project.network.floor(self.active_floor).is_none() {
            self.active_floor = self.project.network.floors[0].id;
        }
        if self
            .split
            .is_some_and(|(floor, _)| self.project.network.floor(floor).is_none())
        {
            self.split = None;
            self.secondary_active = false;
        }
        self.selection.retain(|id| {
            self.project.network.node(*id).is_some()
                || self.project.network.branch(*id).is_some()
                || self.project.drawing.containers.contains_key(id)
        });
        self.tool = None;
        self.placing_preset = None;
        self.library_press = None;
        self.from = None;
        self.port_drag = None;
        self.box_select = None;
        self.canvas_press = None;
        self.dragging = None;
        self.draw_container = false;
        self.container_start = None;
        self.resize = None;
        self.reconnect = None;
    }
    fn dirty(&self) -> bool {
        self.saved.as_ref() != Some(&self.project)
    }
    fn load_project(&mut self, ctx: &Context, p: Project, path: Option<PathBuf>) -> bool {
        if self.dirty() && !self.recovery() {
            return false;
        }
        self.recovery_session = None;
        self.autosaved = None;
        self.autosave_at = Instant::now();
        if let Some(w) = self.worker.take() {
            w.cancel.store(true, Ordering::Relaxed);
        }
        self.project = p;
        self.plan_texture.clear();
        self.split = None;
        self.secondary_active = false;
        self.plan_visible = true;
        self.show_suggestions = self.project.drawing.floor_plans.is_empty();
        self.solve_requested = false;
        self.fan_slider_edit = None;
        self.saved = Some(self.project.clone());
        self.path = path;
        self.results = None;
        self.startup = crate::startup::Startup::default();
        self.undo.clear();
        self.redo.clear();
        self.selection.clear();
        self.multi_select = false;
        self.active_floor = self.project.network.floors[0].id;
        self.cameras = [Camera::default(); 2];
        self.view = View::Building;
        self.focus_circuit = false;
        self.reconcile();
        install(ctx, self.project.drawing.light);
        self.notice("Project opened", false);
        if !self.project.drawing.floor_plans.is_empty() {
            self.overlay = Overlay::System;
            self.fit_next(0, ctx);
            self.fit_next(1, ctx);
        }
        if !self.project.network.branches.is_empty() {
            self.run(ctx);
        }
        true
    }
    pub fn plan_path(&self, image: &str) -> PathBuf {
        self.path
            .as_deref()
            .and_then(std::path::Path::parent)
            .unwrap_or_else(|| std::path::Path::new("."))
            .join(image)
    }
    fn open(&mut self, ctx: &Context) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Airframe project", &["json"])
            .pick_file()
        {
            match storage::load(&path) {
                Ok(p) => {
                    self.load_project(ctx, p, Some(path));
                }
                Err(e) => self.notice(e.to_string(), true),
            }
        }
    }
    fn save(&mut self, save_as: bool) {
        let path = if save_as || self.path.is_none() {
            rfd::FileDialog::new()
                .add_filter("Airframe project", &["json"])
                .set_file_name("project.airflow.json")
                .save_file()
        } else {
            self.path.clone()
        };
        if let Some(mut path) = path {
            if path.extension().is_none() {
                path.set_extension("airflow.json");
            }
            let mut project = self.project.clone();
            let parent = path.parent().unwrap_or_else(|| std::path::Path::new("."));
            for plan in project.drawing.floor_plans.values_mut() {
                let source = self.plan_path(&plan.image);
                if parent.join(&plan.image) != source {
                    match source.canonicalize() {
                        Ok(absolute) => plan.image = absolute.to_string_lossy().into_owned(),
                        Err(error) => {
                            self.notice(format!("Could not locate the floor plan: {error}"), true);
                            return;
                        }
                    }
                }
            }
            match storage::save(&path, &project) {
                Ok(()) => {
                    self.project = project;
                    self.path = Some(path);
                    self.saved = Some(self.project.clone());
                    self.notice("Project saved", false);
                    self.clear_recovery();
                }
                Err(e) => self.notice(e.to_string(), true),
            }
        }
    }
    fn add_floor(&mut self) {
        let prev = self.project.clone();
        let n = self.project.network.floors.len();
        let z = self
            .project
            .network
            .floors
            .iter()
            .map(|f| f.elevation_m)
            .fold(0.0_f64, f64::max)
            + 4.0;
        let f = Floor::new(&format!("Level {:02}", n + 1), z);
        self.active_floor = f.id;
        self.project.network.floors.push(f);
        self.checkpoint(prev);
    }
    pub fn floor_editable(&self, id: Uuid) -> bool {
        self.project
            .network
            .floor(id)
            .is_some_and(|f| !f.locked && f.visible)
    }
    pub fn component_editable(&self, id: Uuid) -> bool {
        if let Some(container) = self.project.drawing.containers.get(&id) {
            return container.floor == self.active_floor && self.floor_editable(container.floor);
        }
        let net = &self.project.network;
        if let Some(n) = net.node(id) {
            return n.floor == self.active_floor && self.floor_editable(n.floor);
        }
        net.branch(id).is_some_and(|b| {
            net.node(b.from)
                .is_some_and(|n| n.floor == self.active_floor && self.floor_editable(n.floor))
        })
    }
    pub fn active_floor_status(&self) -> &'static str {
        if self.floor_editable(self.active_floor) {
            "EDITING"
        } else {
            "READ ONLY"
        }
    }
    pub fn endpoint_available(&self, id: Uuid, part: Part, destination: bool) -> bool {
        self.component_editable(id)
            || (destination
                && part == Part::Riser
                && self.project.network.node(id).is_some_and(|n| {
                    self.project
                        .network
                        .floor(n.floor)
                        .is_some_and(|f| f.visible)
                }))
    }
    pub fn remove_selected(&mut self) {
        let prev = self.project.clone();
        for id in self.selection.clone() {
            if self.component_editable(id) {
                if self.project.drawing.containers.remove(&id).is_some() {
                    continue;
                }
                if self.project.network.node(id).is_some()
                    && self
                        .project
                        .network
                        .branches
                        .iter()
                        .any(|b| (b.from == id || b.to == id) && !self.component_editable(b.id))
                {
                    self.notice("A branch on another floor is attached. Remove or reconnect it on its source floor first.", true);
                    continue;
                }
                if !self.project.heal_inline(id) && !self.project.heal_node(id) {
                    self.project.remove(id);
                }
            }
        }
        self.reconcile();
        self.checkpoint(prev);
    }
    pub fn set_tool(&mut self, tool: Part) {
        self.draw_container = false;
        self.container_start = None;
        self.placing_preset = None;
        if self.view != View::Building {
            self.view = View::Building;
        }
        if self.tool != Some(tool) {
            self.from = None;
        }
        self.tool = Some(tool);
        self.reconnect = None;
    }
    pub fn place(&mut self, part: Part, xy: [f32; 2]) {
        if !self.floor_editable(self.active_floor) {
            self.notice("Choose a visible, unlocked floor.", true);
            return;
        }
        let Some(kind) = part.node() else {
            return;
        };
        let prev = self.project.clone();
        let index = self
            .project
            .network
            .nodes
            .iter()
            .filter(|n| n.kind.label() == kind.label())
            .count()
            + 1;
        let id = self.project.add_node(
            &format!("{} {:02}", kind.label(), index),
            self.active_floor,
            kind,
            xy,
        );
        self.selection = vec![id];
        self.tool = None;
        self.checkpoint(prev);
    }
    pub fn place_riser(&mut self, xy: [f32; 2]) {
        self.place_riser_at(xy, None, None);
    }
    pub fn place_riser_at(
        &mut self,
        xy: [f32; 2],
        wire: Option<WireSplice>,
        endpoint: Option<Uuid>,
    ) {
        if !self.floor_editable(self.active_floor) {
            return;
        }
        let net = &self.project.network;
        let Some(source) = net.floor(self.active_floor) else {
            return;
        };
        let from_floor = source.id;
        let elevation = source.elevation_m;
        let rise: f64 = 3.0;
        let previous = self.project.clone();
        let from = endpoint
            .filter(|id| self.component_editable(*id))
            .unwrap_or_else(|| {
                self.project
                    .add_node("Riser landing", from_floor, NodeKind::Junction, xy)
            });
        if let Some(route) = wire {
            let Some(tail) = self.project.split_branch(route.wire, from, route.fraction) else {
                self.project = previous;
                return;
            };
            self.keep_wire_bend(route.wire, route, 0.0, route.fraction);
            self.keep_wire_bend(tail, route, route.fraction, 1.0);
        }
        let to = self.project.add_node(
            "Riser landing",
            from_floor,
            NodeKind::Junction,
            [xy[0] + 175.0, xy[1]],
        );
        let duct = Duct {
            length_m: rise.max(0.1),
            ..Duct::default()
        };
        let riser = self.project.add_branch(
            "Vertical riser",
            from,
            to,
            System::Unassigned,
            BranchKind::Duct { duct, riser: true },
        );
        self.project
            .network
            .branches
            .last_mut()
            .unwrap()
            .end_elevation_m = elevation + rise;
        self.project.drawing.riser_landings.insert(to, riser);
        if endpoint.is_none() {
            self.project.drawing.riser_landings.insert(from, riser);
        }
        self.selection = vec![if endpoint.is_some() { to } else { from }];
        self.tool = None;
        self.from = None;
        self.notice(
            "Riser placed. Set its destination floor in the inspector.",
            false,
        );
        self.checkpoint(previous);
    }
    pub fn riser_drop_available(&self) -> bool {
        self.floor_editable(self.active_floor)
    }
    pub fn set_riser_destination(&mut self, id: Uuid, floor: Uuid) {
        let net = &self.project.network;
        let Some(riser) = net
            .branch(id)
            .filter(|b| matches!(b.kind, BranchKind::Duct { riser: true, .. }))
        else {
            return;
        };
        let (Some(source), Some(destination)) = (net.node(riser.from), net.floor(floor)) else {
            return;
        };
        if !self.component_editable(id) || !destination.visible {
            return;
        }
        let from_floor = source.floor;
        let old_end = riser.to;
        if net.node(old_end).is_some_and(|n| n.floor == floor) {
            self.go_to_landing(old_end);
            return;
        }
        let elevation = if source.floor == floor {
            riser.start_elevation_m + 3.0
        } else {
            destination.elevation_m
        };
        let pos = self
            .project
            .drawing
            .positions
            .get(&riser.from)
            .copied()
            .unwrap_or_default();
        let old_generated = self.project.drawing.riser_landings.get(&old_end) == Some(&id);
        let reusable = old_generated
            && !net
                .branches
                .iter()
                .any(|b| b.id != id && (b.from == old_end || b.to == old_end));
        let xy = if floor == from_floor {
            [pos[0] + 175.0, pos[1]]
        } else {
            pos
        };
        let new_end = if reusable {
            let landing = self
                .project
                .network
                .nodes
                .iter_mut()
                .find(|n| n.id == old_end)
                .expect("generated landing");
            landing.floor = floor;
            self.project.drawing.positions.insert(old_end, xy);
            old_end
        } else {
            if old_generated {
                self.project.drawing.riser_landings.remove(&old_end);
            }
            self.project.add_riser_landing(id, floor, xy)
        };
        let riser = self
            .project
            .network
            .branches
            .iter_mut()
            .find(|b| b.id == id)
            .expect("existing riser");
        let previous_rise = (riser.end_elevation_m - riser.start_elevation_m).abs();
        riser.end_elevation_m = elevation;
        riser.to = new_end;
        if let BranchKind::Duct { duct, .. } = &mut riser.kind {
            let rise = (riser.end_elevation_m - riser.start_elevation_m).abs();
            // Preserve any length already allowed for bends or horizontal routing.
            duct.length_m = (duct.length_m + rise - previous_rise).max(rise).max(0.1);
        }
        self.go_to_landing(new_end);
    }
    pub fn go_to_landing(&mut self, id: Uuid) {
        let Some(node) = self.project.network.node(id) else {
            return;
        };
        if !self
            .project
            .network
            .floor(node.floor)
            .is_some_and(|f| f.visible)
        {
            self.notice("Show the connected floor before navigating to it.", true);
            return;
        }
        self.active_floor = node.floor;
        self.selection = vec![id];
        self.view = View::Building;
        self.tool = None;
        self.from = None;
        self.reconnect = None;
        let position = self
            .project
            .drawing
            .positions
            .get(&id)
            .copied()
            .unwrap_or_default();
        self.cameras[0].center_on(position);
    }
    pub fn connect(&mut self, part: Part, a: Uuid, b: Uuid) {
        self.connect_ports(part, a, b, [None, None]);
    }
    pub fn pick_connection_node(&mut self, part: Part, id: Uuid) {
        if let Some(from) = self.from {
            self.connect(part, from, id);
        } else if self.endpoint_available(id, part, false) {
            self.set_tool(part);
            self.from = Some(id);
        }
    }
    pub fn connect_ports(&mut self, part: Part, a: Uuid, b: Uuid, ports: [Option<PortSide>; 2]) {
        if a == b
            || !self.endpoint_available(a, part, false)
            || !self.endpoint_available(b, part, true)
        {
            self.notice(
                "Choose different endpoints on the active, unlocked floor. A riser can end on another visible floor.",
                true,
            );
            return;
        }
        let prev = self.project.clone();
        let id = self
            .project
            .add_branch(part.name(), a, b, part.system(), part.branch());
        self.project.network.branches.last_mut().unwrap().aperture = part.aperture();
        if ports.iter().any(Option::is_some) {
            self.project.drawing.ports.insert(id, ports);
        }
        self.selection = vec![id];
        self.tool = None;
        self.from = None;
        self.checkpoint(prev);
    }
    fn keep_wire_bend(&mut self, branch: Uuid, route: WireSplice, from: f64, to: f64) {
        if let Some(bend) = route.bend(from, to) {
            self.project.drawing.bends.insert(branch, bend);
        }
    }

    pub fn splice(&mut self, part: Part, route: WireSplice) {
        let WireSplice { wire, fraction, .. } = route;
        if !self.component_editable(wire) {
            return;
        }
        let original = self
            .project
            .network
            .branch(wire)
            .expect("editable branch")
            .clone();
        let floor = self
            .project
            .network
            .node(original.from)
            .expect("editable endpoint")
            .floor;
        let previous = self.project.clone();
        if let Some(kind) = part.node() {
            let id = self
                .project
                .add_node(kind.label(), floor, kind, route.point(fraction));
            let Some(tail) = self.project.split_branch(wire, id, fraction) else {
                self.project = previous;
                return;
            };
            self.keep_wire_bend(wire, route, 0.0, fraction);
            self.keep_wire_bend(tail, route, fraction, 1.0);
            self.selection = vec![id];
            if original.kind.tau().is_some() {
                self.notice("Node inserted. The downstream opening uses the default loss coefficient; review it in the inspector.", false);
            } else {
                self.notice("Node inserted in series. The original loss was divided between the two branches.", false);
            }
        } else {
            let (start, end) = route.inline_span();
            let before = self.project.add_node(
                "Inline inlet",
                floor,
                NodeKind::Junction,
                route.point(start),
            );
            let after =
                self.project
                    .add_node("Inline outlet", floor, NodeKind::Junction, route.point(end));
            for node in [before, after] {
                self.project.drawing.sizes.insert(node, [70.0, 52.0]);
            }
            let Some(tail) = self.project.split_branch(wire, before, fraction) else {
                self.project = previous;
                return;
            };
            self.project
                .network
                .branches
                .iter_mut()
                .find(|b| b.id == tail)
                .expect("split tail")
                .from = after;
            let system = if part.system() == System::Unassigned {
                original.system
            } else {
                part.system()
            };
            let id = self
                .project
                .add_branch(part.name(), before, after, system, part.branch());
            let b = self
                .project
                .network
                .branches
                .iter_mut()
                .find(|b| b.id == id)
                .expect("inline branch");
            let z = original.start_elevation_m
                + fraction.clamp(0.05, 0.95)
                    * (original.end_elevation_m - original.start_elevation_m);
            b.aperture = part.aperture();
            self.project
                .drawing
                .inline_nodes
                .extend([(before, id), (after, id)]);
            b.start_elevation_m = z;
            b.end_elevation_m = z;
            self.keep_wire_bend(wire, route, 0.0, start);
            self.keep_wire_bend(id, route, start, end);
            self.keep_wire_bend(tail, route, end, 1.0);
            self.selection = vec![id];
            self.notice(if original.kind.tau().is_some() {
                "Component inserted in series with the fan. Review the default downstream opening."
            } else { "Component inserted in series." }, false);
        }
        self.tool = None;
        self.from = None;
        self.checkpoint(previous);
    }
    pub fn replace_end(&mut self, branch: Uuid, is_from: bool, node: Uuid) {
        let kind = self.project.network.branch(branch).map(|b| b.kind);
        let part = if matches!(kind, Some(BranchKind::Duct { riser: true, .. })) {
            Part::Riser
        } else {
            Part::Duct
        };
        if !self.component_editable(branch) || !self.endpoint_available(node, part, !is_from) {
            return;
        }
        let prev = self.project.clone();
        let Some(b) = self
            .project
            .network
            .branches
            .iter_mut()
            .find(|b| b.id == branch)
        else {
            return;
        };
        if (if is_from { b.to } else { b.from }) == node {
            self.notice("Endpoints must be different.", true);
            return;
        }
        if is_from {
            b.from = node;
        } else {
            b.to = node;
        }
        if let Some(sides) = self.project.drawing.ports.get_mut(&branch) {
            sides[usize::from(!is_from)] = None;
        }
        self.reconnect = None;
        self.checkpoint(prev);
    }
    pub fn color(&self, system: System) -> Color32 {
        let [r, g, b] = self.project.drawing.colors[system as usize];
        Color32::from_rgb(r, g, b)
    }
    pub fn floor_color(&self, id: Uuid) -> Color32 {
        let [r, g, b] = self
            .project
            .drawing
            .floor_colors
            .get(&id)
            .copied()
            .unwrap_or_else(|| {
                floor_default(
                    self.project
                        .network
                        .floors
                        .iter()
                        .position(|f| f.id == id)
                        .unwrap_or(0),
                )
            });
        Color32::from_rgb(r, g, b)
    }
    pub fn current_run(&self) -> Option<&OperatingPoint> {
        self.results
            .as_ref()
            .filter(|r| r.matches(&self.project.network))
    }
    pub fn pressure(&self, id: Uuid) -> Option<f64> {
        if let Some((run, sample)) = self.startup_sample() {
            return run
                .snapshot
                .nodes
                .iter()
                .position(|n| n.id == id)
                .map(|i| sample.pressures[i]);
        }
        let run = self.current_run()?;
        let i = run.snapshot.nodes.iter().position(|n| n.id == id)?;
        Some(run.pressures[i])
    }
    pub fn flow(&self, id: Uuid) -> Option<f64> {
        if let Some((run, sample)) = self.startup_sample() {
            return run
                .snapshot
                .branches
                .iter()
                .position(|b| b.id == id)
                .map(|i| sample.flows[i]);
        }
        let run = self.current_run()?;
        let i = run.snapshot.branches.iter().position(|b| b.id == id)?;
        Some(run.flows[i])
    }
    pub fn flow_unit(&self) -> (&'static str, f64) {
        if self.flow_cfm {
            ("CFM", 1.0 / CFM)
        } else {
            ("m³/h", M3_H)
        }
    }
    fn run(&mut self, ctx: &Context) {
        if self.worker.is_some() {
            return;
        }
        self.solve_requested = false;
        let net = self.project.network.clone();
        let (send, inbox) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let ctx = ctx.clone();
        self.worker = Some(Worker { inbox, cancel });
        self.notice("Balancing airflow…", false);
        std::thread::spawn(move || {
            let result = steady::solve(&net, || !flag.load(Ordering::Relaxed));
            let _ = send.send(result);
            ctx.request_repaint();
        });
    }
    fn poll(&mut self) {
        let Some(w) = &self.worker else {
            return;
        };
        match w.inbox.try_recv() {
            Ok(result) => {
                self.worker = None;
                match result {
                    Ok(point) => {
                        let text = if !point.matches(&self.project.network) {
                            "Inputs changed during the solve. Solve airflow again."
                        } else {
                            "Operating point solved"
                        };
                        self.notice(text, false);
                        self.results = Some(point);
                    }
                    Err(SolveError::Cancelled) => self.notice("Solve cancelled", false),
                    Err(e) => self.notice(e.to_string(), true),
                }
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.worker = None;
                self.notice("Solver worker stopped unexpectedly.", true);
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
    }
    fn toolbar(&mut self, ctx: &Context, p: Palette) {
        TopBottomPanel::top("toolbar")
            .frame(panel(p, 12))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    Frame::new()
                        .fill(p.accent)
                        .corner_radius(7)
                        .inner_margin(Margin::symmetric(10, 7))
                        .show(ui, |ui| {
                            ui.label(RichText::new("AF").monospace().strong().color(p.canvas));
                        });
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.label(RichText::new("AIRFRAME").size(16.0).strong());
                        eyebrow(ui, "VENTILATION WORKBENCH", p);
                    });
                    ui.add_space(12.0);
                    ui.menu_button("File", |ui| {
                        if ui.button("New project").clicked() {
                            self.load_project(ctx, Project::new("Untitled project"), None);
                            ui.close();
                        }
                        if ui.button("Open project…").clicked() {
                            self.open(ctx);
                            ui.close();
                        }
                        if ui.button("Save project").clicked() {
                            self.save(false);
                            ui.close();
                        }
                        if ui.button("Save as…").clicked() {
                            self.save(true);
                            ui.close();
                        }
                        if ui.button("Recover unsaved work…").clicked() {
                            self.recovery_entries = Some(crate::recovery::available());
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Single-room example").clicked() {
                            self.load_project(ctx, storage::decode(SINGLE).unwrap(), None);
                            ui.close();
                        }
                        if ui.button("Two-floor example").clicked() {
                            self.load_project(ctx, storage::decode(TWO).unwrap(), None);
                            ui.close();
                        }
                    });
                    if ui
                        .add_enabled(!self.undo.is_empty(), Button::new("Undo"))
                        .on_hover_text("Ctrl+Z")
                        .clicked()
                    {
                        self.history(false);
                    }
                    if ui
                        .add_enabled(!self.redo.is_empty(), Button::new("Redo"))
                        .on_hover_text("Ctrl+Y")
                        .clicked()
                    {
                        self.history(true);
                    }
                    ui.separator();
                    for (view, label) in [
                        (View::Building, "Building"),
                        (View::Circuit, "Circuit"),
                        (View::Results, "Results"),
                    ] {
                        if ui.selectable_label(self.view == view, label).clicked() {
                            self.view = view;
                            self.draw_container = false;
                            self.container_start = None;
                            self.tool = None;
                            self.placing_preset = None;
                            self.from = None;
                            self.reconnect = None;
                            if view == View::Circuit {
                                self.fit_next(1, ctx);
                            }
                        }
                    }
                    if self.view == View::Building
                        && ui
                            .add_enabled(
                                self.project.network.floors.len() > 1,
                                Button::new(if self.split.is_some() {
                                    "Close split"
                                } else {
                                    "Split levels"
                                }),
                            )
                            .clicked()
                    {
                        if self.split.take().is_none()
                            && let Some(other) = self
                                .project
                                .network
                                .floors
                                .iter()
                                .find(|f| f.id != self.active_floor)
                        {
                            self.split = Some((other.id, Camera::default()));
                            ctx.data_mut(|d| {
                                d.insert_temp(Id::new(("split-fit", false)), true);
                                d.insert_temp(Id::new(("split-fit", true)), true);
                            });
                        }
                        self.secondary_active = false;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if let Some(w) = &self.worker {
                            if ui.button("Cancel solve").clicked() {
                                w.cancel.store(true, Ordering::Relaxed);
                                self.solve_requested = false;
                            }
                        } else if ui
                            .add(
                                Button::new(
                                    RichText::new("Solve airflow").strong().color(p.canvas),
                                )
                                .fill(p.accent),
                            )
                            .clicked()
                        {
                            self.run(ctx);
                        }
                        if ui
                            .add_enabled(self.worker.is_none(), Button::new("Reset"))
                            .on_hover_text("Clear the operating point")
                            .clicked()
                        {
                            self.results = None;
                        }
                        if ui.button("Settings").clicked() {
                            self.settings = !self.settings;
                        }
                        ui.menu_button(concat!("v", env!("CARGO_PKG_VERSION")), |ui| {
                            #[cfg(windows)]
                            if self.updater.installed() {
                                if ui.button("Check for updates").clicked() {
                                    self.updater.check(ctx, true);
                                    ui.close();
                                }
                            } else {
                                ui.label(
                                    "Automatic updates are available in the Windows installation.",
                                );
                            }
                            ui.hyperlink_to(
                                "Airframe releases",
                                "https://github.com/Hola3Dprint/Airframe/releases",
                            );
                        });
                    });
                });
            });
    }
    fn library(&mut self, ctx: &Context, p: Palette) {
        SidePanel::left("library").default_width(218.0).width_range(172.0..=310.0).frame(panel(p, 13)).show(ctx, |ui| {
            ScrollArea::vertical().scroll_source(scroll_area::ScrollSource::SCROLL_BAR | scroll_area::ScrollSource::MOUSE_WHEEL).show(ui, |ui| {
                eyebrow(ui, "01  /  BUILD", p);
                heading(ui, "Component library");
                ui.add(TextEdit::singleline(&mut self.search).hint_text("Search components…").desired_width(f32::INFINITY));
                let query = self.search.to_lowercase();
                for (group, parts) in [
                    ("PRESSURE NODES", &[Part::Room,Part::Boundary,Part::Atmosphere,Part::Junction][..]),
                    ("DUCTWORK", &[Part::Inlet,Part::Duct,Part::Riser][..]),
                    ("AIR PATHS", &[Part::Door,Part::Leakage,Part::TransferAir,Part::SupplyAir][..]),
                    ("EQUIPMENT", &[Part::Supply,Part::Exhaust,Part::Transfer,Part::Pressure][..]),
                ] {
                    if !parts.iter().any(|part| part.name().to_lowercase().contains(&query)) { continue; }
                    ui.add_space(5.0); eyebrow(ui, group, p);
                    for &part in parts.iter().filter(|part| part.name().to_lowercase().contains(&query)) {
                        let accent = if part.node().is_some() || part.system() == System::Unassigned { p.accent } else { self.color(part.system()) };
                        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click_and_drag());
                        let active = self.tool == Some(part);
                        ui.painter().rect(rect, 6, if active || response.hovered() { p.raised } else { p.surface }, Stroke::new(1.0_f32, if active { p.accent } else { p.edge }), StrokeKind::Inside);
                        ui.painter().text(rect.left_center()+vec2(9.0,0.0), Align2::LEFT_CENTER, part.mark(), FontId::monospace(11.0), accent);
                        ui.painter().text(rect.left_center()+vec2(38.0,0.0), Align2::LEFT_CENTER, part.name(), FontId::proportional(12.0), p.text);
                        if response.clicked() { self.set_tool(part); }
                        if let Some(origin) = primary_press_in(ui, response.rect) {
                            self.library_press = Some((LibraryDrag::Part(part), origin));
                        }
                        if response.hovered() { ctx.set_cursor_icon(CursorIcon::Grab); }
                        response.on_hover_ui(|ui| component_help::tooltip(
                            ui, part.name(), component_help::summary(part), component_help::library_usage(part),
                        ));
                    }
                }
                self.custom_library(ui, p, &query);
                ui.add_space(6.0); ui.separator();
                ui.horizontal(|ui| { eyebrow(ui,"FLOORS",p); if ui.small_button("+ Add").clicked() { self.add_floor(); } });
                let prev = self.project.clone();
                for (index, floor) in self.project.network.floors.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        let [r,g,b] = self.project.drawing.floor_colors.get(&floor.id).copied().unwrap_or_else(|| floor_default(index));
                        let color = Color32::from_rgb(r,g,b);
                        ui.colored_label(color, "●");
                        let floor_button = ui.selectable_label(self.active_floor == floor.id, &floor.name);
                        if floor_button.clicked() { self.active_floor = floor.id; }
                        floor_button.context_menu(|ui| {
                            if ui.button("Delete level…").clicked() {
                                self.pending_floor_delete = Some(floor.id);
                                ui.close();
                            }
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.toggle_value(&mut floor.locked, "Lock").on_hover_text("Lock floor editing");
                            ui.toggle_value(&mut floor.visible, "Show").on_hover_text("Show floor");
                        });
                    });
                }
                self.checkpoint(prev);
                if ui.add_enabled(self.project.network.floors.len() > 1, Button::new("Delete current level…")).clicked() {
                    self.pending_floor_delete = Some(self.active_floor);
                }
                ui.checkbox(&mut self.all_floors, "Show background floors");
                ui.add_space(4.0);
                ui.label(RichText::new("Drag a card to move it. Drag a side port to connect it to another node.").size(11.5).color(p.muted));
            });
        });
    }
    // Track where the gesture started, even if a quick drag and release is delivered in one frame.
    fn start_library_drag(&mut self, ctx: &Context) {
        let Some((_, origin)) = self.library_press.as_ref() else {
            return;
        };
        let (crossed_threshold, cancelled, held) = ctx.input(|i| {
            let moved = i
                .pointer
                .interact_pos()
                .is_some_and(|at| at.distance_sq(*origin) >= 36.0);
            (
                moved,
                i.key_pressed(Key::Escape) || !i.focused,
                i.pointer.primary_down(),
            )
        });
        if !cancelled && crossed_threshold {
            let (item, _) = self.library_press.take().expect("pending library press");
            self.view = View::Building;
            self.tool = None;
            self.draw_container = false;
            self.container_start = None;
            self.placing_preset = None;
            self.from = None;
            self.port_drag = None;
            self.reconnect = None;
            DragAndDrop::set_payload(ctx, item);
        } else if cancelled || !held {
            self.library_press = None;
        }
    }
    fn settings_window(&mut self, ctx: &Context, p: Palette) {
        if !self.settings {
            return;
        }
        let prev = self.project.clone();
        let mut open = self.settings;
        Window::new("Project settings")
            .open(&mut open)
            .default_width(400.0)
            .show(ctx, |ui| {
                ui.label("Project name");
                ui.text_edit_singleline(&mut self.project.name);
                ui.label("Project notes");
                ui.add(
                    TextEdit::multiline(&mut self.project.notes)
                        .desired_rows(4)
                        .desired_width(f32::INFINITY),
                );
                ui.separator();
                eyebrow(ui, "REFERENCE AIR", p);
                let air = &mut self.project.network.air;
                crate::inspector::number(ui, "Absolute pressure", &mut air.absolute_pa, "Pa", 20.0);
                crate::inspector::number(ui, "Density", &mut air.density, "kg/m³", 0.001);
                crate::inspector::number(ui, "Dynamic viscosity", &mut air.viscosity, "Pa·s", 1e-7);
                ui.separator();
                eyebrow(ui, "DISPLAY", p);
                ui.checkbox(&mut self.project.drawing.light, "Light theme");
                ui.checkbox(&mut self.flow_cfm, "Display airflow in CFM");
                ui.add(
                    Slider::new(&mut self.project.drawing.background_opacity, 0.1..=1.0)
                        .text("Background floor opacity"),
                );
                for (s, rgb) in System::ALL
                    .into_iter()
                    .zip(&mut self.project.drawing.colors)
                {
                    ui.horizontal(|ui| {
                        ui.color_edit_button_srgb(rgb);
                        ui.label(s.label());
                    });
                }
                ui.separator();
                eyebrow(ui, "FLOOR COLORS", p);
                for (index, floor) in self.project.network.floors.iter().enumerate() {
                    let mut color = self
                        .project
                        .drawing
                        .floor_colors
                        .get(&floor.id)
                        .copied()
                        .unwrap_or_else(|| floor_default(index));
                    ui.horizontal(|ui| {
                        if ui.color_edit_button_srgb(&mut color).changed() {
                            self.project.drawing.floor_colors.insert(floor.id, color);
                        }
                        ui.label(&floor.name);
                    });
                }
            });
        self.settings = open;
        self.checkpoint(prev);
    }

    fn floor_delete_window(&mut self, ctx: &Context) {
        let Some(id) = self.pending_floor_delete else {
            return;
        };
        let Some(floor) = self.project.network.floor(id) else {
            self.pending_floor_delete = None;
            return;
        };
        let name = floor.name.clone();
        let nodes: Vec<_> = self
            .project
            .network
            .nodes
            .iter()
            .filter(|n| n.floor == id)
            .map(|n| n.id)
            .collect();
        Window::new("Delete level")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(format!(
                    "Delete {} and its {} nodes, attached branches, and floor plan?",
                    name,
                    nodes.len()
                ));
                if self.project.network.floors.len() <= 1 {
                    ui.label("A project needs at least one level.");
                }
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            self.project.network.floors.len() > 1,
                            Button::new("Delete level"),
                        )
                        .clicked()
                    {
                        let previous = self.project.clone();
                        for node in nodes {
                            self.project.remove(node);
                        }
                        self.project.network.floors.retain(|f| f.id != id);
                        self.project
                            .drawing
                            .containers
                            .retain(|_, group| group.floor != id);
                        self.project.drawing.floor_plans.remove(&id);
                        self.project.drawing.floor_colors.remove(&id);
                        self.plan_texture.remove(&id);
                        self.reconcile();
                        self.fit_next(0, ctx);
                        self.checkpoint(previous);
                        self.notice("Level deleted · Undo to restore", false);
                        self.pending_floor_delete = None;
                    }
                    if ui.button("Cancel").clicked() {
                        self.pending_floor_delete = None;
                    }
                });
            });
    }

    pub(crate) fn import_plan(&mut self, ctx: &Context) {
        if !self.floor_editable(self.active_floor) {
            self.notice("Unlock the active level before importing a plan.", true);
            return;
        }
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Floor plan image", &["png", "jpg", "jpeg"])
            .pick_file()
        else {
            return;
        };
        match image::image_dimensions(&path) {
            Ok((w, h)) => {
                let previous = self.project.clone();
                let scale = 1500.0 / w.max(h) as f32;
                let size = [w as f32 * scale, h as f32 * scale];
                let (origin, size) = self
                    .project
                    .drawing
                    .floor_plans
                    .get(&self.active_floor)
                    .map_or(([-size[0] / 2.0, -size[1] / 2.0], size), |plan| {
                        (
                            plan.origin,
                            [plan.size[0], plan.size[0] * h as f32 / w as f32],
                        )
                    });
                self.project.drawing.floor_plans.insert(
                    self.active_floor,
                    FloorPlan {
                        image: path.to_string_lossy().into_owned(),
                        origin,
                        size,
                    },
                );
                self.plan_visible = true;
                self.plan_texture.remove(&self.active_floor);
                self.fit_next(0, ctx);
                self.checkpoint(previous);
                self.notice("Floor plan imported", false);
            }
            Err(error) => self.notice(format!("Could not read floor plan image: {error}"), true),
        }
    }
    fn recovery(&mut self) -> bool {
        if self.autosaved.as_ref() == Some(&self.project) {
            return true;
        }
        if self.recovery_session.is_none() {
            match crate::recovery::RecoverySession::start() {
                Ok(session) => self.recovery_session = Some(session),
                Err(error) => {
                    self.notice(format!("Could not start autosave · {error}"), true);
                    return false;
                }
            }
        }
        let original = self
            .path
            .as_ref()
            .map(|path| std::path::absolute(path).unwrap_or_else(|_| path.clone()));
        let snapshot = crate::recovery::Snapshot {
            original,
            project: self.project.clone(),
        };
        match self.recovery_session.as_ref().unwrap().write(&snapshot) {
            Ok(()) => {
                self.autosaved = Some(snapshot.project);
                true
            }
            Err(error) => {
                self.notice(format!("Could not write autosave · {error}"), true);
                false
            }
        }
    }

    fn clear_recovery(&mut self) {
        if let Some(session) = &self.recovery_session
            && let Err(error) = session.clear()
        {
            self.notice(format!("Could not clear autosave · {error}"), true);
            return;
        }
        self.autosaved = None;
    }

    fn recovery_window(&mut self, ctx: &Context) {
        let Some(entries) = self.recovery_entries.as_mut() else {
            return;
        };
        let mut open = true;
        let mut chosen = None;
        Window::new("Recover unsaved work")
            .open(&mut open)
            .collapsible(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                if entries.is_empty() {
                    ui.label("No recovery copies are available.");
                } else {
                    ui.label("Choose a recovery copy to reopen its unsaved edits.");
                    ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                        for (index, entry) in entries.iter().enumerate() {
                            ui.horizontal(|ui| {
                                if ui.button("Recover").clicked() {
                                    chosen = Some(index);
                                }
                                ui.vertical(|ui| {
                                    ui.strong(&entry.snapshot.project.name);
                                    if let Some(path) = &entry.snapshot.original {
                                        ui.small(path.display().to_string());
                                    } else {
                                        ui.small("Unsaved project");
                                    }
                                });
                            });
                        }
                    });
                }
            });
        if let Some(index) = chosen {
            let file = entries[index].file.clone();
            self.restore_recovery(ctx, &file);
            return;
        }
        if !open {
            self.recovery_entries = None;
        }
    }
    pub(crate) fn restore_recovery(&mut self, ctx: &Context, file: &std::path::Path) {
        let Some(entry) = self
            .recovery_entries
            .as_ref()
            .and_then(|entries| entries.iter().find(|entry| entry.file == file))
        else {
            return;
        };
        let original = entry.snapshot.original.clone();
        let project = entry.snapshot.project.clone();
        let saved = original
            .as_deref()
            .and_then(|path| storage::load(path).ok());
        if self.load_project(ctx, project, original) {
            self.saved = saved;
            self.recovery_entries = None;
            self.notice("Recovery copy opened", false);
            if self.recovery()
                && let Err(error) = crate::recovery::remove_snapshot(file)
            {
                self.notice(
                    format!("Could not clear the previous recovery copy · {error}"),
                    true,
                );
            }
        }
    }
    fn keys(&mut self, ctx: &Context) {
        if ctx.wants_keyboard_input() {
            return;
        }
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.placing_preset = None;
            self.tool = None;
            self.from = None;
            self.reconnect = None;
            self.port_drag = None;
            self.box_select = None;
            self.canvas_press = None;
            self.draw_container = false;
            self.container_start = None;
            if let Some(drag) = self.dragging.take() {
                self.project = drag.before;
            }
            if let Some((_, _, _, previous)) = self.resize.take() {
                self.project = previous;
            }
        }
        if ctx.input(|i| i.key_pressed(Key::Delete)) {
            self.remove_selected();
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Z)) {
            self.history(false);
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Y)) {
            self.history(true);
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::S)) {
            self.save(false);
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::O)) {
            self.open(ctx);
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::C))
            && let Some(n) = self.picked().and_then(|id| self.project.network.node(id))
        {
            self.clipboard = Some((
                n.name.clone(),
                n.kind,
                *self
                    .project
                    .drawing
                    .positions
                    .get(&n.id)
                    .unwrap_or(&[0.0, 0.0]),
            ));
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::V))
            && self.floor_editable(self.active_floor)
            && let Some((name, kind, xy)) = self.clipboard.clone()
        {
            let prev = self.project.clone();
            let id = self.project.add_node(
                &format!("{name} copy"),
                self.active_floor,
                kind,
                [xy[0] + 35.0, xy[1] + 35.0],
            );
            self.selection = vec![id];
            self.clipboard = Some((name, kind, [xy[0] + 35.0, xy[1] + 35.0]));
            self.checkpoint(prev);
        }
    }
}
fn floor_default(index: usize) -> [u8; 3] {
    const COLORS: [[u8; 3]; 8] = [
        [102, 177, 255],
        [244, 162, 97],
        [119, 216, 163],
        [200, 155, 250],
        [248, 214, 112],
        [92, 211, 225],
        [248, 150, 180],
        [165, 192, 128],
    ];
    COLORS[index % COLORS.len()]
}

pub(crate) fn primary_press_in(ui: &Ui, rect: Rect) -> Option<Pos2> {
    let visible = rect.intersect(ui.clip_rect());
    ui.input(|i| {
        i.events.iter().rev().find_map(|event| match event {
            UiEvent::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: true,
                ..
            } if visible.contains(*pos) => Some(*pos),
            _ => None,
        })
    })
    .filter(|pos| ui.ctx().layer_id_at(*pos) == Some(ui.layer_id()))
}

impl eframe::App for Workbench {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        #[cfg(windows)]
        {
            self.updater.check(ctx, false);
            if let Some((message, error)) = self.updater.poll() {
                self.notice(message, error);
            }
            if self.updater.ready() && !self.close_pending && !self.allow_close {
                let dirty = self.dirty();
                if !dirty || self.recovery() {
                    let project = self
                        .path
                        .as_ref()
                        .map(|path| std::path::absolute(path).unwrap_or_else(|_| path.clone()));
                    let resume = if dirty {
                        self.recovery_session
                            .as_ref()
                            .map(|session| session.snapshot_path())
                    } else {
                        None
                    };
                    match self.updater.launch(project.as_deref(), resume.as_deref()) {
                        Ok(()) => {
                            self.allow_close = true;
                            ctx.send_viewport_cmd(ViewportCommand::Close);
                            return;
                        }
                        Err(error) => {
                            self.notice(format!("Could not start the update · {error}"), true)
                        }
                    }
                } else {
                    self.updater.defer();
                    self.notice(
                        "Update deferred because unsaved work could not be backed up.",
                        true,
                    );
                }
            }
        }
        self.poll();
        self.poll_startup();
        self.keys(ctx);
        if ctx.input(|i| i.viewport().close_requested()) && self.dirty() && !self.allow_close {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.close_pending = true;
        }
        let light = self.project.drawing.light;
        let p = palette(light);
        self.toolbar(ctx, p);
        TopBottomPanel::bottom("status")
            .frame(panel(p, 6))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(tag(
                        if self.status_error { "CHECK" } else { "●" },
                        if self.status_error { p.warning } else { p.good },
                    ));
                    ui.label(
                        RichText::new(&self.status)
                            .size(11.5)
                            .color(if self.status_error {
                                p.warning
                            } else {
                                p.muted
                            }),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        eyebrow(ui, "SI MODEL  /  GAUGE PRESSURE", p);
                        ui.separator();
                        eyebrow(ui, if self.dirty() { "UNSAVED" } else { "WORKSPACE" }, p);
                    });
                });
            });
        self.library(ctx, p);
        self.start_library_drag(ctx);
        self.inspector(ctx, p);
        if self.view != View::Results {
            self.dock(ctx, p);
        }
        if self.solve_requested {
            self.run(ctx);
        }
        ctx.data_mut(|d| d.remove::<bool>(Id::new("airframe-drag-preview-painted")));
        CentralPanel::default()
            .frame(Frame::new().fill(p.canvas))
            .show(ctx, |ui| {
                if self.view == View::Results {
                    self.analysis_view(ui, p);
                } else if self.view == View::Building && self.split.is_some() {
                    self.split_canvas(ui, p);
                } else {
                    self.canvas(ui, p);
                }
            });
        self.drag_preview_outside_canvas(ctx, p);
        self.equation_window(ctx, p);
        self.settings_window(ctx, p);
        self.floor_delete_window(ctx);
        self.preset_window(ctx);
        self.recovery_window(ctx);
        #[cfg(windows)]
        self.updater.show(ctx);
        if self.close_pending {
            Window::new("Save changes?")
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label("This project has unsaved edits.");
                    ui.horizontal(|ui| {
                        if ui.button("Save and close").clicked() {
                            self.save(false);
                            if !self.dirty() {
                                self.allow_close = true;
                                ctx.send_viewport_cmd(ViewportCommand::Close);
                            }
                        }
                        if ui.button("Close without saving").clicked() {
                            self.clear_recovery();
                            self.allow_close = true;
                            ctx.send_viewport_cmd(ViewportCommand::Close);
                        }
                        if ui.button("Keep editing").clicked() {
                            self.close_pending = false;
                        }
                    });
                });
        }
        if light != self.project.drawing.light {
            install(ctx, self.project.drawing.light);
        }
        if !self.allow_close {
            if self.dirty() && self.autosaved.as_ref() != Some(&self.project) {
                let interval = Duration::from_secs(5);
                let remaining = interval.saturating_sub(self.autosave_at.elapsed());
                if remaining.is_zero() {
                    self.recovery();
                    self.autosave_at = Instant::now();
                    ctx.request_repaint_after(interval);
                } else {
                    ctx.request_repaint_after(remaining);
                }
            } else if !self.dirty() && self.autosaved.is_some() {
                self.clear_recovery();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canvas_drags_move_cards_and_connect_side_ports() {
        let ctx = Context::default();
        let mut app = Workbench::new(&ctx, None);
        app.load_project(&ctx, Project::new("Canvas drag test"), None);
        app.project.drawing.snap = false;
        let kind = NodeKind::Junction;
        let first = app
            .project
            .add_node("First", app.active_floor, kind, [-210.0, 0.0]);
        let second = app
            .project
            .add_node("Second", app.active_floor, kind, [210.0, 0.0]);
        let other_level = Floor::new("Other level", 4.0);
        let other = app
            .project
            .add_node("Read-only node", other_level.id, kind, [0.0, -160.0]);
        app.project.network.floors.push(other_level);
        let frame = |app: &mut Workbench, events| {
            let _ = ctx.run(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 800.0))),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    CentralPanel::default().show(ctx, |ui| app.canvas(ui, palette(false)));
                },
            );
        };
        frame(&mut app, vec![]);
        let drag = |app: &mut Workbench, start, end| {
            let button = |pos, pressed| UiEvent::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            frame(
                app,
                vec![
                    UiEvent::PointerMoved(start),
                    button(start, true),
                    UiEvent::PointerMoved(end),
                    button(end, false),
                ],
            );
        };
        drag(&mut app, pos2(390.0, 429.0), pos2(450.0, 509.0));
        assert_eq!(app.project.drawing.positions[&first], [-150.0, 80.0]);
        assert!(app.project.network.branches.is_empty());
        drag(&mut app, pos2(519.0, 509.0), pos2(600.0, 269.0));
        assert!(app.project.network.branches.is_empty());
        assert_eq!(app.project.drawing.positions[&other], [0.0, -160.0]);
        drag(&mut app, pos2(519.0, 509.0), pos2(810.0, 429.0));
        let branch = &app.project.network.branches[0];
        assert_eq!(app.project.network.branches.len(), 1);
        assert_eq!((branch.from, branch.to), (first, second));
        assert_eq!(
            app.project.drawing.ports[&branch.id],
            [Some(PortSide::Right), Some(PortSide::Left)]
        );

        let duct = branch.id;
        app.splice(
            Part::Supply,
            WireSplice {
                wire: duct,
                fraction: 0.5,
                points: [pos2(-81.0, 80.0), pos2(30.0, 40.0), pos2(141.0, 0.0)],
            },
        );
        let fan = app.project.network.branch(app.picked().unwrap()).unwrap();
        let (fan_id, inlet, outlet) = (fan.id, fan.from, fan.to);
        let before = [inlet, outlet].map(|id| app.project.drawing.positions[&id]);
        let start = pos2(
            600.0 + (before[0][0] + before[1][0]) / 2.0,
            429.0 + (before[0][1] + before[1][1]) / 2.0,
        );
        let offset = vec2(-25.0, -85.0);
        drag(&mut app, start, start + offset);
        for (node, [x, y]) in [inlet, outlet].into_iter().zip(before) {
            let [moved_x, moved_y] = app.project.drawing.positions[&node];
            assert!((vec2(moved_x - x, moved_y - y) - offset).length() < 0.001);
        }
        assert_eq!(app.picked(), Some(fan_id));
        let fan = app.project.network.branch(fan_id).unwrap();
        assert_eq!((fan.from, fan.to), (inlet, outlet));
        assert_eq!(app.project.network.branches.len(), 3);
        let [x, y] = app.project.drawing.positions[&outlet];
        drag(&mut app, pos2(600.0 + x, 429.0 + y), pos2(810.0, 429.0));
        let link = app.project.network.branch(app.picked().unwrap()).unwrap();
        assert_eq!((link.from, link.to), (outlet, second));
        assert_eq!(app.project.drawing.positions[&outlet], [x, y]);

        let [from_x, from_y] = app.project.drawing.positions[&inlet];
        let height = ((y - from_y).abs() + 14.0).max(56.0);
        let grip = pos2(
            600.0 + x.max(from_x) - 10.0,
            429.0 + (y + from_y - height) / 2.0 + 5.0,
        );
        drag(&mut app, grip, grip + vec2(12.0, -10.0));
        let [width, new_height] = app.project.drawing.sizes[&fan_id];
        assert!((width - (x - from_x).abs() - 24.0).abs() < 0.001);
        assert!((new_height - height - 20.0).abs() < 0.001);
        let [left, right] = [inlet, outlet].map(|id| app.project.drawing.positions[&id][0]);
        assert!(((right - left).abs() - width).abs() < 0.001);
    }

    #[test]
    fn moving_a_selection_or_container_keeps_items_together() {
        let ctx = Context::default();
        let mut app = Workbench::new(&ctx, None);
        app.load_project(&ctx, Project::new("Container test"), None);
        app.project.drawing.snap = false;
        let floor = app.active_floor;
        let a = app
            .project
            .add_node("A", floor, NodeKind::Junction, [-150.0, 0.0]);
        let b = app
            .project
            .add_node("B", floor, NodeKind::Junction, [150.0, 0.0]);
        let outside = app
            .project
            .add_node("Outside", floor, NodeKind::Junction, [400.0, 0.0]);
        app.connect(Part::Duct, a, b);
        let duct = app.picked().unwrap();
        app.project.drawing.bends.insert(duct, [0.0, -45.0]);
        let frame = |app: &mut Workbench, events| {
            let _ = ctx.run(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 800.0))),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    CentralPanel::default().show(ctx, |ui| app.canvas(ui, palette(false)));
                },
            );
        };
        let drag = |app: &mut Workbench, start, end| {
            let button = |pos, pressed| UiEvent::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            frame(
                app,
                vec![
                    UiEvent::PointerMoved(start),
                    button(start, true),
                    UiEvent::PointerMoved(end),
                    button(end, false),
                ],
            );
        };
        frame(&mut app, vec![]);
        app.selection = vec![a, b];
        drag(&mut app, pos2(450.0, 429.0), pos2(480.0, 449.0));
        assert_eq!(app.selection, [a, b]);
        assert_eq!(app.project.drawing.positions[&a], [-120.0, 20.0]);
        assert_eq!(app.project.drawing.positions[&b], [180.0, 20.0]);
        assert_eq!(app.project.drawing.bends[&duct], [30.0, -25.0]);

        app.draw_container = true;
        drag(&mut app, pos2(390.0, 344.0), pos2(875.0, 515.0));
        let id = app.picked().unwrap();
        let original = app.project.drawing.containers[&id].clone();
        drag(&mut app, pos2(420.0, 354.0), pos2(480.0, 389.0));
        assert_eq!(app.project.drawing.positions[&a], [-60.0, 55.0]);
        assert_eq!(app.project.drawing.positions[&b], [240.0, 55.0]);
        assert_eq!(app.project.drawing.positions[&outside], [400.0, 0.0]);
        assert_eq!(app.project.drawing.bends[&duct], [90.0, 10.0]);
        assert_eq!(
            app.project.drawing.containers[&id].center,
            [original.center[0] + 60.0, original.center[1] + 35.0]
        );
        app.history(false);
        assert_eq!(app.project.drawing.containers[&id], original);
        assert_eq!(app.project.drawing.positions[&b], [180.0, 20.0]);
    }

    #[test]
    fn background_level_names_use_floor_opacity() {
        let ctx = Context::default();
        let mut app = Workbench::new(&ctx, None);
        app.load_project(&ctx, Project::new("Background levels"), None);
        let floor = app.active_floor;
        let room = app.project.add_node(
            "Background room",
            floor,
            NodeKind::Room {
                volume_m3: 100.0,
                initial_pa: 0.0,
            },
            [-300.0, 0.0],
        );
        let outlet = app.project.add_node(
            "Background junction",
            floor,
            NodeKind::Junction,
            [300.0, 0.0],
        );
        app.connect(Part::Duct, room, outlet);
        app.splice(
            Part::Exhaust,
            WireSplice {
                wire: app.picked().unwrap(),
                fraction: 0.5,
                points: [pos2(-219.0, 0.0), pos2(0.0, 0.0), pos2(231.0, 0.0)],
            },
        );
        app.add_floor();
        app.place(Part::Atmosphere, [0.0, 200.0]);
        let foreground_name = app
            .project
            .network
            .node(app.picked().unwrap())
            .unwrap()
            .name
            .clone();
        let palette = palette(false);
        let expected = palette
            .text
            .gamma_multiply(app.project.drawing.background_opacity);
        let output = ctx.run(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 800.0))),
                ..Default::default()
            },
            |ctx| {
                CentralPanel::default().show(ctx, |ui| app.canvas(ui, palette));
            },
        );
        for (name, color) in [
            ("Exhaust fan", expected),
            ("Background room", expected),
            ("Background junction", expected),
            ("Duct segment", expected),
            (foreground_name.as_str(), palette.text),
        ] {
            let label = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    Shape::Text(label) if label.galley.job.text == name => Some(label),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("Missing label {name}"));
            assert_eq!(label.galley.job.sections[0].format.color, color, "{name}");
        }
    }

    #[test]
    fn inserting_a_fan_keeps_the_existing_wire_route() {
        let ctx = Context::default();
        let mut app = Workbench::new(&ctx, None);
        app.load_project(&ctx, Project::new("Inline route test"), None);
        let floor = app.active_floor;
        let a = app
            .project
            .add_node("A", floor, NodeKind::Junction, [0.0, 0.0]);
        let z = app
            .project
            .add_node("Z", floor, NodeKind::Junction, [300.0, 0.0]);
        app.connect_ports(
            Part::Duct,
            a,
            z,
            [Some(PortSide::Right), Some(PortSide::Left)],
        );
        let wire = app.picked().unwrap();
        let bend = [182.0, -85.0];
        app.project.drawing.bends.insert(wire, bend);
        app.splice(
            Part::Exhaust,
            WireSplice {
                wire,
                fraction: 0.2,
                points: [pos2(69.0, 0.0), pos2(bend[0], bend[1]), pos2(231.0, 0.0)],
            },
        );
        let fan = app.project.network.branch(app.picked().unwrap()).unwrap();
        for point in [fan.from, fan.to].map(|id| app.project.drawing.positions[&id]) {
            assert!(point[0] > 69.0 && point[0] < bend[0]);
            let expected_y = (point[0] - 69.0) * bend[1] / (bend[0] - 69.0);
            assert!((point[1] - expected_y).abs() < 1e-4);
        }
        let tail = app
            .project
            .network
            .branches
            .iter()
            .find(|branch| branch.from == fan.to)
            .unwrap();
        assert_eq!(app.project.drawing.bends.get(&tail.id), Some(&bend));
        assert_eq!(app.project.drawing.ports[&wire][0], Some(PortSide::Right));
        assert_eq!(app.project.drawing.ports[&tail.id][1], Some(PortSide::Left));
        assert_eq!(app.project.drawing.positions[&a], [0.0, 0.0]);
        assert_eq!(app.project.drawing.positions[&z], [300.0, 0.0]);
    }

    #[test]
    fn fast_library_drop_places_one_room_in_the_target_pane() {
        let ctx = Context::default();
        let mut app = Workbench::new(&ctx, None);
        app.load_project(&ctx, Project::new("Drag test"), None);
        let second = Floor::new("Level 02", 4.0);
        let second_id = second.id;
        app.project.network.floors.push(second);
        app.split = Some((second_id, Camera::default()));
        let mut frame = |events| {
            let _ = ctx.run(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1480.0, 920.0))),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    app.library(ctx, palette(false));
                    app.start_library_drag(ctx);
                    CentralPanel::default().show(ctx, |ui| app.split_canvas(ui, palette(false)));
                },
            );
        };
        frame(vec![]);
        let source = pos2(105.0, 133.0);
        let destination = pos2(1110.0, 400.0);
        let button = |pos, pressed| UiEvent::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        frame(vec![
            UiEvent::PointerMoved(source),
            button(source, true),
            UiEvent::PointerMoved(destination),
            button(destination, false),
        ]);
        assert_eq!(app.project.network.nodes.len(), 1);
        let room = &app.project.network.nodes[0];
        assert!(matches!(room.kind, NodeKind::Room { .. }));
        assert_eq!(room.floor, second_id);
        assert_eq!(app.picked(), Some(room.id));
    }

    #[test]
    fn door_drop_and_click_connect_rooms_without_ducts_or_junctions() {
        let ctx = Context::default();
        let mut app = Workbench::new(&ctx, None);
        app.load_project(&ctx, Project::new("Door test"), None);
        let floor = app.active_floor;
        let room = Part::Room.node().unwrap();
        let from = app.project.add_node("Zone 1", floor, room, [-210.0, 0.0]);
        let to = app.project.add_node("Zone 2", floor, room, [210.0, 0.0]);
        let frame = |app: &mut Workbench, events| {
            let _ = ctx.run(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 800.0))),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    app.start_library_drag(ctx);
                    CentralPanel::default().show(ctx, |ui| app.canvas(ui, palette(false)));
                },
            );
        };
        frame(&mut app, vec![]);
        let first = pos2(390.0, 429.0);
        let second = pos2(810.0, 429.0);
        let button = |pos, pressed| UiEvent::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        let source = pos2(40.0, 24.0);
        frame(
            &mut app,
            vec![UiEvent::PointerMoved(source), button(source, true)],
        );
        app.library_press = Some((LibraryDrag::Part(Part::Door), source));
        frame(
            &mut app,
            vec![UiEvent::PointerMoved(first), button(first, false)],
        );
        assert_eq!(app.from, Some(from));
        assert!(app.project.network.branches.is_empty());
        frame(
            &mut app,
            vec![UiEvent::PointerMoved(second), button(second, true)],
        );
        frame(&mut app, vec![button(second, false)]);
        assert_eq!(app.project.network.nodes.len(), 2);
        let [door] = app.project.network.branches.as_slice() else {
            panic!("expected one direct branch")
        };
        assert_eq!((door.from, door.to), (from, to));
        assert!(matches!(door.kind, BranchKind::Opening { .. }));
        assert_eq!(door.aperture, Part::Door.aperture());
    }

    #[test]
    fn changing_riser_floor_keeps_existing_local_ducts() {
        let ctx = Context::default();
        let mut app = Workbench::new(&ctx, None);
        app.load_project(&ctx, Project::new("Riser test"), None);
        let source = app.active_floor;
        let second = Floor::new("Level 02", 4.0);
        let third = Floor::new("Level 03", 8.0);
        let (second_id, third_id) = (second.id, third.id);
        app.project.network.floors.extend([second, third]);
        app.place_riser([100.0, 80.0]);
        let riser_id = app.project.network.branches[0].id;
        app.set_riser_destination(riser_id, second_id);
        app.active_floor = source;
        let initial = app.project.network.branches[0].clone();
        assert_eq!(
            app.project.network.node(initial.from).unwrap().floor,
            source
        );
        assert_eq!(
            app.project.network.node(initial.to).unwrap().floor,
            second_id
        );
        let boundary = app.project.add_node(
            "Boundary",
            second_id,
            NodeKind::Boundary { gauge_pa: 0.0 },
            [280.0, 80.0],
        );
        let duct = app.project.add_branch(
            "Local duct",
            initial.to,
            boundary,
            System::Supply,
            Part::Duct.branch(),
        );
        app.set_riser_destination(initial.id, third_id);
        let riser = app.project.network.branch(initial.id).unwrap();
        assert_eq!(app.active_floor, third_id);
        assert_eq!(app.picked(), Some(riser.to));
        assert_eq!(
            app.project.drawing.riser_landings.get(&riser.to),
            Some(&initial.id)
        );
        assert_eq!(app.project.drawing.positions[&riser.to], [100.0, 80.0]);
        assert_eq!(app.project.network.branch(duct).unwrap().from, initial.to);
        assert_eq!(
            app.project.network.node(initial.to).unwrap().floor,
            second_id
        );
        assert!(!app.project.drawing.riser_landings.contains_key(&initial.to));
        let BranchKind::Duct { duct, .. } = riser.kind else {
            panic!("expected a riser duct");
        };
        assert_eq!(duct.length_m, 8.0);
    }
}
