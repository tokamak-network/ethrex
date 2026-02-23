use crate::models::Incident;
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;
use std::time::UNIX_EPOCH;
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct IncidentRow {
    pub id: i64,
    pub scenario: String,
    pub severity: String,
    pub message: String,
    pub false_positive: Option<bool>,
}

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

    pub fn mark_false_positive(&self, incident_id: i64, is_false_positive: bool) -> Result<(), StorageError> {
        let value: i64 = if is_false_positive { 1 } else { 0 };
        self.connection.execute(
            "UPDATE incidents SET false_positive = ?1 WHERE id = ?2",
            params![value, incident_id],
        )?;
        Ok(())
    }

    pub fn false_positive_rate(&self) -> Result<Option<f64>, StorageError> {
        let mut statement = self.connection.prepare(
            "
            SELECT
                SUM(CASE WHEN false_positive = 1 THEN 1 ELSE 0 END) as fp,
                SUM(CASE WHEN false_positive IS NOT NULL THEN 1 ELSE 0 END) as labeled
            FROM incidents
            ",
        )?;

        let pair: Option<(i64, i64)> = statement
            .query_row([], |row| Ok((row.get(0)?, row.get(1)?)))
            .optional()?;

        let (false_positives, labeled) = match pair {
            Some(values) => values,
            None => return Ok(None),
        };

        if labeled == 0 {
            return Ok(None);
        }

        Ok(Some(false_positives as f64 / labeled as f64))
    }

    pub fn list_recent(&self, limit: usize) -> Result<Vec<IncidentRow>, StorageError> {
        let mut statement = self.connection.prepare(
            "
            SELECT id, scenario, severity, message, false_positive
            FROM incidents
            ORDER BY id DESC
            LIMIT ?1
            ",
        )?;

        let rows = statement.query_map([limit as i64], |row| {
            let fp_raw: Option<i64> = row.get(4)?;
            let false_positive = fp_raw.map(|value| value != 0);

            Ok(IncidentRow {
                id: row.get(0)?,
                scenario: row.get(1)?,
                severity: row.get(2)?,
                message: row.get(3)?,
                false_positive,
            })
        })?;

        let mut incidents = Vec::new();
        for row in rows {
            incidents.push(row?);
        }

        Ok(incidents)
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

    #[test]
    fn calculates_false_positive_rate_from_labeled_incidents() {
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

        let base_incident = Incident {
            scenario: Scenario::ExecutionRpcTimeout,
            severity: Severity::Warning,
            message: "rpc timeout burst".to_owned(),
            detected_at: UNIX_EPOCH,
            evidence: json!({"timeout_rate": 33.5}),
        };

        let first_id = match repository.insert(&base_incident) {
            Ok(id) => id,
            Err(_) => return,
        };
        let second_id = match repository.insert(&base_incident) {
            Ok(id) => id,
            Err(_) => return,
        };

        assert!(repository.mark_false_positive(first_id, true).is_ok());
        assert!(repository.mark_false_positive(second_id, false).is_ok());

        let rate = repository.false_positive_rate();
        assert!(rate.is_ok());
        let value = match rate {
            Ok(Some(v)) => v,
            _ => return,
        };

        assert_eq!(value, 0.5);
    }

    #[test]
    fn lists_recent_incidents_with_label_status() {
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

        let first = Incident {
            scenario: Scenario::ExecutionRpcTimeout,
            severity: Severity::Warning,
            message: "first".to_owned(),
            detected_at: UNIX_EPOCH,
            evidence: json!({}),
        };
        let second = Incident {
            scenario: Scenario::CpuPressure,
            severity: Severity::Warning,
            message: "second".to_owned(),
            detected_at: UNIX_EPOCH,
            evidence: json!({}),
        };

        let first_id = match repository.insert(&first) {
            Ok(id) => id,
            Err(_) => return,
        };
        let second_id = match repository.insert(&second) {
            Ok(id) => id,
            Err(_) => return,
        };

        assert!(repository.mark_false_positive(second_id, true).is_ok());

        let rows_result = repository.list_recent(2);
        assert!(rows_result.is_ok());
        let rows = match rows_result {
            Ok(rows) => rows,
            Err(_) => return,
        };

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, second_id);
        assert_eq!(rows[0].false_positive, Some(true));
        assert_eq!(rows[1].id, first_id);
        assert_eq!(rows[1].false_positive, None);
    }
}
