use std::sync::Arc;

use serde_json::Value;

use crate::kernel::PanicLog;
use crate::memory::EpisodicEntry;
use crate::security::AuditEntry;

pub type SharedStore = Option<Arc<PersistentStore>>;

pub struct PersistentStore {
    conn: std::sync::Mutex<rusqlite::Connection>,
}

impl PersistentStore {
    pub fn open(path: &str) -> Result<Self, String> {
        let conn = rusqlite::Connection::open(path)
            .map_err(|e| format!("Failed to open SQLite database at {}: {}", path, e))?;

        let store = PersistentStore {
            conn: std::sync::Mutex::new(conn),
        };
        store.init_tables()?;
        Ok(store)
    }

    fn init_tables(&self) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS episodic_entries (
                event_id TEXT PRIMARY KEY,
                pid TEXT NOT NULL,
                event_type TEXT NOT NULL,
                payload TEXT NOT NULL,
                error TEXT,
                timestamp TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_episodic_pid ON episodic_entries(pid);
            CREATE INDEX IF NOT EXISTS idx_episodic_timestamp ON episodic_entries(timestamp);

            CREATE TABLE IF NOT EXISTS audit_entries (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                action TEXT NOT NULL,
                result TEXT NOT NULL,
                reason TEXT NOT NULL,
                timestamp TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_audit_timestamp ON audit_entries(timestamp);

            CREATE TABLE IF NOT EXISTS panic_logs (
                pid TEXT NOT NULL,
                reason TEXT NOT NULL,
                timestamp TEXT NOT NULL,
                tokens_used INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_panic_timestamp ON panic_logs(timestamp);
            ",
        )
        .map_err(|e| format!("Failed to initialize database tables: {}", e))?;
        Ok(())
    }

    pub fn append_episodic(&self, entry: &EpisodicEntry) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO episodic_entries (event_id, pid, event_type, payload, error, timestamp)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                entry.event_id,
                entry.pid,
                entry.event_type,
                entry.payload.to_string(),
                entry.error,
                entry.timestamp,
            ],
        )
        .map_err(|e| format!("Failed to insert episodic entry: {}", e))?;
        Ok(())
    }

    pub fn query_episodic(
        &self,
        pid: Option<&str>,
        limit: usize,
    ) -> Result<Vec<EpisodicEntry>, String> {
        let conn = self.conn.lock().unwrap();
        let (sql, params): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = if let Some(pid_val) = pid {
            (
                "SELECT event_id, pid, event_type, payload, error, timestamp
                 FROM episodic_entries WHERE pid = ?1
                 ORDER BY timestamp DESC LIMIT ?2",
                vec![Box::new(pid_val.to_string()), Box::new(limit as i64)],
            )
        } else {
            (
                "SELECT event_id, pid, event_type, payload, error, timestamp
                 FROM episodic_entries
                 ORDER BY timestamp DESC LIMIT ?1",
                vec![Box::new(limit as i64)],
            )
        };

        let mut stmt = conn
            .prepare(sql)
            .map_err(|e| format!("Failed to prepare query: {}", e))?;

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();

        let rows = stmt
            .query_map(param_refs.as_slice(), Self::map_episodic_row)
            .map_err(|e| format!("Failed to query episodic entries: {}", e))?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| format!("Failed to read episodic row: {}", e))?);
        }
        Ok(results)
    }

    fn map_episodic_row(row: &rusqlite::Row) -> rusqlite::Result<EpisodicEntry> {
        let payload_str: String = row.get(3)?;
        let payload: Value =
            serde_json::from_str(&payload_str).unwrap_or(serde_json::json!({"raw": payload_str}));
        Ok(EpisodicEntry {
            event_id: row.get(0)?,
            pid: row.get(1)?,
            event_type: row.get(2)?,
            payload,
            error: row.get(4)?,
            timestamp: row.get(5)?,
        })
    }

    pub fn append_audit(&self, entry: &AuditEntry) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO audit_entries (action, result, reason, timestamp)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                entry.action.to_string(),
                entry.result,
                entry.reason,
                entry.timestamp,
            ],
        )
        .map_err(|e| format!("Failed to insert audit entry: {}", e))?;
        Ok(())
    }

    pub fn query_audit(&self, limit: usize) -> Result<Vec<AuditEntry>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT action, result, reason, timestamp
                 FROM audit_entries ORDER BY timestamp DESC LIMIT ?1",
            )
            .map_err(|e| format!("Failed to prepare audit query: {}", e))?;

        let rows = stmt
            .query_map(rusqlite::params![limit as i64], |row| {
                let action_str: String = row.get(0)?;
                let action: Value =
                    serde_json::from_str(&action_str).unwrap_or(serde_json::json!({"raw": action_str}));
                Ok(AuditEntry {
                    action,
                    result: row.get(1)?,
                    reason: row.get(2)?,
                    timestamp: row.get(3)?,
                })
            })
            .map_err(|e| format!("Failed to query audit entries: {}", e))?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| format!("Failed to read audit row: {}", e))?);
        }
        Ok(results)
    }

    pub fn append_panic(&self, entry: &PanicLog) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO panic_logs (pid, reason, timestamp, tokens_used)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![entry.pid, entry.reason, entry.timestamp, entry.tokens_used],
        )
        .map_err(|e| format!("Failed to insert panic log: {}", e))?;
        Ok(())
    }

    pub fn query_panic_log(&self, limit: usize) -> Result<Vec<PanicLog>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT pid, reason, timestamp, tokens_used
                 FROM panic_logs ORDER BY timestamp DESC LIMIT ?1",
            )
            .map_err(|e| format!("Failed to prepare panic query: {}", e))?;

        let rows = stmt
            .query_map(rusqlite::params![limit as i64], |row| {
                Ok(PanicLog {
                    pid: row.get(0)?,
                    reason: row.get(1)?,
                    timestamp: row.get(2)?,
                    tokens_used: row.get(3)?,
                })
            })
            .map_err(|e| format!("Failed to query panic logs: {}", e))?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| format!("Failed to read panic row: {}", e))?);
        }
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::EpisodicEntry;
    use crate::kernel::PanicLog;
    use serde_json::json;

    fn test_store() -> PersistentStore {
        PersistentStore::open(":memory:").unwrap()
    }

    #[test]
    fn test_append_and_query_episodic() {
        let store = test_store();
        let entry = EpisodicEntry {
            event_id: "evt-1".to_string(),
            pid: "proc-1".to_string(),
            event_type: "TEST".to_string(),
            payload: json!({"key": "value"}),
            error: None,
            timestamp: "2026-01-01T00:00:00Z".to_string(),
        };
        store.append_episodic(&entry).unwrap();

        let results = store.query_episodic(Some("proc-1"), 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].event_id, "evt-1");
        assert_eq!(results[0].pid, "proc-1");
    }

    #[test]
    fn test_query_episodic_empty() {
        let store = test_store();
        let results = store.query_episodic(None, 10).unwrap();
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_append_and_query_audit() {
        let store = test_store();
        store
            .append_audit(&AuditEntry {
                action: json!({"action": "read"}),
                result: "ALLOWED".to_string(),
                reason: "verified".to_string(),
                timestamp: "2026-01-01T00:00:00Z".to_string(),
            })
            .unwrap();

        let results = store.query_audit(10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].result, "ALLOWED");
    }

    #[test]
    fn test_append_and_query_panic() {
        let store = test_store();
        store
            .append_panic(&PanicLog {
                pid: "proc-1".to_string(),
                reason: "out of tokens".to_string(),
                timestamp: "2026-01-01T00:00:00Z".to_string(),
                tokens_used: 500,
            })
            .unwrap();

        let results = store.query_panic_log(10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].reason, "out of tokens");
        assert_eq!(results[0].tokens_used, 500);
    }

    #[test]
    fn test_multiple_entries_respect_limit() {
        let store = test_store();
        for i in 0..5 {
            store
                .append_episodic(&EpisodicEntry {
                    event_id: format!("evt-{}", i),
                    pid: "proc-1".to_string(),
                    event_type: "TEST".to_string(),
                    payload: json!({"i": i}),
                    error: None,
                    timestamp: format!("2026-01-{:02}T00:00:00Z", i + 1),
                })
                .unwrap();
        }

        let all = store.query_episodic(Some("proc-1"), 10).unwrap();
        assert_eq!(all.len(), 5);

        let limited = store.query_episodic(Some("proc-1"), 2).unwrap();
        assert_eq!(limited.len(), 2);
    }
}
