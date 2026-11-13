use crate::{style::*, workbench::Workbench};
use airflow::{
    model::{BranchKind, Event, Scenario},
    solver::{self, Run, Sample, SolveError, steady::FlowBalance},
};
use eframe::egui::*;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use uuid::Uuid;

#[derive(Default)]
pub(crate) struct Startup {
    run: Option<Run>,
    receiver: Option<Receiver<Result<Run, SolveError>>>,
    time: f64,
    playing: bool,
    pub visible: bool,
}

impl Workbench {
    pub(crate) fn display_balance(&self, id: Uuid) -> Option<FlowBalance> {
        if let Some((run, sample)) = self.startup_sample() {
            run.snapshot.node(id)?;
            let mut balance = FlowBalance::default();
            for (b, q) in run.snapshot.branches.iter().zip(&sample.flows) {
                let inward = if b.to == id {
                    *q
                } else if b.from == id {
                    -*q
                } else {
                    continue;
                };
                balance.inflow += inward.max(0.0);
                balance.outflow += (-inward).max(0.0);
            }
            Some(balance)
        } else {
            self.current_run()?.balance(id)
        }
    }
    pub(crate) fn startup_sample(&self) -> Option<(&Run, &Sample)> {
        let run = self.startup.run.as_ref()?;
        if !self.startup.visible
            || run.snapshot.air != self.project.network.air
            || run.snapshot.nodes != self.project.network.nodes
            || run.snapshot.branches != self.project.network.branches
        {
            return None;
        }
        Some((run, run.at(self.startup.time)))
    }

    pub(crate) fn poll_startup(&mut self) {
        let Some(receiver) = self.startup.receiver.as_ref() else {
            return;
        };
        match receiver.try_recv() {
            Ok(result) => {
                self.startup.receiver = None;
                match result {
                    Ok(run) => {
                        self.startup.time = 0.0;
                        self.startup.run = Some(run);
                        self.notice("Startup run ready · drag the timeline to review it", false);
                    }
                    Err(error) => self.notice(error.to_string(), true),
                }
            }
            Err(TryRecvError::Disconnected) => {
                self.startup.receiver = None;
                self.notice("Startup worker stopped.", true);
            }
            Err(TryRecvError::Empty) => {}
        }
    }

    fn run_startup(&mut self, ctx: &Context) {
        let net = self.project.network.clone();
        let events: Vec<_> = net
            .branches
            .iter()
            .filter_map(|b| {
                let command = match b.kind {
                    BranchKind::FlowFan { .. } => 1.0,
                    BranchKind::PressureFan { .. } => b.fan_speed_fraction.unwrap_or(1.0),
                    _ => return None,
                };
                Some(Event {
                    fan: b.id,
                    time_s: 0.0,
                    command,
                })
            })
            .collect();
        if events.is_empty() {
            self.notice("Add a fan before running startup.", true);
            return;
        }
        let duration_s = self
            .project
            .scenarios
            .first()
            .map_or(30.0, |s| s.duration_s.clamp(1.0, 120.0));
        let scenario = Scenario {
            name: "Fan startup".into(),
            duration_s,
            events,
        };
        let (send, recv) = mpsc::channel();
        self.startup.receiver = Some(recv);
        self.startup.playing = false;
        self.startup.run = None;
        self.notice("Simulating fan startup…", false);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = solver::simulate(&net, &scenario, |_| true);
            let _ = send.send(result);
            ctx.request_repaint();
        });
    }

    pub(crate) fn startup_controls(&mut self, ui: &mut Ui, p: Palette) {
        ui.horizontal(|ui| {
            if ui
                .selectable_label(!self.startup.visible, "Steady flow")
                .clicked()
            {
                self.startup.visible = false;
                self.startup.playing = false;
            }
            if ui
                .selectable_label(self.startup.visible, "Fan startup")
                .clicked()
            {
                self.startup.visible = true;
            }
        });
        if !self.startup.visible {
            return;
        }
        let pending = self.startup.receiver.is_some();
        if self.startup_sample().is_none() {
            if ui
                .add_enabled(
                    !pending,
                    Button::new(if pending {
                        "Simulating…"
                    } else {
                        "Run startup"
                    }),
                )
                .clicked()
            {
                self.run_startup(ui.ctx());
            }
            ui.small("Starts fans from rest and shows the pressure and airflow response.");
            return;
        }
        let duration = self.startup.run.as_ref().unwrap().scenario.duration_s;
        ui.horizontal(|ui| {
            if ui
                .button(if self.startup.playing {
                    "Pause"
                } else {
                    "Play"
                })
                .clicked()
            {
                if self.startup.time >= duration {
                    self.startup.time = 0.0;
                }
                self.startup.playing = !self.startup.playing;
            }
            if ui.button("↺").on_hover_text("Back to the start").clicked() {
                self.startup.time = 0.0;
            }
            ui.scope(|ui| {
                ui.spacing_mut().slider_width = (ui.available_width() - 104.0).clamp(110.0, 900.0);
                if ui
                    .add(
                        Slider::new(&mut self.startup.time, 0.0..=duration)
                            .text("s")
                            .max_decimals(2),
                    )
                    .dragged()
                {
                    self.startup.playing = false;
                }
            });
        });
        if self.startup.playing {
            let dt = ui.input(|i| f64::from(i.stable_dt)).min(0.1);
            self.startup.time = (self.startup.time + dt).min(duration);
            self.startup.playing = self.startup.time < duration;
            ui.ctx().request_repaint();
        }
        if let Some((run, sample)) = self.startup_sample() {
            let (unit, factor) = self.flow_unit();
            let fan = self
                .picked()
                .and_then(|id| {
                    run.snapshot
                        .branches
                        .iter()
                        .position(|b| b.id == id && b.kind.tau().is_some())
                })
                .or_else(|| {
                    run.snapshot
                        .branches
                        .iter()
                        .position(|b| b.kind.tau().is_some())
                });
            if let Some(i) = fan {
                ui.label(tag(
                    &format!(
                        "{} · {:+.1} {unit}  /  {:.2} s",
                        run.snapshot.branches[i].name,
                        sample.flows[i] * factor,
                        sample.time_s
                    ),
                    p.accent,
                ));
            }
        }
    }

    pub(crate) fn startup_report(&mut self, ui: &mut Ui, p: Palette) {
        let Some((run, sample)) = self.startup_sample() else {
            return;
        };
        let (unit, factor) = self.flow_unit();
        let mut select = None;
        ScrollArea::both().show(ui, |ui| {
            eyebrow(ui, "PRESSURES AT SAMPLE", p);
            Grid::new("startup-pressures").striped(true).show(ui, |ui| {
                for (n, value) in run.snapshot.nodes.iter().zip(&sample.pressures) {
                    if ui
                        .selectable_label(self.picked() == Some(n.id), &n.name)
                        .clicked()
                    {
                        select = Some(n.id);
                    }
                    ui.monospace(format!("{value:+.3} Pa"));
                    ui.end_row();
                }
            });
            ui.separator();
            eyebrow(ui, "FLOWS AT SAMPLE", p);
            Grid::new("startup-flows").striped(true).show(ui, |ui| {
                for (b, value) in run.snapshot.branches.iter().zip(&sample.flows) {
                    if ui
                        .selectable_label(self.picked() == Some(b.id), &b.name)
                        .clicked()
                    {
                        select = Some(b.id);
                    }
                    ui.monospace(format!("{:+.3} {unit}", value * factor));
                    ui.end_row();
                }
            });
        });
        if let Some(id) = select {
            self.selection = vec![id];
        }
    }
}
