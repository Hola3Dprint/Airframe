use airflow::{model::Project, storage};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io,
    path::{Path, PathBuf},
};
use uuid::Uuid;

const SNAPSHOT: &str = "snapshot.json";
const LOCK: &str = "active.lock";

fn root() -> PathBuf {
    crate::local_data_folder().join("recovery")
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Snapshot {
    pub original: Option<PathBuf>,
    pub project: Project,
}

pub(crate) struct RecoveryEntry {
    pub file: PathBuf,
    pub snapshot: Snapshot,
}

pub(crate) struct RecoverySession {
    dir: PathBuf,
    lock: File,
}

impl RecoverySession {
    pub fn start() -> io::Result<Self> {
        Self::start_in(&root())
    }

    fn start_in(root: &Path) -> io::Result<Self> {
        fs::create_dir_all(root)?;
        let dir = root.join(Uuid::new_v4().to_string());
        fs::create_dir(&dir)?;
        let lock = File::options()
            .read(true)
            .write(true)
            .create_new(true)
            .open(dir.join(LOCK))?;
        lock.try_lock()?;
        Ok(Self { dir, lock })
    }

    pub fn write(&self, snapshot: &Snapshot) -> Result<(), storage::FileError> {
        storage::save_json(&self.dir.join(SNAPSHOT), snapshot)
    }

    pub fn clear(&self) -> io::Result<()> {
        remove_snapshot(&self.snapshot_path())
    }

    pub fn snapshot_path(&self) -> PathBuf {
        self.dir.join(SNAPSHOT)
    }
}

impl Drop for RecoverySession {
    fn drop(&mut self) {
        if !self.dir.join(SNAPSHOT).exists() {
            let _ = self.lock.unlock();
            let _ = fs::remove_file(self.dir.join(LOCK));
            let _ = fs::remove_dir(&self.dir);
        }
    }
}

pub(crate) fn remove_snapshot(file: &Path) -> io::Result<()> {
    match fs::remove_file(file) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

pub(crate) fn available() -> Vec<RecoveryEntry> {
    available_in(&root())
}

fn available_in(root: &Path) -> Vec<RecoveryEntry> {
    let Ok(dirs) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut entries = Vec::new();
    for dir in dirs.flatten() {
        let file = dir.path().join(SNAPSHOT);
        let Ok(modified) = file.metadata().and_then(|m| m.modified()) else {
            continue;
        };
        let Ok(lock) = File::options()
            .read(true)
            .write(true)
            .open(dir.path().join(LOCK))
        else {
            continue;
        };
        if lock.try_lock().is_err() {
            continue;
        }
        let Ok(snapshot) = File::open(&file).and_then(|input| {
            serde_json::from_reader::<_, Snapshot>(io::BufReader::new(input))
                .map_err(io::Error::from)
        }) else {
            continue;
        };
        if snapshot.project.schema == airflow::model::SCHEMA
            && !snapshot.project.network.floors.is_empty()
            && !snapshot.project.scenarios.is_empty()
        {
            entries.push((modified, RecoveryEntry { file, snapshot }));
        }
    }
    entries.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    entries.into_iter().map(|(_, entry)| entry).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_finds_the_latest_unsaved_edits_after_a_session_ends() {
        let root = std::env::temp_dir().join(format!("airframe-test-{}", Uuid::new_v4()));
        let session = RecoverySession::start_in(&root).unwrap();
        let original = root.join("saved.airflow.json");
        let mut snapshot = Snapshot {
            original: Some(original.clone()),
            project: Project::new("Unsaved edits"),
        };
        storage::save(&original, &snapshot.project).unwrap();
        session.write(&snapshot).unwrap();
        snapshot.project.notes = "Latest edit".into();
        session.write(&snapshot).unwrap();
        assert!(
            available_in(&root).is_empty(),
            "An open session owns the snapshot"
        );

        let dir = session.dir.clone();
        drop(session);
        let entries = available_in(&root);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].snapshot.original.as_ref(), Some(&original));
        assert_eq!(entries[0].snapshot.project.notes, "Latest edit");
        assert!(storage::load(&original).unwrap().notes.is_empty());

        remove_snapshot(&entries[0].file).unwrap();
        assert!(available_in(&root).is_empty());
        fs::remove_file(dir.join(LOCK)).unwrap();
        fs::remove_dir(dir).unwrap();
        fs::remove_file(original).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
