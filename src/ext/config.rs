//! Extension preferences: global ones in Nebula's config folder, workspace
//! ones in `<project>/.nova/Configuration.json` like Nova.

use std::collections::HashMap;
use std::path::PathBuf;

use serde_json::{Map, Value};

#[derive(Default)]
pub struct ConfigStore {
    global: Map<String, Value>,
    workspace: Map<String, Value>,
    root: Option<PathBuf>,
    defaults: HashMap<String, Value>,
}

fn global_path() -> PathBuf {
    crate::settings::config_dir().join("extension-config.json")
}

fn read(path: &std::path::Path) -> Map<String, Value> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| super::manifest::parse_lenient(&t))
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

impl ConfigStore {
    pub fn load() -> Self {
        Self { global: read(&global_path()), ..Default::default() }
    }

    pub fn set_workspace(&mut self, root: Option<PathBuf>) {
        if self.root == root {
            return;
        }
        self.workspace = root.as_ref().map(|r| read(&r.join(".nova").join("Configuration.json"))).unwrap_or_default();
        self.root = root;
    }

    pub fn set_defaults(&mut self, defaults: impl IntoIterator<Item = (String, Value)>) {
        self.defaults = defaults.into_iter().collect();
    }

    /// The explicitly set value, if any.
    pub fn explicit(&self, workspace: bool, key: &str) -> Option<&Value> {
        if workspace { self.workspace.get(key) } else { self.global.get(key) }
    }

    pub fn get(&self, workspace: bool, key: &str) -> Value {
        self.explicit(workspace, key)
            .or_else(|| self.defaults.get(key))
            .cloned()
            .unwrap_or(Value::Null)
    }

    /// Sets (or with `Null` removes) a value; returns the previous effective value.
    pub fn set(&mut self, workspace: bool, key: &str, value: Value) -> Value {
        let old = self.get(workspace, key);
        let map = if workspace { &mut self.workspace } else { &mut self.global };
        if value.is_null() {
            map.remove(key);
        } else {
            map.insert(key.to_string(), value);
        }
        self.save(workspace);
        old
    }

    fn save(&self, workspace: bool) {
        let (path, map) = if workspace {
            let Some(root) = &self.root else { return };
            let dir = root.join(".nova");
            let _ = std::fs::create_dir_all(&dir);
            (dir.join("Configuration.json"), &self.workspace)
        } else {
            let _ = std::fs::create_dir_all(crate::settings::config_dir());
            (global_path(), &self.global)
        };
        if let Ok(text) = serde_json::to_string_pretty(map) {
            let _ = std::fs::write(path, text);
        }
    }
}

impl ConfigStore {
    /// All values below `section.` as a nested object (workspace values win
    /// over global ones, which win over defaults), for LSP `workspace/configuration`.
    pub fn section(&self, section: &str) -> Value {
        let prefix = format!("{section}.");
        let mut flat: std::collections::BTreeMap<String, Value> = std::collections::BTreeMap::new();
        for (k, v) in &self.defaults {
            flat.insert(k.clone(), v.clone());
        }
        for (k, v) in self.global.iter().chain(self.workspace.iter()) {
            flat.insert(k.clone(), v.clone());
        }
        if let Some(v) = flat.get(section)
            && v.is_object()
        {
            return v.clone();
        }
        let mut root = Map::new();
        for (k, v) in flat {
            let Some(rest) = k.strip_prefix(&prefix) else { continue };
            let parts: Vec<&str> = rest.split('.').collect();
            let mut node = &mut root;
            for p in &parts[..parts.len() - 1] {
                let entry = node.entry(p.to_string()).or_insert_with(|| Value::Object(Map::new()));
                if !entry.is_object() {
                    *entry = Value::Object(Map::new());
                }
                node = entry.as_object_mut().unwrap();
            }
            node.insert(parts[parts.len() - 1].to_string(), v);
        }
        if root.is_empty() { Value::Null } else { Value::Object(root) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_sections() {
        let mut c = ConfigStore::default();
        c.set_defaults([("php.files.maxSize".to_string(), Value::from(5)), ("php.enable".to_string(), Value::from(true))]);
        c.global.insert("php.files.maxSize".into(), Value::from(9));
        let s = c.section("php");
        assert_eq!(s["files"]["maxSize"], 9);
        assert_eq!(s["enable"], true);
        assert!(c.section("other").is_null());
    }
}
