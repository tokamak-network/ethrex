use crate::models::Incident;
use rusqlite::{Connection, params};
use std::path::Path;
use std::time::UNIX_EPOCH;
use thiserror::Error;

#[derive(Debug)]
pub struct IncidentRepository {
    connection: Connection,
}

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("time conversion error: {0}")]
    Time(#[from] std::time::SystemTimeError),
    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),
}

impl IncidentRepository {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let connection = Connection::open(path)?;
        let repository = Self { connection };
        repository.ensure_schema()?;
        Ok(repository)
    }

    fn ensure_schema(&self) -> Result<(), StorageError> {
        self.connection.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS incidents (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                scenario TEXT NOT NULL,
                severity TEXT NOT NULL,
                message TEXT NOT NULL,
                detected_at_unix INTEGER NOT NULL,
                evidence_json TEXT NOT NULL,
                false_positive INTEGER
            );
            ",
        )?;

        Ok(())
    }

    pub fn insert(&self, incident: &Incident) -> Result<i64, StorageError> {
        let detected_at = incident.detected_at.duration_since(UNIX_EPOCH)?.as_secs();
        let evidence_json = serde_json::to_string(&incident.evidence)?;
        self.connection.execute(
            "
            INSERT INTO incidents (scenario, severity, message, detected_at_unix, evidence_json, false_positive)
            VALUES (?1, ?2, ?3, ?4, ?5, NULL)
            ",
            params![
                format!("{:?}", incident.scenario),
                format!("{:?}", incident.severity),
                incident.message,
                detected_at,
                evidence_json,
            ],
        )?;

        Ok(self.connection.last_insert_rowid())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Scenario, Severity};
    use serde_json::json;
    use tempfile::NamedTempFile;

    #[test]
    fn inserts_incident_into_sqlite() {
        let file_result = NamedTempFile::new();
        assert!(file_result.is_ok());
        let file = match file_result {
            Ok(file) => file,
            Err(_) => return,
        };

        let repository_result = IncidentRepository::open(file.path());
        assert!(repository_result.is_ok());
        let repository = match repository_result {
            Ok(repository) => repository,
            Err(_) => return,
        };

        let incident = Incident {
            scenario: Scenario::ExecutionRpcTimeout,
            severity: Severity::Warning,
            message: "rpc timeout burst".to_owned(),
            detected_at: UNIX_EPOCH,
            evidence: json!({"timeout_rate": 33.5}),
        };

        let row_id_result = repository.insert(&incident);
        assert!(row_id_result.is_ok());
        let row_id = match row_id_result {
            Ok(id) => id,
            Err(_) => return,
        };

        assert!(row_id > 0);
    }
}
