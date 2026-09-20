//! Durable replacement without deleting the last good copy on a failed rename.
use crate::error::{AppError, AppResult};
use std::{
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
};

pub fn atomic_write(path: &Path, bytes: &[u8]) -> AppResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| AppError::Io("missing parent directory".into()))?;
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(".shadow-write-{:016x}.tmp", rand::random::<u64>()));
    let result = (|| -> AppResult<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

/// Resolve aliases in the existing prefix, including case on Windows. Missing
/// descendants are allowed, but parent traversal is normalized before use.
pub fn resolve(path: &Path) -> AppResult<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut normalized = PathBuf::new();
    for part in absolute.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    let mut prefix = normalized.as_path();
    let mut suffix = Vec::new();
    while !prefix.exists() {
        suffix.push(
            prefix
                .file_name()
                .ok_or_else(|| AppError::Io("invalid path".into()))?
                .to_owned(),
        );
        prefix = prefix
            .parent()
            .ok_or_else(|| AppError::Io("invalid path".into()))?;
    }
    let mut result = fs::canonicalize(prefix)?;
    for part in suffix.into_iter().rev() {
        result.push(part);
    }
    Ok(result)
}

pub fn path_key(path: &Path) -> AppResult<PathBuf> {
    Ok(PathBuf::from(
        resolve(path)?.to_string_lossy().to_lowercase(),
    ))
}

pub fn is_reparse(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    fs::symlink_metadata(path)
        .map(|m| m.file_attributes() & 0x400 != 0)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replacement_failure_preserves_original() {
        let dir = std::env::temp_dir().join(format!("shadow-atomic-{}", rand::random::<u64>()));
        fs::create_dir_all(dir.join("target")).unwrap();
        fs::write(dir.join("target/original"), b"keep").unwrap();
        assert!(atomic_write(&dir.join("target"), b"replacement").is_err());
        assert_eq!(fs::read(dir.join("target/original")).unwrap(), b"keep");
        fs::remove_file(dir.join("target/original")).unwrap();
        fs::remove_dir(dir.join("target")).unwrap();
        fs::remove_dir(dir).unwrap();
    }
}
