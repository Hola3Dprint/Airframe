use airflow::{model::*, storage::save};
use std::path::Path;

fn room() -> NodeKind {
    NodeKind::Room {
        volume_m3: 100.0,
        initial_pa: 0.0,
    }
}
fn outside() -> NodeKind {
    NodeKind::Boundary { gauge_pa: 0.0 }
}
fn fan(q: f64) -> BranchKind {
    BranchKind::FlowFan {
        design_m3_s: q,
        tau_s: 5.0,
    }
}
fn events(p: &mut Project, fan: uuid::Uuid, start: f64, stop: f64) {
    p.scenarios[0].events.extend([
        Event {
            fan,
            time_s: start,
            command: 1.0,
        },
        Event {
            fan,
            time_s: stop,
            command: 0.0,
        },
    ]);
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut single = Project::new("Single-room airflow");
    single.notes = "Benchmark: 600 m³/h exhaust through a passive inlet, giving a −50 Pa operating point. Try changing the fan airflow and solving again.".into();
    let f = single.network.floors[0].id;
    let a = single.add_node("Outdoor intake", f, outside(), [-280.0, 10.0]);
    let b = single.add_node("Test room", f, room(), [0.0, 10.0]);
    let c = single.add_node("Outdoor discharge", f, outside(), [280.0, 10.0]);
    let q = 600.0 / M3_H;
    single.add_branch(
        "Passive inlet",
        a,
        b,
        System::Outdoor,
        BranchKind::Opening {
            k: 50.0 / q.powi(2),
        },
    );
    let exhaust = single.add_branch("Exhaust fan", b, c, System::Exhaust, fan(q));
    events(&mut single, exhaust, 0.0, 60.0);
    save(Path::new("examples/single-room.airflow.json"), &single)?;

    let mut two = Project::new("Two-floor ventilation");
    two.notes = "Synthetic example for testing stacked routes and a connected riser. Elevations describe the layout; stack pressure is not modeled.".into();
    let f0 = two.network.floors[0].id;
    two.network.floors.push(Floor::new("Level 02", 4.0));
    let f1 = two.network.floors[1].id;
    let a = two.add_node("Outdoor intake", f0, outside(), [-300., -120.]);
    let b = two.add_node("Main corridor", f0, room(), [-20., 0.]);
    let c = two.add_node("Studio 01", f0, room(), [255., 0.]);
    let d = two.add_node("Studio 02", f0, room(), [240., 165.]);
    let e = two.add_node("Studio 03", f1, room(), [-20., 0.]);
    let f = two.add_node("Studio 04", f1, room(), [255., 0.]);
    let g = two.add_node("Outdoor discharge", f1, outside(), [-115., -145.]);
    let h = two.add_node("Exhaust junction", f1, NodeKind::Junction, [280., -145.]);
    let supply = two.add_branch("Supply fan", a, b, System::Supply, fan(0.20));
    let duct = BranchKind::Duct {
        duct: Duct::default(),
        riser: false,
    };
    two.add_branch("Lower transfer duct", b, c, System::Transfer, duct);
    two.add_branch("Upper transfer duct", e, f, System::Transfer, duct);
    two.add_branch(
        "Door transfer",
        c,
        d,
        System::Transfer,
        BranchKind::Opening { k: 500.0 },
    );
    two.add_branch(
        "Relief",
        d,
        a,
        System::Outdoor,
        BranchKind::Opening { k: 800.0 },
    );
    let riser = BranchKind::Duct {
        duct: Duct {
            length_m: 4.0,
            ..Duct::default()
        },
        riser: true,
    };
    two.add_branch("Vertical riser", b, e, System::Supply, riser);
    two.add_branch("Exhaust duct", f, h, System::Exhaust, duct);
    let exhaust = two.add_branch("Exhaust fan", h, g, System::Exhaust, fan(0.16));
    events(&mut two, supply, 0.0, 75.0);
    events(&mut two, exhaust, 10.0, 60.0);
    save(Path::new("examples/two-floor.airflow.json"), &two)?;

    Ok(())
}
