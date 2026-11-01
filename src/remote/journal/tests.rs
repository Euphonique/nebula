use std::collections::BTreeMap;

use anyhow::{Result, bail};

use super::*;
use crate::remote::{Backend, RemoteEntry, Stamp};

/// A server in memory; every write bumps the time so stamps change.
#[derive(Default)]
struct Mem {
    files: BTreeMap<String, (Vec<u8>, i64)>,
    dirs: std::collections::BTreeSet<String>,
    clock: i64,
}

impl Mem {
    fn with(files: &[(&str, &str)]) -> Self {
        let mut m = Mem::default();
        m.dirs.insert("/".into());
        for (p, c) in files {
            m.put(p, c.as_bytes());
        }
        m
    }

    fn put(&mut self, path: &str, data: &[u8]) {
        self.clock += 1;
        let mut dir = crate::remote::parent(path);
        while dir != "/" {
            self.dirs.insert(dir.clone());
            dir = crate::remote::parent(&dir);
        }
        self.files.insert(path.into(), (data.to_vec(), self.clock));
    }

    fn text(&self, path: &str) -> Option<String> {
        self.files.get(path).map(|(d, _)| String::from_utf8_lossy(d).into_owned())
    }
}

impl Backend for Mem {
    fn home(&mut self) -> Result<String> {
        Ok("/".into())
    }
    fn list(&mut self, path: &str) -> Result<Vec<RemoteEntry>> {
        if !self.dirs.contains(path) {
            bail!("no such folder");
        }
        let child = |p: &str| crate::remote::parent(p) == path && p != path;
        let mut out: Vec<RemoteEntry> = self
            .dirs
            .iter()
            .filter(|d| child(d))
            .map(|d| RemoteEntry { name: d.rsplit('/').next().unwrap().into(), path: d.clone(), is_dir: true, size: 0, mode: None, mtime: None })
            .collect();
        out.extend(self.files.iter().filter(|(p, _)| child(p)).map(|(p, (d, t))| RemoteEntry {
            name: p.rsplit('/').next().unwrap().into(),
            path: p.clone(),
            is_dir: false,
            size: d.len() as u64,
            mode: None,
            mtime: Some(*t),
        }));
        Ok(out)
    }
    fn read(&mut self, path: &str) -> Result<Vec<u8>> {
        match self.files.get(path) {
            Some((d, _)) => Ok(d.clone()),
            None => bail!("no such file"),
        }
    }
    fn write(&mut self, path: &str, data: &[u8]) -> Result<()> {
        if !self.dirs.contains(&crate::remote::parent(path)) {
            bail!("no such folder");
        }
        self.put(path, data);
        Ok(())
    }
    fn mkdir(&mut self, path: &str) -> Result<()> {
        if !self.dirs.insert(path.into()) {
            bail!("exists");
        }
        Ok(())
    }
    fn remove_file(&mut self, path: &str) -> Result<()> {
        self.files.remove(path).map(|_| ()).ok_or_else(|| anyhow::anyhow!("no such file"))
    }
    fn remove_dir(&mut self, path: &str) -> Result<()> {
        if self.files.keys().any(|p| p.starts_with(&format!("{path}/"))) || self.dirs.iter().any(|d| d.starts_with(&format!("{path}/"))) {
            bail!("not empty");
        }
        self.dirs.remove(path);
        Ok(())
    }
    fn rename(&mut self, from: &str, to: &str) -> Result<()> {
        let f = self.files.remove(from).ok_or_else(|| anyhow::anyhow!("no such file"))?;
        self.files.insert(to.into(), f);
        Ok(())
    }
    fn chmod(&mut self, _: &str, _: u32) -> Result<()> {
        Ok(())
    }
    fn ping(&mut self) -> Result<()> {
        Ok(())
    }
    fn stat(&mut self, path: &str) -> Result<Option<Stamp>> {
        Ok(self.files.get(path).map(|(d, t)| Stamp { size: d.len() as u64, mtime: Some(*t) }))
    }
}

/// Runs `f` through a recorder and commits it like the worker does.
fn record(m: &mut Mem, site: &str, id: u64, kind: Kind, f: impl FnOnce(&mut dyn Backend)) {
    let mut rec = Recorder::new(m, site);
    f(&mut rec);
    let actions = std::mem::take(&mut rec.actions);
    commit(site, id, kind, String::new(), actions, None);
}

fn undo_entry(m: &mut Mem, site: &str, id: u64, new_id: u64) -> Undone {
    let entry = get(site, id).unwrap();
    let mut rec = Recorder::new(m, site);
    let out = undo(&mut rec, site, &entry);
    let actions = std::mem::take(&mut rec.actions);
    commit(site, new_id, Kind::Undo, String::new(), actions, Some((entry.id, entry.kind)));
    out
}

#[test]
fn undo_restores_overwritten_new_and_deleted_files() {
    crate::remote::integration_tests::isolated_config();
    let site = "journal-basic";
    let mut m = Mem::with(&[("/www/index.php", "alt"), ("/www/old.css", "a{}")]);
    record(&mut m, site, 1, Kind::Publish, |b| {
        b.write("/www/index.php", b"neu").unwrap();
        crate::remote::write_creating(b, "/www/img/logo.svg", b"<svg/>").unwrap();
        b.remove_file("/www/old.css").unwrap();
    });
    let entry = get(site, 1).unwrap();
    assert_eq!(entry.file_count(), 3);
    assert!(changed_since(&mut m, &entry).unwrap().is_empty());

    let out = undo_entry(&mut m, site, 1, 2);
    assert!(out.failed.is_empty(), "{:?}", out.failed);
    assert_eq!(out.restored, 3);
    assert_eq!(m.text("/www/index.php").as_deref(), Some("alt"));
    assert_eq!(m.text("/www/old.css").as_deref(), Some("a{}"));
    assert!(m.text("/www/img/logo.svg").is_none());
    assert!(!m.dirs.contains("/www/img"), "created folder goes again");
    assert!(get(site, 1).unwrap().undone);

    // Undoing the undo brings the publish back.
    undo_entry(&mut m, site, 2, 3);
    assert_eq!(m.text("/www/index.php").as_deref(), Some("neu"));
    assert_eq!(m.text("/www/img/logo.svg").as_deref(), Some("<svg/>"));
    assert!(m.text("/www/old.css").is_none());
    assert!(!get(site, 1).unwrap().undone);
    assert!(get(site, 2).unwrap().undone);
}

#[test]
fn later_changes_are_reported() {
    crate::remote::integration_tests::isolated_config();
    let site = "journal-conflict";
    let mut m = Mem::with(&[("/a.txt", "1"), ("/b.txt", "1")]);
    record(&mut m, site, 1, Kind::Upload, |b| {
        b.write("/a.txt", b"2").unwrap();
        b.remove_file("/b.txt").unwrap();
    });
    m.put("/a.txt", b"von woanders");
    m.put("/b.txt", b"wieder da");
    let mut changed = changed_since(&mut m, &get(site, 1).unwrap()).unwrap();
    changed.sort();
    assert_eq!(changed, ["/a.txt", "/b.txt"]);
}

#[test]
fn repeated_saves_merge_and_unchanged_writes_are_skipped() {
    crate::remote::integration_tests::isolated_config();
    let site = "journal-saves";
    let mut m = Mem::with(&[("/s.css", "v1")]);
    for (id, text) in [(1, "v2"), (2, "v3"), (3, "v3")] {
        record(&mut m, site, id, Kind::Save, |b| b.write("/s.css", text.as_bytes()).unwrap());
    }
    let entries = list(site);
    assert_eq!(entries.len(), 1, "one entry for the editing session");
    assert!(entries[0].has_id(2));
    undo_entry(&mut m, site, 2, 4);
    assert_eq!(m.text("/s.css").as_deref(), Some("v1"));
}

#[test]
fn backup_read_is_reused_and_bak_not_recorded() {
    crate::remote::integration_tests::isolated_config();
    let site = "journal-bak";
    let mut m = Mem::with(&[("/x.php", "alt")]);
    let mut rec = Recorder::new(&mut m, site);
    crate::remote::backup_file(&mut rec, "/x.php").unwrap();
    rec.write("/x.php", b"neu").unwrap();
    assert_eq!(rec.actions.len(), 1);
    assert!(matches!(&rec.actions[0].change, Change::Wrote { prior: Prior::Kept(_), .. }));
}

#[test]
fn folder_delete_and_rename_undo() {
    crate::remote::integration_tests::isolated_config();
    let site = "journal-dirs";
    let mut m = Mem::with(&[("/d/a.txt", "a"), ("/d/sub/b.txt", "b"), ("/r.txt", "r")]);
    record(&mut m, site, 1, Kind::Delete, |b| crate::remote::remove_recursive(b, "/d").unwrap());
    assert!(!m.dirs.contains("/d"));
    record(&mut m, site, 2, Kind::Rename, |b| b.rename("/r.txt", "/s.txt").unwrap());
    undo_entry(&mut m, site, 1, 3);
    assert_eq!(m.text("/d/sub/b.txt").as_deref(), Some("b"));
    assert_eq!(m.text("/d/a.txt").as_deref(), Some("a"));
    undo_entry(&mut m, site, 2, 4);
    assert_eq!(m.text("/r.txt").as_deref(), Some("r"));
    assert!(m.text("/s.txt").is_none());
}
