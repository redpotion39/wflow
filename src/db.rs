use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};
use tracing::info;

use std::sync::Mutex;

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
    /// Opens the SQLite database at specified path or default `%APPDATA%\Wispr Flow\flow.sqlite`
    pub fn open_or_default(custom_path: Option<&Path>) -> Result<Self> {
        let path = match custom_path {
            Some(p) => p.to_path_buf(),
            None => {
                #[cfg(target_os = "windows")]
                {
                    let app_data = std::env::var("APPDATA").unwrap_or_else(|_| ".".to_string());
                    let dir = PathBuf::from(app_data).join("Wispr Flow");
                    let _ = std::fs::create_dir_all(&dir);
                    dir.join("flow.sqlite")
                }
                #[cfg(not(target_os = "windows"))]
                {
                    let data_dir = if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
                        PathBuf::from(xdg).join("wflow")
                    } else if let Ok(home) = std::env::var("HOME") {
                        PathBuf::from(home).join(".local").join("share").join("wflow")
                    } else {
                        PathBuf::from(".")
                    };
                    let _ = std::fs::create_dir_all(&data_dir);
                    data_dir.join("flow.sqlite")
                }
            }
        };

        info!("Opening SQLite database at: {:?}", path);
        let conn = Connection::open(&path).context("Failed to open SQLite database")?;
        let db = Self { conn: Mutex::new(conn) };
        db.init_tables()?;
        Ok(db)
    }

    /// Initializes tables matching Wispr Flow schema
    pub fn init_tables(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
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
            "#,
            [],
        )?;

        conn.execute(
            r#"
            CREATE TABLE IF NOT EXISTS Dictionary (
                id VARCHAR(36) NOT NULL PRIMARY KEY,
                word TEXT NOT NULL,
                replacement TEXT,
                isStarred TINYINT(1) NOT NULL DEFAULT 0,
                createdAt DATETIME,
                updatedAt DATETIME
            );
            "#,
            [],
        )?;

        Ok(())
    }

    /// Records a completed dictation into the History table
    pub fn save_history(&self, record: &HistoryRecord) -> Result<()> {
        let now: DateTime<Utc> = Utc::now();
        let num_words = record.formatted_text.split_whitespace().count() as i32;

        let conn = self.conn.lock().unwrap();
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
                "rust-1.0.0",
                record.formatted_text,
                "rust_native",
                "win32",
            ],
        )?;

        info!("Saved dictation {} to database", record.transcript_id);
        Ok(())
    }

    /// Fetches recent dictation history ordered by timestamp descending
    pub fn get_recent_history(&self, limit: usize) -> Result<Vec<HistoryRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT transcriptEntityId, COALESCE(asrText, ''), COALESCE(formattedText, ''), COALESCE(app, ''), textboxContents, duration, e2eLatency, COALESCE(timestamp, '') 
             FROM History ORDER BY timestamp DESC LIMIT ?1"
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
        for r in rows {
            if let Ok(rec) = r {
                list.push(rec);
            }
        }
        Ok(list)
    }

    /// Deletes a history entry by transcriptEntityId
    pub fn delete_history(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM History WHERE transcriptEntityId = ?1", params![id])?;
        Ok(())
    }

    /// Fetches all personal dictionary words
    pub fn get_dictionary_words(&self) -> Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT word FROM Dictionary WHERE word IS NOT NULL")?;
        let rows = stmt.query_map([], |row| row.get(0))?;

        let mut words = Vec::new();
        for r in rows {
            if let Ok(w) = r {
                words.push(w);
            }
        }
        Ok(words)
    }

    /// Fetches all personal dictionary entries (words and snippets)
    pub fn get_all_dictionary_entries(&self) -> Result<Vec<DictionaryEntry>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT id, word, replacement FROM Dictionary ORDER BY createdAt DESC")?;
        let rows = stmt.query_map([], |row| {
            Ok(DictionaryEntry {
                id: row.get(0)?,
                word: row.get(1)?,
                replacement: row.get(2)?,
            })
        })?;

        let mut list = Vec::new();
        for r in rows {
            if let Ok(entry) = r {
                list.push(entry);
            }
        }
        Ok(list)
    }

    /// Adds or replaces a dictionary vocabulary or snippet
    pub fn add_dictionary_entry(&self, word: &str, replacement: Option<&str>) -> Result<()> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO Dictionary (id, word, replacement, isStarred, createdAt, updatedAt) VALUES (?1, ?2, ?3, 0, ?4, ?4)",
            params![id, word.trim(), replacement.map(|s| s.trim()), now],
        )?;
        info!("Added dictionary entry: '{}' -> {:?}", word, replacement);
        Ok(())
    }

    /// Deletes a dictionary entry by id
    pub fn delete_dictionary_entry(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM Dictionary WHERE id = ?1", params![id])?;
        info!("Deleted dictionary entry {}", id);
        Ok(())
    }

    /// Fetches all active text snippets (trigger -> replacement)
    pub fn get_snippets(&self) -> Result<Vec<(String, String)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT word, replacement FROM Dictionary WHERE replacement IS NOT NULL AND TRIM(replacement) != ''")?;
        let rows = stmt.query_map([], |row| {
            let word: String = row.get(0)?;
            let repl: String = row.get(1)?;
            Ok((word, repl))
        })?;

        let mut list = Vec::new();
        for r in rows {
            if let Ok(pair) = r {
                list.push(pair);
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

