use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use std::path::Path;
use tracing::info;

use std::sync::Mutex;

const SCHEMA_VERSION: i64 = 1;
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct Database {
    conn: Mutex<Connection>,
}

#[derive(Debug, Clone)]
pub struct HistoryRecord {
    pub transcript_id: String,
    pub asr_text: String,
    pub formatted_text: String,
    pub app: String,
    pub textbox_contents: Option<String>,
    pub duration_secs: f64,
    pub e2e_latency_secs: f64,
    pub timestamp: String,
}

impl Database {
    /// Opens the SQLite database at the specified path or the default data dir.
    pub fn open_or_default(custom_path: Option<&Path>) -> Result<Self> {
        let path = match custom_path {
            Some(p) => p.to_path_buf(),
            None => {
                let dir = crate::paths::ensure_dir(crate::paths::data_dir());
                dir.join("flow.sqlite")
            }
        };

        info!("Opening SQLite database at: {:?}", path);
        let conn = Connection::open(&path).context("Failed to open SQLite database")?;

        // WAL keeps the GUI reads responsive while the worker writes.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;

        let db = Self {
            conn: Mutex::new(conn),
        };
        db.init_tables()?;
        Ok(db)
    }

    /// Initializes tables matching the Wispr Flow schema and applies migrations.
    pub fn init_tables(&self) -> Result<()> {
        let conn = self.lock();
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS History (
                transcriptEntityId VARCHAR(36) NOT NULL UNIQUE PRIMARY KEY,
                asrText TEXT,
                formattedText TEXT,
                editedText TEXT,
                timestamp DATETIME,
                audio BLOB,
                screenshot BLOB,
                additionalContext JSON,
                status VARCHAR(255),
                app VARCHAR(255),
                url VARCHAR(255),
                e2eLatency FLOAT,
                needsUploading TINYINT(1) NOT NULL DEFAULT 0,
                duration FLOAT,
                numWords INTEGER,
                textboxContents TEXT,
                appVersion VARCHAR(255) NOT NULL DEFAULT '0.0.0',
                language TEXT,
                detectedLanguage TEXT,
                pastedText TEXT,
                transcriptOrigin TEXT,
                platform VARCHAR(255)
            );

            CREATE TABLE IF NOT EXISTS Dictionary (
                id VARCHAR(36) NOT NULL PRIMARY KEY,
                word TEXT NOT NULL,
                replacement TEXT,
                isStarred TINYINT(1) NOT NULL DEFAULT 0,
                createdAt DATETIME,
                updatedAt DATETIME
            );

            CREATE INDEX IF NOT EXISTS idx_history_timestamp
                ON History(timestamp DESC);
            CREATE INDEX IF NOT EXISTS idx_dictionary_word
                ON Dictionary(word);
            "#,
        )?;

        let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version < SCHEMA_VERSION {
            conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            info!(
                "Migrated database schema from version {} to {}",
                version, SCHEMA_VERSION
            );
        }

        Ok(())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        // Recover from a poisoned mutex instead of panicking the whole app.
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Records a completed dictation into the History table.
    pub fn save_history(&self, record: &HistoryRecord) -> Result<()> {
        let now: DateTime<Utc> = Utc::now();
        let num_words = record.formatted_text.split_whitespace().count() as i32;

        let conn = self.lock();
        conn.execute(
            r#"
            INSERT OR REPLACE INTO History (
                transcriptEntityId,
                asrText,
                formattedText,
                timestamp,
                status,
                app,
                e2eLatency,
                duration,
                numWords,
                textboxContents,
                appVersion,
                pastedText,
                transcriptOrigin,
                platform
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14);
            "#,
            params![
                record.transcript_id,
                record.asr_text,
                record.formatted_text,
                now.to_rfc3339(),
                "DONE",
                record.app,
                record.e2e_latency_secs,
                record.duration_secs,
                num_words,
                record.textbox_contents,
                APP_VERSION,
                record.formatted_text,
                "rust_native",
                std::env::consts::OS,
            ],
        )?;

        info!("Saved dictation {} to database", record.transcript_id);
        Ok(())
    }

    /// Fetches recent dictation history ordered by timestamp descending.
    pub fn get_recent_history(&self, limit: usize) -> Result<Vec<HistoryRecord>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT transcriptEntityId, COALESCE(asrText, ''), COALESCE(formattedText, ''), COALESCE(app, ''), textboxContents, duration, e2eLatency, COALESCE(timestamp, '')
             FROM History ORDER BY timestamp DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            let duration: f64 = row.get::<_, Option<f64>>(5)?.unwrap_or(0.0);
            let latency: f64 = row.get::<_, Option<f64>>(6)?.unwrap_or(0.0);
            Ok(HistoryRecord {
                transcript_id: row.get(0)?,
                asr_text: row.get(1)?,
                formatted_text: row.get(2)?,
                app: row.get(3)?,
                textbox_contents: row.get(4)?,
                duration_secs: duration,
                e2e_latency_secs: latency,
                timestamp: row.get(7)?,
            })
        })?;

        let mut list = Vec::new();
        for row in rows {
            match row {
                Ok(record) => list.push(record),
                Err(err) => tracing::warn!("Skipping unreadable history row: {}", err),
            }
        }
        Ok(list)
    }

    /// Deletes a history entry by transcriptEntityId.
    pub fn delete_history(&self, id: &str) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "DELETE FROM History WHERE transcriptEntityId = ?1",
            params![id],
        )?;
        Ok(())
    }

    /// Removes every stored dictation (used by the privacy panel).
    pub fn clear_history(&self) -> Result<usize> {
        let conn = self.lock();
        let removed = conn.execute("DELETE FROM History", [])?;
        info!("Cleared {} history rows", removed);
        Ok(removed)
    }

    /// Fetches all personal dictionary words.
    pub fn get_dictionary_words(&self) -> Result<Vec<String>> {
        let conn = self.lock();
        let mut stmt = conn.prepare("SELECT word FROM Dictionary WHERE word IS NOT NULL")?;
        let rows = stmt.query_map([], |row| row.get(0))?;

        let mut words = Vec::new();
        for row in rows {
            match row {
                Ok(word) => words.push(word),
                Err(err) => tracing::warn!("Skipping unreadable dictionary row: {}", err),
            }
        }
        Ok(words)
    }

    /// Fetches all personal dictionary entries (words and snippets).
    pub fn get_all_dictionary_entries(&self) -> Result<Vec<DictionaryEntry>> {
        let conn = self.lock();
        let mut stmt =
            conn.prepare("SELECT id, word, replacement FROM Dictionary ORDER BY createdAt DESC")?;
        let rows = stmt.query_map([], |row| {
            Ok(DictionaryEntry {
                id: row.get(0)?,
                word: row.get(1)?,
                replacement: row.get(2)?,
            })
        })?;

        let mut list = Vec::new();
        for row in rows {
            match row {
                Ok(entry) => list.push(entry),
                Err(err) => tracing::warn!("Skipping unreadable dictionary row: {}", err),
            }
        }
        Ok(list)
    }

    /// Adds or replaces a dictionary vocabulary or snippet.
    pub fn add_dictionary_entry(&self, word: &str, replacement: Option<&str>) -> Result<()> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let conn = self.lock();
        conn.execute(
            "INSERT INTO Dictionary (id, word, replacement, isStarred, createdAt, updatedAt) VALUES (?1, ?2, ?3, 0, ?4, ?4)",
            params![id, word.trim(), replacement.map(|s| s.trim()), now],
        )?;
        info!("Added dictionary entry: '{}' -> {:?}", word, replacement);
        Ok(())
    }

    /// Deletes a dictionary entry by id.
    pub fn delete_dictionary_entry(&self, id: &str) -> Result<()> {
        let conn = self.lock();
        conn.execute("DELETE FROM Dictionary WHERE id = ?1", params![id])?;
        info!("Deleted dictionary entry {}", id);
        Ok(())
    }

    /// Fetches all active text snippets (trigger -> replacement).
    pub fn get_snippets(&self) -> Result<Vec<(String, String)>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT word, replacement FROM Dictionary WHERE replacement IS NOT NULL AND TRIM(replacement) != ''",
        )?;
        let rows = stmt.query_map([], |row| {
            let word: String = row.get(0)?;
            let repl: String = row.get(1)?;
            Ok((word, repl))
        })?;

        let mut list = Vec::new();
        for row in rows {
            match row {
                Ok(pair) => list.push(pair),
                Err(err) => tracing::warn!("Skipping unreadable snippet row: {}", err),
            }
        }
        Ok(list)
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DictionaryEntry {
    pub id: String,
    pub word: String,
    pub replacement: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> (tempfile::TempDir, Database) {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open_or_default(Some(&dir.path().join("test.sqlite"))).unwrap();
        (dir, db)
    }

    fn record(id: &str) -> HistoryRecord {
        HistoryRecord {
            transcript_id: id.to_string(),
            asr_text: "hello".to_string(),
            formatted_text: "Hello.".to_string(),
            app: "test".to_string(),
            textbox_contents: None,
            duration_secs: 1.5,
            e2e_latency_secs: 0.2,
            timestamp: Utc::now().to_rfc3339(),
        }
    }

    #[test]
    fn stores_and_reads_history() {
        let (_dir, db) = temp_db();
        db.save_history(&record("a")).unwrap();
        db.save_history(&record("b")).unwrap();
        let rows = db.get_recent_history(10).unwrap();
        assert_eq!(rows.len(), 2);
        db.delete_history("a").unwrap();
        assert_eq!(db.get_recent_history(10).unwrap().len(), 1);
        assert_eq!(db.clear_history().unwrap(), 1);
        assert!(db.get_recent_history(10).unwrap().is_empty());
    }

    #[test]
    fn dictionary_and_snippets() {
        let (_dir, db) = temp_db();
        db.add_dictionary_entry("Wflow", None).unwrap();
        db.add_dictionary_entry("brb", Some("be right back"))
            .unwrap();

        let words = db.get_dictionary_words().unwrap();
        assert!(words.contains(&"Wflow".to_string()));

        let snippets = db.get_snippets().unwrap();
        assert_eq!(
            snippets,
            vec![("brb".to_string(), "be right back".to_string())]
        );

        let entries = db.get_all_dictionary_entries().unwrap();
        assert_eq!(entries.len(), 2);
        db.delete_dictionary_entry(&entries[0].id).unwrap();
        assert_eq!(db.get_all_dictionary_entries().unwrap().len(), 1);
    }

    #[test]
    fn schema_version_is_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("version.sqlite");
        let _db = Database::open_or_default(Some(&path)).unwrap();
        let conn = Connection::open(&path).unwrap();
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        let journal: String = conn
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .unwrap();
        assert_eq!(journal.to_lowercase(), "wal");
    }
}
