//! Versioned JSON projects and CSV results.
use crate::{
    model::{Project, SCHEMA},
    solver::Run,
};
use std::{
    fs,
    io::{BufReader, BufWriter, Write},
    path::Path,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum FileError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Csv(#[from] csv::Error),
    #[error("Unsupported project schema (this version reads schema {SCHEMA}).")]
    Version,
    #[error("A project needs at least one floor and one scenario.")]
    Empty,
}
pub fn decode(reader: impl std::io::Read) -> Result<Project, FileError> {
    let value: serde_json::Value = serde_json::from_reader(reader)?;
    if value.get("schema").and_then(serde_json::Value::as_u64) != Some(u64::from(SCHEMA)) {
        return Err(FileError::Version);
    }
    let project: Project = serde_json::from_value(value)?;
    if project.network.floors.is_empty() || project.scenarios.is_empty() {
        return Err(FileError::Empty);
    }
    Ok(project)
}
pub fn load(path: &Path) -> Result<Project, FileError> {
    decode(BufReader::new(fs::File::open(path)?))
}
pub fn save(path: &Path, project: &Project) -> Result<(), FileError> {
    save_json(path, project)
}

pub fn save_json(path: &Path, value: &impl serde::Serialize) -> Result<(), FileError> {
    let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut writer = BufWriter::new(fs::File::create(&temp)?);
        serde_json::to_writer_pretty(&mut writer, value)?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        writer.get_ref().sync_all()?;
        drop(writer);
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}
pub fn export_csv(path: &Path, run: &Run) -> Result<(), FileError> {
    let mut writer = csv::Writer::from_path(path)?;
    let mut header = vec!["time_s".to_owned()];
    header.extend(
        run.snapshot
            .nodes
            .iter()
            .map(|n| format!("{} [{}] gauge_Pa", n.name, n.id)),
    );
    header.extend(
        run.snapshot
            .branches
            .iter()
            .map(|b| format!("{} [{}] m3_s", b.name, b.id)),
    );
    header.extend(
        run.snapshot
            .branches
            .iter()
            .filter(|b| b.kind.tau().is_some())
            .map(|b| format!("{} [{}] fan_fraction", b.name, b.id)),
    );
    writer.write_record(header)?;
    for sample in &run.samples {
        let mut row = vec![sample.time_s.to_string()];
        row.extend(sample.pressures.iter().map(f64::to_string));
        row.extend(sample.flows.iter().map(f64::to_string));
        row.extend(sample.fan_fraction.iter().flatten().map(f64::to_string));
        writer.write_record(row)?;
    }
    writer.flush()?;
    Ok(())
}

/// Export node balances and branch flows in SI, with stable component IDs.
pub fn export_operating_point(
    path: &Path,
    point: &crate::solver::steady::OperatingPoint,
) -> Result<(), FileError> {
    let mut writer = csv::Writer::from_path(path)?;
    writer.write_record([
        "kind",
        "id",
        "name",
        "from_id",
        "to_id",
        "gauge_pa",
        "inflow_m3_s",
        "outflow_m3_s",
        "net_m3_s",
        "flow_m3_s",
        "from_minus_to_pa",
    ])?;
    for (i, node) in point.snapshot.nodes.iter().enumerate() {
        let balance = point.balances[i];
        writer.write_record([
            node.kind.label().to_owned(),
            node.id.to_string(),
            node.name.clone(),
            String::new(),
            String::new(),
            point.pressures[i].to_string(),
            balance.inflow.to_string(),
            balance.outflow.to_string(),
            balance.net().to_string(),
            String::new(),
            String::new(),
        ])?;
    }
    let index: std::collections::HashMap<_, _> = point
        .snapshot
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id, i))
        .collect();
    for (i, branch) in point.snapshot.branches.iter().enumerate() {
        let pressure_drop =
            point.pressures[index[&branch.from]] - point.pressures[index[&branch.to]];
        writer.write_record([
            branch.kind.label().to_owned(),
            branch.id.to_string(),
            branch.name.clone(),
            branch.from.to_string(),
            branch.to.to_string(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            point.flows[i].to_string(),
            pressure_drop.to_string(),
        ])?;
    }
    writer.flush()?;
    Ok(())
}
