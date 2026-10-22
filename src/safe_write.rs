//! Saving without risking the file: the new contents go into a temporary
//! file next to it first, which then replaces the original in one step. If
//! writing breaks off (network drive gone, disk full), the old file stays
//! complete.

use std::io::Write;
use std::path::{Path, PathBuf};

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
    let tmp = temp_path(path);
    let result = write_temp(&tmp, bytes).and_then(|()| {
        // ReplaceFile keeps attributes and permissions; some network file
        // systems don't support it, a plain rename still replaces in one step.
        crate::win32::replace_file(path, &tmp).or_else(|_| std::fs::rename(&tmp, path))
    });
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

fn write_temp(tmp: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut f = std::fs::File::create(tmp)?;
    f.write_all(bytes)?;
    // On disk before the old file goes away.
    f.sync_all()
}

/// `.name.nebula-save` next to the file (same folder, so replacing is one step).
fn temp_path(path: &Path) -> PathBuf {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    path.with_file_name(format!(".{name}.nebula-save"))
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
        assert!(!super::temp_path(&path).exists());
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn failed_write_keeps_the_old_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, b"alt").unwrap();
        // The temporary file can't be created: a folder is in the way.
        std::fs::create_dir(super::temp_path(&path)).unwrap();
        assert!(super::write(&path, b"neu").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"alt");
    }
}
