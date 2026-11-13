use crate::workbench::Part;
use airflow::model::{Branch, BranchKind, NodeKind, System};
use eframe::egui::Ui;

pub(crate) fn tooltip(ui: &mut Ui, name: &str, summary: &str, usage: &str) {
    ui.set_max_width(290.0);
    ui.strong(name);
    ui.label(summary);
    if !usage.is_empty() {
        ui.small(usage);
    }
}

pub(crate) fn summary(part: Part) -> &'static str {
    match part {
        Part::Room => "Represents a room or zone with its own air pressure and volume.",
        Part::Boundary => "Holds a known gauge pressure, such as outdoor air at 0 Pa.",
        Part::Atmosphere => {
            "Represents outdoor air at 0 Pa gauge. It is the ground reference in Circuit view."
        }
        Part::Junction => "Joins airflow paths at a common pressure without storing air.",
        Part::Inlet => {
            "Lets air flow in either direction according to the pressure difference and opening resistance."
        }
        Part::Door => "Models airflow through an opening between rooms.",
        Part::Leakage => {
            "Models unintended airflow through small gaps, such as cracks or door undercuts."
        }
        Part::TransferAir => "Models a passive grille that lets air move between spaces.",
        Part::SupplyAir => "Delivers a specified airflow from a supply path into a room.",
        Part::Duct => {
            "Carries air between nodes. Its size, length, roughness, and fittings determine pressure loss."
        }
        Part::Riser => "Carries air through a vertical duct on one level or between levels.",
        Part::Supply => "Pushes a set airflow into a duct or room.",
        Part::Exhaust => "Draws a set airflow from a room or duct to a discharge point.",
        Part::Transfer => "Moves a set airflow from one space to another.",
        Part::Pressure => {
            "Uses a fan curve and speed to determine pressure rise and delivered airflow."
        }
    }
}

pub(crate) fn library_usage(part: Part) -> &'static str {
    match part {
        Part::Room => "Drag onto the canvas, then set the room volume in the Inspector.",
        Part::Boundary => "Drag onto the canvas, then set its reference pressure in the Inspector.",
        Part::Atmosphere => {
            "Connect a passive opening for a vent, or an exhaust fan for powered discharge."
        }
        Part::Junction => "Drag onto the canvas where multiple ducts or openings need to meet.",
        Part::Riser => {
            "Drop on the canvas, a node, or a wire. Use the Inspector to set the destination level."
        }
        _ => {
            "Select it, then drag from a source port to a destination node. Drop it on a wire to insert it in series."
        }
    }
}

pub(crate) fn node_summary(kind: NodeKind) -> &'static str {
    summary(match kind {
        NodeKind::Room { .. } => Part::Room,
        NodeKind::Boundary { .. } => Part::Boundary,
        NodeKind::Atmosphere => Part::Atmosphere,
        NodeKind::Junction => Part::Junction,
    })
}

pub(crate) fn branch_summary(branch: &Branch) -> (&'static str, &'static str) {
    match branch.kind {
        BranchKind::Opening { .. } if branch.aperture.is_some() => (
            "Models airflow through a physical opening.",
            "Set the free area, discharge coefficient, and open fraction in the Inspector.",
        ),
        BranchKind::Opening { .. } => (
            summary(Part::Inlet),
            "Select the branch to edit its loss coefficient.",
        ),
        BranchKind::Duct { riser: true, .. } => (
            summary(Part::Riser),
            "Select the riser to edit its duct properties or change the destination level.",
        ),
        BranchKind::Duct { .. } => (
            summary(Part::Duct),
            "Select the duct to edit its dimensions, roughness, and fitting losses.",
        ),
        BranchKind::FlowFan { .. } => (
            match branch.system {
                System::Supply => summary(Part::Supply),
                System::Exhaust => summary(Part::Exhaust),
                System::Transfer => summary(Part::Transfer),
                _ => "Delivers a specified airflow in the direction of the branch.",
            },
            "Select it to set the airflow or switch to a fan curve.",
        ),
        BranchKind::PressureFan { .. } => (
            summary(Part::Pressure),
            "Select the fan to edit its curve. Use the airflow slider to adjust speed.",
        ),
    }
}
