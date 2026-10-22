//! JSON files that several Nebula windows (processes) share, like
//! settings.json and sites.json. Each window remembers the state it last
//! read or wrote ("base"). Saving merges three ways: what this window changed
//! since then wins, everything else comes from the file as it is now – so a
//! window never overwrites another window's changes. Windows also read the
//! file again when another one changed it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use anyhow::Result;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

struct Synced {
    /// The file's content when this window last read or wrote it.
    base: Value,
    /// Its modification time then.
    stamp: Option<SystemTime>,
}

static FILES: Mutex<Option<HashMap<PathBuf, Synced>>> = Mutex::new(None);

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn read(path: &Path) -> Option<Value> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

fn remember(path: &Path, base: Value) {
    let mut files = FILES.lock().unwrap();
    files.get_or_insert_with(HashMap::new).insert(path.to_path_buf(), Synced { base, stamp: modified(path) });
}

/// Reads the file (`None` if missing or broken) and remembers it as the base.
pub fn load<T: DeserializeOwned + Serialize>(path: &Path) -> Option<T> {
    let value = read(path)?;
    let parsed: T = serde_json::from_value(value).ok()?;
    // The base is what the program makes of it (defaults filled in).
    remember(path, serde_json::to_value(&parsed).unwrap_or(Value::Null));
    Some(parsed)
}

/// Writes `value`, keeping changes other windows made meanwhile. `keep` are
/// top-level keys this window never writes (they stay as in the file).
pub fn save<T: Serialize>(path: &Path, value: &T, keep: &[&str]) -> Result<()> {
    let mine = serde_json::to_value(value)?;
    let base = FILES.lock().unwrap().as_ref().and_then(|f| f.get(path)).map(|s| s.base.clone());
    let disk = read(path);
    let mut merged = match (&base, &disk) {
        (Some(base), Some(disk)) => merge3(base, &mine, disk),
        _ => mine,
    };
    if let (Some(disk), Value::Object(m)) = (&disk, &mut merged) {
        for key in keep {
            match disk.get(*key) {
                Some(v) => m.insert(key.to_string(), v.clone()),
                None => m.remove(*key),
            };
        }
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // A name of its own per process: two windows may save at the same moment.
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&tmp, serde_json::to_vec_pretty(&merged)?)?;
    std::fs::rename(&tmp, path)?;
    remember(path, merged);
    Ok(())
}

/// If another window changed the file: `current` with those changes (and this
/// window's unsaved changes kept); `None` if nothing changed.
pub fn reload<T: DeserializeOwned + Serialize>(path: &Path, current: &T) -> Option<T> {
    let stamp = modified(path);
    let base = {
        let files = FILES.lock().unwrap();
        let synced = files.as_ref()?.get(path)?;
        if synced.stamp == stamp {
            return None;
        }
        synced.base.clone()
    };
    let disk = read(path)?;
    let mine = serde_json::to_value(current).ok()?;
    let merged: T = serde_json::from_value(merge3(&base, &mine, &disk)).ok()?;
    let disk_parsed: T = serde_json::from_value(disk).ok()?;
    remember(path, serde_json::to_value(&disk_parsed).ok()?);
    Some(merged)
}

/// Three-way merge: where `mine` differs from `base` it wins, else `theirs`.
/// Objects are merged key by key; lists and values are taken as a whole.
pub fn merge3(base: &Value, mine: &Value, theirs: &Value) -> Value {
    if mine == base {
        return theirs.clone();
    }
    if theirs == base || mine == theirs {
        return mine.clone();
    }
    match (base, mine, theirs) {
        (Value::Object(b), Value::Object(m), Value::Object(t)) => {
            let mut out = Map::new();
            let keys: std::collections::BTreeSet<&String> = b.keys().chain(m.keys()).chain(t.keys()).collect();
            for k in keys {
                let merged = match (b.get(k), m.get(k), t.get(k)) {
                    (b, Some(m), Some(t)) => Some(merge3(b.unwrap_or(&Value::Null), m, t)),
                    // Removed by one side, unchanged by the other: removed.
                    (Some(b), Some(m), None) => (m != b).then(|| m.clone()),
                    (Some(b), None, Some(t)) => (t != b).then(|| t.clone()),
                    (None, Some(v), None) | (None, None, Some(v)) => Some(v.clone()),
                    _ => None,
                };
                if let Some(v) = merged {
                    out.insert(k.clone(), v);
                }
            }
            Value::Object(out)
        }
        // Both changed the same value: this window's choice wins.
        _ => mine.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn merges_changes_of_both_windows() {
        let base = json!({ "a": 1, "b": { "x": 1, "y": 1 }, "list": [1], "gone": 5 });
        let mine = json!({ "a": 2, "b": { "x": 1, "y": 2 }, "list": [1], "gone": 5 });
        let theirs = json!({ "a": 1, "b": { "x": 3, "y": 1 }, "list": [1, 2], "new": true });
        assert_eq!(merge3(&base, &mine, &theirs), json!({ "a": 2, "b": { "x": 3, "y": 2 }, "list": [1, 2], "new": true }));
    }

    #[test]
    fn two_windows_keep_each_others_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.json");
        std::fs::write(&path, r#"{ "agents": ["claude"], "width": 300, "session": "A" }"#).unwrap();
        let a: Value = load(&path).unwrap();
        // Window B (simulated): adds an agent and saves.
        let mut b = a.clone();
        b["agents"] = json!(["claude", "ollama"]);
        let b_base = a.clone();
        let disk = read(&path).unwrap();
        std::fs::write(&path, serde_json::to_vec(&merge3(&b_base, &b, &disk)).unwrap()).unwrap();
        // Window A changed the width and saves without the session (not its to write).
        let mut a2 = a.clone();
        a2["width"] = json!(420);
        a2["session"] = json!("B");
        save(&path, &a2, &["session"]).unwrap();
        assert_eq!(read(&path).unwrap(), json!({ "agents": ["claude", "ollama"], "width": 420, "session": "A" }));
    }
}
