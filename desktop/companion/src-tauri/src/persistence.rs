use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::model::{CompanionSnapshot, MAX_SNAPSHOT_BYTES, ValidationError};

const SNAPSHOT_FILE_NAME: &str = "companion-snapshot.json";

#[derive(Debug, Clone)]
pub struct SnapshotStore {
    path: PathBuf,
}

#[derive(Debug, Error)]
pub enum PersistenceError {
    #[error("snapshot storage failed: {0}")]
    Io(#[from] io::Error),
    #[error("snapshot JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("snapshot validation failed: {0}")]
    Validation(#[from] ValidationError),
    #[error("snapshot JSON exceeds the {MAX_SNAPSHOT_BYTES}-byte limit")]
    TooLarge,
}

impl SnapshotStore {
    pub fn in_directory(directory: impl Into<PathBuf>) -> Self {
        Self {
            path: directory.into().join(SNAPSHOT_FILE_NAME),
        }
    }

    #[cfg(test)]
    fn at_path(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn load(&self) -> Result<Option<CompanionSnapshot>, PersistenceError> {
        let file = match File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };

        let metadata = file.metadata()?;
        if metadata.len() > MAX_SNAPSHOT_BYTES as u64 {
            return Err(PersistenceError::TooLarge);
        }

        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        file.take((MAX_SNAPSHOT_BYTES + 1) as u64)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_SNAPSHOT_BYTES {
            return Err(PersistenceError::TooLarge);
        }

        let snapshot: CompanionSnapshot = serde_json::from_slice(&bytes)?;
        snapshot.validate()?;
        Ok(Some(snapshot))
    }

    pub fn save(&self, snapshot: &CompanionSnapshot) -> Result<(), PersistenceError> {
        snapshot.validate()?;
        let bytes = serde_json::to_vec_pretty(snapshot)?;
        if bytes.len() > MAX_SNAPSHOT_BYTES {
            return Err(PersistenceError::TooLarge);
        }

        let parent = self.path.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "snapshot path has no parent")
        })?;
        fs::create_dir_all(parent)?;

        let file_name = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(SNAPSHOT_FILE_NAME);
        let mut temporary = tempfile::Builder::new()
            .prefix(&format!(".{file_name}."))
            .suffix(".tmp")
            .tempfile_in(parent)?;
        temporary.write_all(&bytes)?;
        temporary.as_file().sync_all()?;
        temporary.persist(&self.path).map_err(|error| error.error)?;

        sync_directory(parent)?;
        Ok(())
    }
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{MAX_DIETARY_TAGS, SCHEMA_VERSION};

    #[test]
    fn atomically_round_trips_a_valid_snapshot() {
        let directory = tempfile::tempdir().unwrap();
        let store = SnapshotStore::in_directory(directory.path());
        let mut snapshot = CompanionSnapshot::default();
        snapshot.settings.user_name = "Ari".to_owned();

        store.save(&snapshot).unwrap();
        snapshot.settings.user_name = "Mina".to_owned();
        store.save(&snapshot).unwrap();

        assert_eq!(store.load().unwrap(), Some(snapshot));
        assert!(directory.path().read_dir().unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")
        }));
    }

    #[test]
    fn rejects_invalid_persisted_snapshot() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("invalid.json");
        let store = SnapshotStore::at_path(&path);
        let invalid = format!(
            r#"{{"schemaVersion":{},"settings":{{"companionName":"Nyx","userName":"","timezone":"UTC","budget":"everyday","dietary":{},"avoid":[],"quietMode":false,"onboardingComplete":false,"meals":[]}},"history":[],"runtime":{{}}}}"#,
            SCHEMA_VERSION,
            serde_json::to_string(
                &(0..=MAX_DIETARY_TAGS)
                    .map(|index| format!("tag-{index}"))
                    .collect::<Vec<_>>()
            )
            .unwrap()
        );
        fs::write(path, invalid).unwrap();

        assert!(matches!(store.load(), Err(PersistenceError::Validation(_))));
    }

    #[test]
    fn rejects_unknown_json_fields() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("unknown.json");
        let store = SnapshotStore::at_path(&path);
        let mut value = serde_json::to_value(CompanionSnapshot::default()).unwrap();
        value["unexpected"] = serde_json::json!(true);
        fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();

        assert!(matches!(store.load(), Err(PersistenceError::Json(_))));
    }
}
