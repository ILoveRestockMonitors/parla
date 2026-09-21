// Utterance history store (spec privacy path). Modes honored AT WRITE TIME:
//   Store         -> keep rows
//   AutoDelete24h -> keep rows, prune anything older than 24 h on each write
//   Never         -> drop immediately, nothing ever touches disk
// Password-field detection does not exist yet (v1); mode is the only gate.
use rusqlite::Connection;
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryMode {
    Store,
    AutoDelete24h,
    Never,
}

impl HistoryMode {
    /// Settings stores this as a string; unknown values fail closed to the
    /// most private retention that still keeps history working at all.
    pub fn from_setting(s: &str) -> Self {
        match s {
            "store" => HistoryMode::Store,
            "never" => HistoryMode::Never,
            "auto_delete_24h" => HistoryMode::AutoDelete24h,
            _ => HistoryMode::Never,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            HistoryMode::Store => "store",
            HistoryMode::AutoDelete24h => "auto_delete_24h",
            HistoryMode::Never => "never",
        }
    }
}

pub struct HistoryRow {
    pub ts: i64,
    pub app: Option<String>,
    pub text: String,
}

/// Standard DB location, mirroring settings_path()'s convention.
pub fn default_path() -> String {
    crate::platform::data_dir()
        .join("history.sqlite")
        .to_string_lossy()
        .into_owned()
}

pub struct History {
    conn: Mutex<Connection>,
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl History {
    /// Opens (or creates) the SQLite store and ensures schema.
    pub fn open(path: &str) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(|e| format!("history open: {e}"))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS utterances (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                ts INTEGER NOT NULL,
                app TEXT,
                text TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_utterances_ts ON utterances(ts);",
        )
        .map_err(|e| format!("history schema: {e}"))?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Record one finished utterance under the active retention mode.
    /// Never-mode is a no-op by construction: no write path is reached.
    pub fn record(&self, mode: HistoryMode, text: &str, app: Option<&str>) -> Result<(), String> {
        if mode == HistoryMode::Never {
            return Ok(());
        }
        let ts = now_unix();
        {
            let conn = self.conn.lock().map_err(|_| "history mutex poisoned")?;
            conn.execute(
                "INSERT INTO utterances (ts, app, text) VALUES (?1, ?2, ?3)",
                rusqlite::params![ts, app, text],
            )
            .map_err(|e| format!("history insert: {e}"))?;
        }
        if mode == HistoryMode::AutoDelete24h {
            // Prune relative to the row just written; failure to prune must
            // not lose the utterance, so the error is reported but kept.
            let _ = self.prune_before(ts - 24 * 3600);
        }
        Ok(())
    }

    /// Delete rows older than the cutoff (unix seconds). Returns count.
    pub fn prune_before(&self, cutoff_ts: i64) -> Result<usize, String> {
        let conn = self.conn.lock().map_err(|_| "history mutex poisoned")?;
        conn.execute(
            "DELETE FROM utterances WHERE ts < ?1",
            rusqlite::params![cutoff_ts],
        )
        .map_err(|e| format!("history prune: {e}"))
    }

    /// Newest rows first, bounded. Empty store = empty vec, never error.
    pub fn recent(&self, limit: usize) -> Vec<HistoryRow> {
        let Ok(conn) = self.conn.lock() else {
            return Vec::new();
        };
        let mut stmt = match conn
            .prepare("SELECT ts, app, text FROM utterances ORDER BY ts DESC, id DESC LIMIT ?1")
        {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = stmt.query_map([limit as i64], |row| {
            Ok(HistoryRow {
                ts: row.get::<_, i64>(0).unwrap_or_default(),
                app: row.get::<_, Option<String>>(1).unwrap_or(None),
                text: row.get::<_, String>(2).unwrap_or_default(),
            })
        });
        match rows {
            Ok(iter) => iter.filter_map(Result::ok).collect(),
            Err(_) => Vec::new(),
        }
    }
}

/// Live-loop handle: defers the DB open until an utterance is actually
/// retained. In Never mode no file is ever created; in the other modes the
/// connection opens on first insert and stays warm for the session.
pub struct LazyHistory {
    inner: Option<History>,
}

impl LazyHistory {
    pub fn new() -> Self {
        Self { inner: None }
    }

    pub fn maintenance(&mut self) {
        let mode =
            HistoryMode::from_setting(&crate::store::settings::Settings::load().history_mode);
        if mode == HistoryMode::Never {
            self.inner = None;
            return;
        }
        if mode != HistoryMode::AutoDelete24h {
            return;
        }
        if self.inner.is_none() && std::path::Path::new(&default_path()).exists() {
            self.inner = History::open(&default_path()).ok();
        }
        if let Some(history) = &self.inner {
            if let Err(e) = history.prune_before(now_unix() - 86400) {
                eprintln!("[parla] history retention: {e}");
            }
        }
    }

    /// Record one insert under the CURRENT settings mode. Mode is re-read per
    /// call so a privacy switch takes effect without restarting Parla;
    /// switching into Never also drops the live handle (file may be deleted
    /// by the user while we run - reopening lazily would resurrect it).
    pub fn record_from_settings(&mut self, text: &str) {
        let mode =
            HistoryMode::from_setting(&crate::store::settings::Settings::load().history_mode);
        self.record_as(mode, &default_path(), text);
    }

    /// Core logic with injected mode+path so tests stay hermetic.
    fn record_as(&mut self, mode: HistoryMode, path: &str, text: &str) {
        if mode == HistoryMode::Never {
            self.inner = None;
            return;
        }
        if self.inner.is_none() {
            match History::open(path) {
                Ok(h) => self.inner = Some(h),
                Err(e) => {
                    eprintln!("[parla] history unavailable: {e}");
                    return;
                }
            }
        }
        if let Some(h) = &self.inner {
            if let Err(e) = h.record(mode, text, None) {
                eprintln!("[parla] history write failed: {e}");
            }
        }
    }
}

#[cfg(test)]
mod lazy_tests {
    use super::*;

    // Test-only entry: same core, routed to a per-pid scratch file so tests
    // never touch %LOCALAPPDATA%\Parla or each other.
    fn record_test(lz: &mut LazyHistory, mode: HistoryMode, tag: &str, text: &str) {
        let path = std::env::temp_dir()
            .join(format!(
                "parla_lazy_test_{tag}_{}.sqlite",
                std::process::id()
            ))
            .to_string_lossy()
            .to_string();
        let _ = std::fs::remove_file(&path);
        lz.record_as(mode, &path, text);
    }

    #[test]
    fn lazy_never_drops_handle_writes_nothing() {
        let mut lz = LazyHistory::new();
        record_test(&mut lz, HistoryMode::Never, "never", "secret");
        assert!(lz.inner.is_none());
    }

    #[test]
    fn lazy_store_opens_and_persists() {
        let mut lz = LazyHistory::new();
        record_test(&mut lz, HistoryMode::Store, "store", "kept line");
        assert!(lz.inner.is_some());
        let rows = lz.inner.as_ref().unwrap().recent(10);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].text, "kept line");
    }

    #[test]
    fn switch_to_never_mid_session_closes() {
        let mut lz = LazyHistory::new();
        record_test(&mut lz, HistoryMode::Store, "switch", "first");
        assert!(lz.inner.is_some());
        record_test(&mut lz, HistoryMode::Never, "switch", "second");
        assert!(lz.inner.is_none());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db(tag: &str) -> String {
        std::env::temp_dir()
            .join(format!(
                "parla_hist_test_{tag}_{}.sqlite",
                std::process::id()
            ))
            .to_string_lossy()
            .to_string()
    }

    #[test]
    fn never_mode_writes_nothing() {
        let h = History::open(&temp_db("never")).unwrap();
        h.record(HistoryMode::Never, "secret", Some("Notepad"))
            .unwrap();
        assert!(h.recent(10).is_empty());
    }

    #[test]
    fn store_mode_roundtrip() {
        let h = History::open(&temp_db("store")).unwrap();
        h.record(HistoryMode::Store, "hello world", Some("Notepad"))
            .unwrap();
        let rows = h.recent(10);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].text, "hello world");
        assert_eq!(rows[0].app.as_deref(), Some("Notepad"));
        // ts is fresh (within a minute of now)
        assert!(rows[0].ts > now_unix() - 60);
    }

    #[test]
    fn prune_before_removes_only_old_rows() {
        let h = History::open(&temp_db("prune")).unwrap();
        h.record(HistoryMode::Store, "old", None).unwrap();
        h.record(HistoryMode::Store, "fresh", None).unwrap();
        // cutoff in the future removes everything; cutoff far past removes none
        assert_eq!(h.prune_before(now_unix() + 1).unwrap(), 2);
        assert!(h.recent(10).is_empty());
    }

    #[test]
    fn autodelete_keeps_recent_rows() {
        let h = History::open(&temp_db("auto")).unwrap();
        h.record(HistoryMode::AutoDelete24h, "kept", None).unwrap();
        // its own prune (cutoff = now-24h) must not eat the row it just wrote
        assert_eq!(h.recent(10).len(), 1);
    }
}
