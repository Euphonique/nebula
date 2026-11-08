//! Saving without risking the file: the new contents go into a temporary
//! file next to it first, which then replaces the original in one step. If
//! writing breaks off (network drive gone, disk full), the old file stays
//! complete.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// Writes `bytes` to `path` so that `path` holds either the old or the new
/// contents, never a part. Attributes and permissions of an existing file stay.
pub fn write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        // A new file: nothing to lose.
        return std::fs::write(path, bytes);
    };
    // Replacing a link would turn it into a plain file; a read-only file
    // gets the usual error from writing.
    if meta.file_type().is_symlink() || meta.permissions().readonly() {
        return std::fs::write(path, bytes);
    }
    let tmp = write_temp(path, bytes)?;
    let result = {
        // ReplaceFile keeps attributes and permissions; some network file
        // systems don't support it, a plain rename still replaces in one step.
        crate::win32::replace_file(path, &tmp).or_else(|_| std::fs::rename(&tmp, path))
    };
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

fn write_temp(path: &Path, bytes: &[u8]) -> std::io::Result<PathBuf> {
    for _ in 0..100 {
        let tmp = temp_path(path);
        let mut f = match std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        };
        let result = f.write_all(bytes).and_then(|()| f.sync_all());
        if let Err(e) = result {
            drop(f);
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        return Ok(tmp);
    }
    Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "could not create a unique save file"))
}

/// A unique temporary path next to the file, so replacing is one step.
fn temp_path(path: &Path) -> PathBuf {
    let serial = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    path.with_file_name(format!(".nebula-save-{}-{serial}", std::process::id()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn replaces_contents_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        super::write(&path, b"neu").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"neu");
        std::fs::write(&path, b"alter, laengerer Inhalt").unwrap();
        super::write(&path, b"kurz").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"kurz");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    /// Check on a real drive, e.g. a network share:
    /// `NEBULA_SAFE_WRITE_DIR=A:\tmp cargo test safe_write_in_folder -- --ignored`
    #[test]
    #[ignore]
    fn safe_write_in_folder() {
        let dir = std::path::PathBuf::from(std::env::var("NEBULA_SAFE_WRITE_DIR").unwrap());
        let path = dir.join("nebula-safe-write-test.txt");
        std::fs::write(&path, b"alt").unwrap();
        super::write(&path, b"neu").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"neu");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn existing_save_temp_name_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, b"alt").unwrap();
        let neighbor = dir.path().join(".a.txt.nebula-save");
        std::fs::write(&neighbor, b"keep").unwrap();
        super::write(&path, b"neu").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"neu");
        assert_eq!(std::fs::read(&neighbor).unwrap(), b"keep");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    #[test]
    fn failed_replace_keeps_the_existing_directory_contents() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::create_dir(&path).unwrap();
        let old = path.join("old.txt");
        std::fs::write(&old, b"alt").unwrap();
        assert!(super::write(&path, b"neu").is_err());
        assert_eq!(std::fs::read(&old).unwrap(), b"alt");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

}
