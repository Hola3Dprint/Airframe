use airflow::{
    model::{CFM, NodeKind},
    solver::steady::solve,
    storage,
};
use std::{io, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1).map(PathBuf::from);
    let input = args.next().ok_or_else(|| {
        io::Error::other("Usage: solve_airflow <project.json> [flow.csv|flow.xlsx]")
    })?;
    let project = storage::load(&input)?;
    let point = solve(&project.network, || true)?;
    println!("{} | Steady airflow", project.name);
    for (i, node) in point.snapshot.nodes.iter().enumerate() {
        if matches!(node.kind, NodeKind::Room { .. }) {
            let b = point.balances[i];
            println!(
                "{}: {:+.3} Pa, in {:.3} CFM, out {:.3} CFM",
                node.name,
                point.pressures[i],
                b.inflow / CFM,
                b.outflow / CFM
            );
        }
    }
    println!(
        "Maximum internal imbalance {:.2e} m³/s",
        point.max_imbalance
    );
    for warning in &point.warnings {
        eprintln!("{warning}");
    }
    if let Some(path) = args.next() {
        if path
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("xlsx"))
        {
            airflow::excel::export(&path, &project.name, &point)?;
        } else {
            storage::export_operating_point(&path, &point)?;
        }
    }
    Ok(())
}
