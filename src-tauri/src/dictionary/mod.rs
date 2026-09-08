// Custom dictionary / replacement rules / snippets.
// Applied TWICE per spec §7.6: (1) ASR hotword biasing list, (2) deterministic
// pre-pass BEFORE the LLM + preferred-spellings block inside the prompt.
pub mod apply;

use rusqlite::Connection;
use std::sync::Mutex;

#[derive(Clone, Debug)]
pub struct Entry {
    pub term: String,                // preferred spelling ("Supabase", "Okafor")
    pub replacement: Option<String>, // rule: "super base" -> Some("Supabase")
    pub snippet: Option<String>,     // voice cue -> expansion block
}

/// Immutable vocabulary captured at utterance start.  Keeping aliases and
/// canonical spellings together prevents a dictionary edit during inference
/// from changing the meaning of an in-flight utterance.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct VocabularySnapshot {
    pub version: u64,
    pub canonical: Vec<String>,
    pub aliases: Vec<(String, String)>,
}

pub struct Dictionary {
    conn: Mutex<Connection>,
}

impl Dictionary {
    /// One-time, removable vocabulary migration. A later user deletion is
    /// respected; existing aliases are never overwritten by a migration.
    pub fn seed_reliability_v2(&self) -> Result<(), String> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| "dictionary lock unavailable")?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS migrations (name TEXT PRIMARY KEY);")
            .map_err(|e| e.to_string())?;
        let applied: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM migrations WHERE name='reliability-v2')",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if !applied {
            for (term, canonical) in [
                ("Claude", None),
                ("claw ed", Some("Claude")),
                ("Parakeet", None),
                ("parakee", Some("Parakeet")),
            ] {
                tx.execute("INSERT INTO entries(term,replacement) SELECT ?1,?2 WHERE NOT EXISTS(SELECT 1 FROM entries WHERE lower(term)=lower(?1))", rusqlite::params![term,canonical]).map_err(|e| e.to_string())?;
            }
            tx.execute("INSERT INTO migrations(name) VALUES('reliability-v2')", [])
                .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())
    }
    /// Opens (or creates) the SQLite store and ensures schema.
    pub fn open(path: &str) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(|e| format!("dict open: {e}"))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS entries (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                term TEXT NOT NULL UNIQUE,
                replacement TEXT,
                snippet TEXT
            );",
        )
        .map_err(|e| format!("dict schema: {e}"))?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Insert or update an entry (term is the stable key).
    pub fn add_entry(&self, entry: &Entry) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|_| "dict mutex poisoned")?;
        conn.execute(
            "INSERT INTO entries (term, replacement, snippet) VALUES (?1, ?2, ?3)
             ON CONFLICT(term) DO UPDATE SET replacement=excluded.replacement, snippet=excluded.snippet",
            rusqlite::params![entry.term, entry.replacement, entry.snippet],
        )
        .map_err(|e| format!("dict insert: {e}"))?;
        Ok(())
    }

    /// All entries, alphabetical. Empty store = empty vec, never error.
    pub fn all_entries(&self) -> Vec<Entry> {
        let Ok(conn) = self.conn.lock() else {
            return Vec::new();
        };
        let mut stmt =
            match conn.prepare("SELECT term, replacement, snippet FROM entries ORDER BY term") {
                Ok(s) => s,
                Err(_) => return Vec::new(),
            };
        let rows = stmt.query_map([], |row| {
            Ok(Entry {
                term: row.get::<_, String>(0).unwrap_or_default(),
                replacement: row.get::<_, Option<String>>(1).unwrap_or(None),
                snippet: row.get::<_, Option<String>>(2).unwrap_or(None),
            })
        });
        match rows {
            Ok(iter) => iter.filter_map(Result::ok).collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Delete an entry by exact term. Returns whether a row was removed.
    pub fn remove_entry(&self, term: &str) -> Result<bool, String> {
        let conn = self.conn.lock().map_err(|_| "dict mutex poisoned")?;
        let n = conn
            .execute(
                "DELETE FROM entries WHERE term = ?1",
                rusqlite::params![term],
            )
            .map_err(|e| format!("dict delete: {e}"))?;
        Ok(n > 0)
    }

    /// Hotword list for ASR initial_prompt/keyterm biasing: plain terms only.
    pub fn hotwords(&self) -> Vec<String> {
        self.snapshot().canonical
    }

    pub fn snapshot(&self) -> VocabularySnapshot {
        let entries = self.all_entries();
        let mut canonical = Vec::new();
        let mut aliases = Vec::new();
        for e in &entries {
            if e.snippet.is_some() {
                continue;
            }
            let target = e.replacement.as_deref().unwrap_or(&e.term).trim();
            if target.is_empty() {
                continue;
            }
            if !canonical
                .iter()
                .any(|x: &String| x.eq_ignore_ascii_case(target))
            {
                canonical.push(target.to_string());
            }
            if e.term != target {
                aliases.push((e.term.clone(), target.to_string()));
            }
        }
        // A deterministic content version is enough for provenance and does
        // not require a mutable database counter.
        let mut version = 1469598103934665603u64;
        for s in canonical
            .iter()
            .chain(aliases.iter().flat_map(|(a, b)| [a, b]))
        {
            for byte in s.as_bytes() {
                version = (version ^ *byte as u64).wrapping_mul(1099511628211);
            }
        }
        VocabularySnapshot {
            version,
            canonical,
            aliases,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> String {
        let dir = std::env::temp_dir();
        // unique per test run; stale files from crashed runs are harmless
        dir.join(format!(
            "parla_dict_test_{}.sqlite",
            std::process::id() as u64 + rand_suffix()
        ))
        .to_string_lossy()
        .to_string()
    }

    fn rand_suffix() -> u64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.subsec_nanos() as u64)
            .unwrap_or(0)
    }

    #[test]
    fn roundtrip_and_hotwords() {
        let path = temp_db();
        let dict = Dictionary::open(&path).expect("open");
        dict.add_entry(&Entry {
            term: "Supabase".into(),
            replacement: None,
            snippet: None,
        })
        .unwrap();
        dict.add_entry(&Entry {
            term: "super base".into(),
            replacement: Some("Supabase".into()),
            snippet: None,
        })
        .unwrap();

        assert_eq!(dict.hotwords(), vec!["Supabase".to_string()]);
        assert_eq!(dict.all_entries().len(), 2);

        // upsert same term updates, not duplicates
        dict.add_entry(&Entry {
            term: "Supabase".into(),
            replacement: Some("Supabase".into()),
            snippet: None,
        })
        .unwrap();
        assert_eq!(dict.all_entries().len(), 2);

        // reopen: persistence across restarts
        drop(dict);
        let dict2 = Dictionary::open(&path).expect("reopen");
        assert_eq!(dict2.all_entries().len(), 2);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn apply_uses_store_terms() {
        let path = temp_db();
        let dict = Dictionary::open(&path).unwrap();
        dict.add_entry(&Entry {
            term: "super base".into(),
            replacement: Some("Supabase".into()),
            snippet: None,
        })
        .unwrap();
        let cleaned =
            apply::apply_replacements("email okafor about super base", &dict.all_entries());
        assert_eq!(cleaned, "email okafor about Supabase");
        let _ = std::fs::remove_file(&path);
    }
}
