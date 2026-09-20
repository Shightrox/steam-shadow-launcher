//! One file per restore point: VDF and registry always belong to the same switch.
use crate::{
    error::{AppError, AppResult},
    storage,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

static LOCK: Mutex<()> = Mutex::new(());
const KEEP: usize = 10;
#[derive(Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub version: u32,
    pub created_at: u64,
    pub auto_login_user: Option<String>,
    pub loginusers: Option<String>,
}
#[derive(Default, Serialize)]
pub struct Cleanup {
    pub removed: usize,
    pub migrated: usize,
    pub retained: usize,
}
fn directory(ws: &Path) -> AppResult<PathBuf> {
    let dir = ws.join("backups");
    if storage::is_reparse(&dir) {
        return Err(AppError::Workspace("BACKUPS_UNSAFE_PATH".into()));
    }
    fs::create_dir_all(&dir)?;
    Ok(dir)
}
fn timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn legacy_stamp(name: &str, prefix: &str, suffix: &str) -> Option<u64> {
    let value = name.strip_prefix(prefix)?.strip_suffix(suffix)?;
    if value.is_empty() || !value.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}
fn snapshots(dir: &Path) -> AppResult<Vec<(PathBuf, Snapshot)>> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !entry.file_type()?.is_file()
            || storage::is_reparse(&entry.path())
            || !name.starts_with("switch-")
            || !name.ends_with(".json")
        {
            continue;
        }
        if let Ok(s) = serde_json::from_slice::<Snapshot>(&fs::read(entry.path())?) {
            if s.version == 1
                && s.loginusers
                    .as_ref()
                    .is_none_or(|v| STANDARD.decode(v).is_ok())
            {
                out.push((entry.path(), s));
            }
        }
    }
    out.sort_by_key(|(_, s)| std::cmp::Reverse(s.created_at));
    Ok(out)
}
fn persist(dir: &Path, snapshot: &Snapshot) -> AppResult<PathBuf> {
    let path = dir.join(format!(
        "switch-{}-{:016x}.json",
        snapshot.created_at,
        rand::random::<u64>()
    ));
    storage::atomic_write(&path, &serde_json::to_vec(snapshot)?)?;
    Ok(path)
}
fn maintain_inner(dir: &Path) -> AppResult<Cleanup> {
    let mut report = Cleanup::default();
    let mut entries = fs::read_dir(dir)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        if !entry.file_type()?.is_file() || storage::is_reparse(&entry.path()) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if legacy_stamp(&name, "loginusers-PATCHED-", ".vdf").is_some() {
            fs::remove_file(entry.path())?;
            report.removed += 1;
            continue;
        }
        let Some(stamp) = legacy_stamp(&name, "loginusers-", ".vdf") else {
            continue;
        };
        let registry = dir.join(format!("registry-{stamp}.json"));
        if !registry.is_file() || storage::is_reparse(&registry) {
            continue;
        }
        #[derive(Deserialize)]
        struct Legacy {
            auto_login_user: Option<String>,
        }
        let Ok(old) = serde_json::from_slice::<Legacy>(&fs::read(&registry)?) else {
            continue;
        };
        let next = Snapshot {
            version: 1,
            created_at: stamp.saturating_mul(1000),
            auto_login_user: old.auto_login_user,
            loginusers: Some(STANDARD.encode(fs::read(entry.path())?)),
        };
        let exists = snapshots(dir)?.iter().any(|(_, s)| {
            s.created_at == next.created_at
                && s.loginusers == next.loginusers
                && s.auto_login_user == next.auto_login_user
        });
        if !exists {
            persist(dir, &next)?;
        }
        // Conversion was durable before either legacy member is removed.
        fs::remove_file(entry.path())?;
        fs::remove_file(registry)?;
        report.migrated += 1;
        report.removed += 2;
    }
    let mut previous: Option<Snapshot> = None;
    for (path, snapshot) in snapshots(dir)? {
        let duplicate = previous.as_ref().is_some_and(|p| {
            p.loginusers == snapshot.loginusers && p.auto_login_user == snapshot.auto_login_user
        });
        if duplicate || report.retained >= KEEP {
            fs::remove_file(path)?;
            report.removed += 1;
        } else {
            report.retained += 1;
            previous = Some(snapshot);
        }
    }
    Ok(report)
}
pub fn maintain(ws: &Path) -> AppResult<Cleanup> {
    let _lock = LOCK.lock().unwrap();
    maintain_inner(&directory(ws)?)
}
pub fn create(
    ws: &Path,
    loginusers: Option<Vec<u8>>,
    auto_login_user: Option<String>,
) -> AppResult<PathBuf> {
    let _lock = LOCK.lock().unwrap();
    let dir = directory(ws)?;
    maintain_inner(&dir)?;
    let snapshot = Snapshot {
        version: 1,
        created_at: timestamp(),
        auto_login_user,
        loginusers: loginusers.map(|b| STANDARD.encode(b)),
    };
    if let Some((path, previous)) = snapshots(&dir)?.first() {
        if previous.loginusers == snapshot.loginusers
            && previous.auto_login_user == snapshot.auto_login_user
        {
            return Ok(path.clone());
        }
    }
    let path = persist(&dir, &snapshot)?;
    maintain_inner(&dir)?;
    Ok(path)
}
pub fn latest(ws: &Path) -> AppResult<Snapshot> {
    let _lock = LOCK.lock().unwrap();
    let dir = directory(ws)?;
    maintain_inner(&dir)?;
    snapshots(&dir)?
        .into_iter()
        .next()
        .map(|(_, s)| s)
        .ok_or_else(|| AppError::NotFound("NO_RESTORE_POINT".into()))
}
pub fn restore_vdf(snapshot: &Snapshot, path: &Path) -> AppResult<()> {
    match &snapshot.loginusers {
        Some(data) => storage::atomic_write(
            path,
            &STANDARD
                .decode(data)
                .map_err(|_| AppError::Config("INVALID_RESTORE_POINT".into()))?,
        ),
        None if path.exists() => {
            fs::remove_file(path)?;
            Ok(())
        }
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn migration_ignores_patched_and_retention_keeps_consistent_pairs() {
        let ws = std::env::temp_dir().join(format!("shadow-backups-{}", rand::random::<u64>()));
        let dir = directory(&ws).unwrap();
        for i in 1..=15 {
            fs::write(
                dir.join(format!("loginusers-{i}.vdf")),
                format!("original-{i}"),
            )
            .unwrap();
            fs::write(
                dir.join(format!("registry-{i}.json")),
                format!(r#"{{"auto_login_user":"user-{i}"}}"#),
            )
            .unwrap();
            fs::write(dir.join(format!("loginusers-PATCHED-{i}.vdf")), "PATCHED").unwrap();
        }
        fs::write(dir.join("personal-backup.txt"), "untouched").unwrap();
        let r = maintain(&ws).unwrap();
        assert_eq!(r.retained, KEEP);
        assert_eq!(r.migrated, 15);
        let last = latest(&ws).unwrap();
        assert_eq!(last.auto_login_user.as_deref(), Some("user-15"));
        assert_eq!(
            STANDARD.decode(last.loginusers.unwrap()).unwrap(),
            b"original-15"
        );
        assert!(dir.join("personal-backup.txt").exists());
        let a = create(&ws, Some(b"same".to_vec()), Some("same".into())).unwrap();
        let b = create(&ws, Some(b"same".to_vec()), Some("same".into())).unwrap();
        assert_eq!(a, b);
        assert_eq!(snapshots(&dir).unwrap().len(), KEEP);
        assert!(fs::canonicalize(&ws)
            .unwrap()
            .starts_with(fs::canonicalize(std::env::temp_dir()).unwrap()));
        fs::remove_dir_all(ws).unwrap();
    }
}
