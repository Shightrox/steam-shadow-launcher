use crate::error::{AppError, AppResult};
use serde::Serialize;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum JunctionHealth {
    Healthy,
    Missing,
    Stale { actual: PathBuf },
    NotAJunction,
}

fn is_reparse_point(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    match std::fs::symlink_metadata(path) {
        Ok(md) => md.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0,
        Err(_) => false,
    }
}

pub fn is_junction(path: &Path) -> bool {
    is_reparse_point(path)
}

fn read_junction_target(path: &Path) -> Option<PathBuf> {
    // Use std::fs::read_link which on Windows works for both symlinks and junctions
    std::fs::read_link(path).ok().map(|p| {
        // Strip the \??\ prefix that read_link sometimes returns for junctions
        let s = p.to_string_lossy().to_string();
        let stripped = s
            .strip_prefix(r"\??\")
            .or_else(|| s.strip_prefix(r"\\?\"))
            .unwrap_or(&s);
        PathBuf::from(stripped)
    })
}

pub fn create(link: &Path, target: &Path) -> AppResult<()> {
    if link.exists() || is_reparse_point(link) {
        return Err(AppError::Junction(format!(
            "link path already exists: {}",
            link.display()
        )));
    }
    if !target.exists() {
        return Err(AppError::Junction(format!(
            "target does not exist: {}",
            target.display()
        )));
    }
    if let Some(parent) = link.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let status = Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|e| AppError::Junction(format!("mklink spawn failed: {e}")))?;
    if !status.success() {
        return Err(AppError::Junction(format!(
            "mklink /J failed with code {:?}",
            status.code()
        )));
    }
    Ok(())
}

pub fn verify(link: &Path, expected: &Path) -> AppResult<JunctionHealth> {
    if !link.exists() && !is_reparse_point(link) {
        return Ok(JunctionHealth::Missing);
    }
    if !is_reparse_point(link) {
        return Ok(JunctionHealth::NotAJunction);
    }
    let actual = match read_junction_target(link) {
        Some(p) => p,
        None => return Ok(JunctionHealth::NotAJunction),
    };
    let norm_a = std::fs::canonicalize(&actual).unwrap_or(actual.clone());
    let norm_e = std::fs::canonicalize(expected).unwrap_or(expected.to_path_buf());
    if norm_a == norm_e {
        Ok(JunctionHealth::Healthy)
    } else {
        Ok(JunctionHealth::Stale { actual })
    }
}

/// Remove a junction without deleting the target's contents.
/// On Windows `RemoveDirectoryW` (i.e. `std::fs::remove_dir`) on a junction
/// only removes the reparse point.
pub fn remove(link: &Path) -> AppResult<()> {
    if !link.exists() && !is_reparse_point(link) {
        return Ok(());
    }
    if is_reparse_point(link) {
        std::fs::remove_dir(link)
            .map_err(|e| AppError::Junction(format!("remove junction failed: {e}")))?;
    } else {
        return Err(AppError::Junction(format!(
            "refusing to remove non-junction at {}",
            link.display()
        )));
    }
    Ok(())
}

pub fn repair(link: &Path, target: &Path) -> AppResult<()> {
    if is_reparse_point(link) {
        remove(link)?;
    }
    create(link, target)
}

#[derive(Debug, Default, Serialize)]
pub struct CleanupReport {
    pub repaired: Vec<String>,
    pub removed: Vec<String>,
    pub errors: Vec<String>,
}

pub fn cleanup_stale(workspace: &Path, main_steamapps: &Path) -> AppResult<CleanupReport> {
    let mut report = CleanupReport::default();
    let accounts = workspace.join("accounts");
    if !accounts.exists() {
        return Ok(report);
    }
    for entry in std::fs::read_dir(&accounts)? {
        let entry = entry?;
        let p = entry.path();
        if !p.is_dir() {
            continue;
        }
        let link = p.join("steamapps");
        match verify(&link, main_steamapps) {
            Ok(JunctionHealth::Healthy) => {}
            Ok(JunctionHealth::Missing) => {
                if let Err(e) = create(&link, main_steamapps) {
                    report.errors.push(format!("{}: {e}", link.display()));
                } else {
                    report.repaired.push(link.display().to_string());
                }
            }
            Ok(JunctionHealth::Stale { .. }) => {
                if let Err(e) = repair(&link, main_steamapps) {
                    report.errors.push(format!("{}: {e}", link.display()));
                } else {
                    report.repaired.push(link.display().to_string());
                }
            }
            Ok(JunctionHealth::NotAJunction) => {
                report.errors.push(format!(
                    "{} exists but is not a junction; manual fix required",
                    link.display()
                ));
            }
            Err(e) => report.errors.push(format!("{}: {e}", link.display())),
        }
    }
    Ok(report)
}
