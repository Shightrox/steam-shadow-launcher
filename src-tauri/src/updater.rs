//! Self-update checker + applier.
//!
//! Hits the GitHub Releases API for our own repo, picks the portable `.exe`
//! asset out of the latest tag, and either reports the new version to the UI
//! or downloads & swaps the running binary on disk.
//!
//! The swap-on-Windows trick: a running `.exe` cannot overwrite itself, but
//! it CAN be renamed (NTFS lets you rename a file even while it's open as an
//! image). We rename the live exe to `*.old`, drop the freshly-downloaded
//! exe in its place, then spawn a tiny batch script that:
//!
//!   1. Waits for the current process to exit.
//!   2. Deletes the `*.old` sidecar.
//!   3. Re-launches the (now updated) main exe.
//!
//! The batch file lives in `%TEMP%` and is detached via `CREATE_NO_WINDOW |
//! DETACHED_PROCESS` so the Windows shell doesn't flash a black console
//! window in the user's face. We then ask Tauri to gracefully exit.

use crate::error::{AppError, AppResult};
use crate::http;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::PathBuf;

const REPO_RELEASES: &str =
    "https://api.github.com/repos/Shightrox/steam-shadow-launcher/releases/latest";

#[derive(Debug, Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
    size: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct GhRelease {
    tag_name: String,
    name: Option<String>,
    body: Option<String>,
    html_url: String,
    assets: Vec<GhAsset>,
}

/// What we hand back to the UI. `download_url` is `None` when there's no
/// portable .exe asset attached to the release (defensive — should never
/// happen in practice but we don't want to crash the renderer).
#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    pub current: String,
    pub latest: String,
    pub has_update: bool,
    pub asset_name: Option<String>,
    pub download_url: Option<String>,
    pub asset_size: Option<u64>,
    pub release_url: String,
    pub release_title: String,
    /// Markdown body of the release notes (truncated to keep IPC sane).
    pub notes: String,
}

/// Strip a leading `v`/`V` and split into `(major, minor, patch, ..)`. Any
/// non-numeric trailing component (e.g. `-rc1`) collapses to `0` so a real
/// release sorts after a pre-release of the same number.
fn parse_semver(tag: &str) -> Vec<u64> {
    let s = tag.trim_start_matches(['v', 'V']);
    // Drop pre-release / build metadata for ordering — we compare only the
    // numeric prefix, which is enough for our flat `vX.Y.Z` tagging scheme.
    let core = s.split(|c: char| c == '-' || c == '+').next().unwrap_or("");
    core.split('.')
        .map(|p| p.chars().take_while(|c| c.is_ascii_digit()).collect::<String>())
        .map(|p| p.parse::<u64>().unwrap_or(0))
        .collect()
}

fn is_strictly_newer(latest: &str, current: &str) -> bool {
    let a = parse_semver(latest);
    let b = parse_semver(current);
    let n = a.len().max(b.len());
    for i in 0..n {
        let ai = a.get(i).copied().unwrap_or(0);
        let bi = b.get(i).copied().unwrap_or(0);
        if ai != bi {
            return ai > bi;
        }
    }
    false
}

/// Pick the portable .exe asset out of a release. We look for a name
/// containing both "portable" and ending in ".exe" (case-insensitive).
fn pick_portable_asset(assets: &[GhAsset]) -> Option<&GhAsset> {
    assets.iter().find(|a| {
        let n = a.name.to_ascii_lowercase();
        n.ends_with(".exe") && n.contains("portable")
    })
}

pub fn check_update() -> AppResult<UpdateInfo> {
    let current = env!("CARGO_PKG_VERSION").to_string();
    let resp = http::shared()
        .get(REPO_RELEASES)
        .header("Accept", "application/vnd.github+json")
        .send()
        .map_err(|e| AppError::Other(format!("github releases: {e}")))?;
    if !resp.status().is_success() {
        return Err(AppError::Other(format!(
            "github releases HTTP {}",
            resp.status()
        )));
    }
    let rel: GhRelease = resp
        .json()
        .map_err(|e| AppError::Other(format!("parse release json: {e}")))?;
    let latest_clean = rel.tag_name.trim_start_matches(['v', 'V']).to_string();
    let has_update = is_strictly_newer(&rel.tag_name, &current);
    let asset = pick_portable_asset(&rel.assets);
    let notes = rel
        .body
        .clone()
        .unwrap_or_default()
        .chars()
        .take(4000)
        .collect();
    Ok(UpdateInfo {
        current,
        latest: latest_clean.clone(),
        has_update,
        asset_name: asset.map(|a| a.name.clone()),
        download_url: asset.map(|a| a.browser_download_url.clone()),
        asset_size: asset.and_then(|a| a.size),
        release_url: rel.html_url,
        release_title: rel.name.unwrap_or_else(|| format!("v{latest_clean}")),
        notes,
    })
}

/// Download URL → write the swap batch → exit. The batch handles the actual
/// rename + relaunch after our process is gone.
///
/// Returns once the new exe is on disk and the batch is queued. The caller
/// (Tauri command layer) then asks the AppHandle to exit so the swap can
/// proceed.
pub fn apply_update(url: &str) -> AppResult<()> {
    let current_exe = std::env::current_exe()
        .map_err(|e| AppError::Other(format!("current_exe: {e}")))?;
    let exe_dir = current_exe
        .parent()
        .ok_or_else(|| AppError::Other("current_exe has no parent".into()))?
        .to_path_buf();
    let exe_name = current_exe
        .file_name()
        .ok_or_else(|| AppError::Other("current_exe has no file name".into()))?
        .to_owned();

    // 1. Download the new build into a sibling temp file. Same-volume so the
    //    final rename is atomic.
    let new_exe: PathBuf = exe_dir.join(format!(
        "{}.new",
        exe_name.to_string_lossy()
    ));
    if new_exe.exists() {
        let _ = fs::remove_file(&new_exe);
    }
    crate::download::download_with_progress(url, &new_exe, |_done, _total| {})?;

    // 2. Stage the swap. Move live exe → *.old (we can't unlink it while the
    //    process is alive; rename works), then move new exe into the live
    //    name. After this point, the on-disk binary is already the new one
    //    and re-launching the same path executes the new build.
    let old_exe = exe_dir.join(format!("{}.old", exe_name.to_string_lossy()));
    if old_exe.exists() {
        let _ = fs::remove_file(&old_exe);
    }
    fs::rename(&current_exe, &old_exe).map_err(|e| {
        AppError::Other(format!("rename live exe → .old failed: {e}"))
    })?;
    if let Err(e) = fs::rename(&new_exe, &current_exe) {
        // Best-effort rollback so we don't strand the user with no exe.
        let _ = fs::rename(&old_exe, &current_exe);
        return Err(AppError::Other(format!(
            "promote new exe failed: {e}"
        )));
    }

    // 3. Drop a tiny .cmd that waits for our process to be gone, removes the
    //    *.old sidecar (it's an open image right now, so unlink fails until
    //    we exit), and relaunches the new exe. Detach via START "" so the
    //    cmd terminates immediately and our re-launch isn't a child of it.
    let pid = std::process::id();
    let cmd_path = std::env::temp_dir().join(format!(
        "ssl-update-{}-{}.cmd",
        pid,
        chrono_unix_seconds()
    ));
    let exe_for_relaunch = current_exe.to_string_lossy().to_string();
    let old_for_cleanup = old_exe.to_string_lossy().to_string();
    let script = format!(
        "@echo off\r\n\
         setlocal\r\n\
         rem Wait up to ~30 s for PID {pid} to disappear before swapping.\r\n\
         set /a TRIES=0\r\n\
         :wait\r\n\
         tasklist /FI \"PID eq {pid}\" 2>nul | find \"{pid}\" >nul\r\n\
         if errorlevel 1 goto gone\r\n\
         if %TRIES% GEQ 60 goto gone\r\n\
         set /a TRIES+=1\r\n\
         timeout /T 1 /NOBREAK >nul\r\n\
         goto wait\r\n\
         :gone\r\n\
         del /F /Q \"{old}\" 2>nul\r\n\
         start \"\" \"{exe}\"\r\n\
         del /F /Q \"%~f0\" 2>nul\r\n",
        pid = pid,
        old = old_for_cleanup,
        exe = exe_for_relaunch,
    );
    let mut f = fs::File::create(&cmd_path)
        .map_err(|e| AppError::Other(format!("write update.cmd: {e}")))?;
    f.write_all(script.as_bytes())
        .map_err(|e| AppError::Other(format!("write update.cmd: {e}")))?;
    drop(f);

    spawn_detached_cmd(&cmd_path)?;
    Ok(())
}

fn chrono_unix_seconds() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(windows)]
fn spawn_detached_cmd(cmd_path: &std::path::Path) -> AppResult<()> {
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    // Flags: DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW.
    // Without DETACHED_PROCESS the cmd inherits our console (which is
    // non-existent in --windows-subsystem=windows builds and would cause
    // the script to fail). CREATE_NO_WINDOW suppresses the console flash.
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    Command::new("cmd.exe")
        .args(["/C", &cmd_path.to_string_lossy()])
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| AppError::Other(format!("spawn update.cmd: {e}")))?;
    Ok(())
}

#[cfg(not(windows))]
fn spawn_detached_cmd(_cmd_path: &std::path::Path) -> AppResult<()> {
    Err(AppError::Other("self-update only supported on Windows".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_strict_compare() {
        assert!(is_strictly_newer("v0.2.1", "0.2.0"));
        assert!(is_strictly_newer("0.3.0", "0.2.99"));
        assert!(!is_strictly_newer("v0.2.0", "0.2.0"));
        assert!(!is_strictly_newer("v0.1.9", "0.2.0"));
        assert!(is_strictly_newer("v1.0.0", "0.99.99"));
    }

    #[test]
    fn semver_handles_prerelease() {
        // Pre-release suffix stripped → equal core, no upgrade.
        assert!(!is_strictly_newer("v0.2.0-rc1", "0.2.0"));
    }
}
