use airflow::{model::NodeKind, solver::simulate, storage};
use std::{io, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1).map(PathBuf::from);
    let input = args
        .next()
        .ok_or_else(|| io::Error::other("Usage: simulate_project <project.json> [results.csv]"))?;
    let p = storage::load(&input)?;
    let run = simulate(&p.network, &p.scenarios[0], |_| true)?;
    println!("{} | {}", p.name, run.scenario.name);
    for (i, node) in p.network.nodes.iter().enumerate() {
        if matches!(node.kind, NodeKind::Room { .. }) {
            let values = run.samples.iter().map(|s| s.pressures[i]);
            let lo = values.clone().fold(f64::INFINITY, f64::min);
            let hi = values.fold(f64::NEG_INFINITY, f64::max);
            let end = run.samples.last().expect("completed run").pressures[i];
            println!(
                "{}: min {lo:+.6} Pa, max {hi:+.6} Pa, final {end:+.6} Pa",
                node.name
            );
        }
    }
    println!(
        "{} samples, {} adaptive steps; max junction imbalance {:.2e} m³/s",
        run.samples.len(),
        run.steps,
        run.max_junction_residual
    );
    for warning in &run.warnings {
        eprintln!("{warning}");
    }
    if let Some(path) = args.next() {
        storage::export_csv(&path, &run)?;
    }
    Ok(())
}
