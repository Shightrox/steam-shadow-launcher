use crate::error::{AppError, AppResult};
use crate::steam_paths::MainSteamInfo;
use crate::steam_process;
use std::fs;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use winreg::enums::*;
use winreg::RegKey;

#[allow(dead_code)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const DETACHED_PROCESS: u32 = 0x0000_0008;
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

fn now_ts() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn backups_dir(workspace: &Path) -> PathBuf {
    workspace.join("backups")
}

pub fn backup_loginusers(workspace: &Path, main: &MainSteamInfo) -> AppResult<Option<PathBuf>> {
    let src = main.install_dir.join("config").join("loginusers.vdf");
    if !src.exists() {
        return Ok(None);
    }
    let dir = backups_dir(workspace);
    fs::create_dir_all(&dir)?;
    let ts = now_ts();
    let dst = dir.join(format!("loginusers-{ts}.vdf"));
    fs::copy(&src, &dst)?;
    Ok(Some(dst))
}

#[derive(serde::Serialize, serde::Deserialize)]
struct RegistryBackup {
    timestamp: u64,
    auto_login_user: Option<String>,
}

pub fn backup_registry(workspace: &Path) -> AppResult<PathBuf> {
    let dir = backups_dir(workspace);
    fs::create_dir_all(&dir)?;
    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(r"Software\Valve\Steam", KEY_READ)
        .ok();
    let auto = key.and_then(|k| k.get_value::<String, _>("AutoLoginUser").ok());
    let ts = now_ts();
    let backup = RegistryBackup {
        timestamp: ts,
        auto_login_user: auto,
    };
    let path = dir.join(format!("registry-{ts}.json"));
    fs::write(&path, serde_json::to_string_pretty(&backup)?)?;
    Ok(path)
}

/// Patch loginusers.vdf in place: keep ALL user blocks, but set flags so
/// the target account is `MostRecent=1, AllowAutoLogin=1, RememberPassword=1`
/// and every other account has `MostRecent=0`. This matches the technique
/// used by TcNo Account Switcher (which is known to work with current Steam).
///
/// Removing other user blocks (the previous SAM-style approach) is NOT
/// necessary and may actually break auto-login because Steam keeps companion
/// data (refresh tokens in CEF Local Storage, Accounts map in config.vdf)
/// keyed by AccountName — if the VDF entry disappears, Steam can fall back
/// to the credential prompt.
/// exist — it always shows the account picker. Stripping loginusers.vdf
/// down to a single user is the technique used by Steam Account Manager.
/// The original file is restored via revert_last() from our backup.
pub fn patch_loginusers(main: &MainSteamInfo, target_login: &str) -> AppResult<()> {
    let path = main.install_dir.join("config").join("loginusers.vdf");
    if !path.exists() {
        return Err(AppError::NotFound("loginusers.vdf not found".into()));
    }
    let bytes = fs::read(&path)?;
    let txt = String::from_utf8_lossy(&bytes).to_string();
    let (patched, target_seen) = isolate_target_account_checked(&txt, target_login);
    if !target_seen {
        return Err(AppError::NotFound(format!(
            "TARGET_NOT_IN_VDF: '{}' not found in loginusers.vdf — log in to that account once via the main Steam first",
            target_login
        )));
    }
    let tmp = path.with_extension("vdf.sslnew");
    fs::write(&tmp, patched.as_bytes())?;
    fs::rename(&tmp, &path)?;
    Ok(())
}

/// Inspect current state and log it. Helps debug failed auto-login.
fn diagnose_state(main: &MainSteamInfo, target_login: &str) -> AppResult<()> {
    let cfg_dir = main.install_dir.join("config");
    let lu = cfg_dir.join("loginusers.vdf");
    if let Ok(b) = fs::read(&lu) {
        let txt = String::from_utf8_lossy(&b);
        let mut current = String::new();
        let mut found_target = false;
        let mut accounts: Vec<(String, bool, bool, bool)> = Vec::new(); // name, mostrecent, autologin, remember
        let mut mr = false;
        let mut al = false;
        let mut rp = false;
        for line in txt.lines() {
            let t = line.trim();
            if let Some(rest) = t.strip_prefix("\"AccountName\"") {
                let parts: Vec<&str> = rest.split('"').collect();
                if parts.len() >= 2 {
                    if !current.is_empty() {
                        accounts.push((current.clone(), mr, al, rp));
                    }
                    current = parts[1].to_string();
                    mr = false;
                    al = false;
                    rp = false;
                    if current.eq_ignore_ascii_case(target_login) {
                        found_target = true;
                    }
                }
            } else if t.starts_with("\"MostRecent\"") && t.contains("\"1\"") {
                mr = true;
            } else if t.starts_with("\"AllowAutoLogin\"") && t.contains("\"1\"") {
                al = true;
            } else if t.starts_with("\"RememberPassword\"") && t.contains("\"1\"") {
                rp = true;
            }
        }
        if !current.is_empty() {
            accounts.push((current, mr, al, rp));
        }
        tracing::info!(
            "diagnose: loginusers.vdf has {} accounts, target '{}' present={}",
            accounts.len(),
            target_login,
            found_target
        );
        for (n, mr, al, rp) in &accounts {
            tracing::info!(
                "  - {} MostRecent={} AllowAutoLogin={} RememberPassword={}",
                n, mr, al, rp
            );
        }
    } else {
        tracing::warn!("diagnose: loginusers.vdf unreadable");
    }
    // ssfn tokens are per-account in install root; their presence is the
    // single biggest indicator that a token-based auto-login is even possible.
    let mut ssfn = Vec::new();
    if let Ok(rd) = fs::read_dir(&main.install_dir) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if n.starts_with("ssfn") {
                ssfn.push(n);
            }
        }
    }
    tracing::info!("diagnose: ssfn files in install dir: {:?}", ssfn);
    let cfg = cfg_dir.join("config.vdf");
    if let Ok(b) = fs::read(&cfg) {
        let txt = String::from_utf8_lossy(&b);
        let has_cc = txt.contains("ConnectCache");
        tracing::info!("diagnose: config.vdf size={} has ConnectCache={}", b.len(), has_cc);
    }
    let _ = target_login;
    Ok(())
}

/// Re-flag loginusers.vdf so `target_login` becomes the active, auto-login
/// account. All other blocks are preserved but forced to MostRecent=0.
fn isolate_target_account(txt: &str, target_login: &str) -> String {
    isolate_target_account_checked(txt, target_login).0
}

fn isolate_target_account_checked(txt: &str, target_login: &str) -> (String, bool) {
    // Pass: walk lines, track whether we're inside a user block, capture
    // AccountName for each block, then rewrite flag lines for matching /
    // non-matching blocks. We also ensure required flags exist in the
    // matching block (inject them before the closing brace if missing).
    //
    // Format conventions mirror what Steam itself writes:
    //   "users"
    //   {
    //       "<steamid>"
    //       {
    //           "AccountName"   "foo"
    //           "PersonaName"   ...
    //           "RememberPassword"   "1"
    //           "WantsOfflineMode"   "0"
    //           "SkipOfflineModeWarning"   "0"
    //           "AllowAutoLogin"    "1"
    //           "MostRecent"    "0"
    //           "Timestamp"    "..."
    //       }
    //       ...
    //   }
    //
    // Line endings: Steam writes LF on Linux/macOS and CRLF on Windows.
    // We detect and preserve the dominant line ending of the input.

    let newline = if txt.contains("\r\n") { "\r\n" } else { "\n" };
    let lines: Vec<&str> = txt.split_inclusive('\n').collect();

    // First pass: find AccountName for each user block so we know which
    // block matches the target. A user block starts when we see "<steamid>"
    // at depth==1 inside the "users" object, followed by { at depth 2.
    // We track depth roughly.
    #[derive(Default, Clone)]
    struct Block {
        start: usize,         // line index of steamid line
        open: usize,          // line index of opening brace
        close: usize,         // line index of closing brace
        account_name: String,
    }
    let mut blocks: Vec<Block> = Vec::new();
    let mut in_users = false;
    let mut depth = 0usize;
    let mut cur: Option<Block> = None;
    let mut pending_steamid: Option<usize> = None;
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim();
        if !in_users {
            if t.eq_ignore_ascii_case("\"users\"") {
                in_users = true;
            }
            continue;
        }
        if t == "{" {
            depth += 1;
            if depth == 2 {
                if let (Some(sid_line), None) = (pending_steamid.take(), cur.as_ref()) {
                    cur = Some(Block {
                        start: sid_line,
                        open: i,
                        close: 0,
                        account_name: String::new(),
                    });
                }
            }
            continue;
        }
        if t == "}" {
            if depth == 2 {
                if let Some(mut b) = cur.take() {
                    b.close = i;
                    blocks.push(b);
                }
            }
            if depth == 0 {
                // closing of outermost {
            } else {
                depth -= 1;
            }
            continue;
        }
        if depth == 1 {
            // Expect a "<steamid>" token
            if t.starts_with('"') && t.ends_with('"') && t.len() > 2 {
                pending_steamid = Some(i);
            }
        } else if depth == 2 {
            if let Some(b) = cur.as_mut() {
                if let Some(rest) = t.strip_prefix("\"AccountName\"") {
                    let parts: Vec<&str> = rest.split('"').collect();
                    if parts.len() >= 2 {
                        b.account_name = parts[1].to_string();
                    }
                }
            }
        }
    }

    // Figure out the indent style from one of the flag lines we find.
    let indent = detect_indent(&lines);

    // Build output line-by-line, rewriting flag lines within each block.
    let mut current_block: Option<&Block> = None;
    let mut out = String::with_capacity(txt.len() + 256);
    let mut target_seen = false;
    for (i, line) in lines.iter().enumerate() {
        // Enter / exit block tracking
        if let Some(b) = blocks.iter().find(|b| b.open == i) {
            current_block = Some(b);
            out.push_str(line);
            continue;
        }
        if let Some(b) = current_block {
            if i == b.close {
                // Inject any missing required flags just before the '}' line.
                let is_target = b.account_name.eq_ignore_ascii_case(target_login);
                let (present_mr, present_al, present_rp, present_wom, present_som) =
                    scan_flags(&lines[b.open + 1..b.close]);
                let mr_val = if is_target { "1" } else { "0" };
                if !present_mr {
                    push_kv(&mut out, &indent, "MostRecent", mr_val, newline);
                }
                if is_target {
                    if !present_al {
                        push_kv(&mut out, &indent, "AllowAutoLogin", "1", newline);
                    }
                    if !present_rp {
                        push_kv(&mut out, &indent, "RememberPassword", "1", newline);
                    }
                    if !present_wom {
                        push_kv(&mut out, &indent, "WantsOfflineMode", "0", newline);
                    }
                    if !present_som {
                        push_kv(&mut out, &indent, "SkipOfflineModeWarning", "0", newline);
                    }
                    target_seen = true;
                }
                out.push_str(line);
                current_block = None;
                continue;
            }
            // Inside a block — rewrite flag lines.
            let t = line.trim_start();
            let is_target = b.account_name.eq_ignore_ascii_case(target_login);
            let rewritten = if t.starts_with("\"MostRecent\"") {
                Some(make_kv(&indent, "MostRecent", if is_target { "1" } else { "0" }, newline))
            } else if is_target && t.starts_with("\"AllowAutoLogin\"") {
                Some(make_kv(&indent, "AllowAutoLogin", "1", newline))
            } else if is_target && t.starts_with("\"RememberPassword\"") {
                Some(make_kv(&indent, "RememberPassword", "1", newline))
            } else if is_target && t.starts_with("\"WantsOfflineMode\"") {
                Some(make_kv(&indent, "WantsOfflineMode", "0", newline))
            } else if is_target && t.starts_with("\"SkipOfflineModeWarning\"") {
                Some(make_kv(&indent, "SkipOfflineModeWarning", "0", newline))
            } else {
                None
            };
            if let Some(r) = rewritten {
                out.push_str(&r);
            } else {
                out.push_str(line);
            }
            continue;
        }
        out.push_str(line);
    }

    if !target_seen {
        tracing::warn!(
            "isolate_target_account: target '{}' not in loginusers.vdf",
            target_login
        );
    }

    (out, target_seen)
}

fn detect_indent(lines: &[&str]) -> String {
    // Look for a flag line like `\t\t"RememberPassword" ...` to pick
    // up the depth-2 indentation. Default to 2 tabs.
    for l in lines {
        if l.contains("\"AccountName\"") || l.contains("\"MostRecent\"") {
            let end = l.len() - l.trim_start().len();
            return l[..end].to_string();
        }
    }
    "\t\t".to_string()
}

fn make_kv(indent: &str, k: &str, v: &str, newline: &str) -> String {
    format!("{indent}\"{k}\"\t\t\"{v}\"{newline}")
}

fn push_kv(out: &mut String, indent: &str, k: &str, v: &str, newline: &str) {
    out.push_str(&make_kv(indent, k, v, newline));
}

/// Returns (MostRecent, AllowAutoLogin, RememberPassword, WantsOfflineMode, SkipOfflineModeWarning).
fn scan_flags(block_lines: &[&str]) -> (bool, bool, bool, bool, bool) {
    let mut mr = false;
    let mut al = false;
    let mut rp = false;
    let mut wom = false;
    let mut som = false;
    for l in block_lines {
        let t = l.trim_start();
        if t.starts_with("\"MostRecent\"") {
            mr = true;
        } else if t.starts_with("\"AllowAutoLogin\"") {
            al = true;
        } else if t.starts_with("\"RememberPassword\"") {
            rp = true;
        } else if t.starts_with("\"WantsOfflineMode\"") {
            wom = true;
        } else if t.starts_with("\"SkipOfflineModeWarning\"") {
            som = true;
        }
    }
    (mr, al, rp, wom, som)
}

pub fn write_autologin(login: &str) -> AppResult<()> {
    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(r"Software\Valve\Steam", KEY_WRITE | KEY_READ)
        .map_err(|e| AppError::Registry(format!("open Valve\\Steam for write: {e}")))?;
    key.set_value("AutoLoginUser", &login.to_string())
        .map_err(|e| AppError::Registry(format!("set AutoLoginUser: {e}")))?;
    Ok(())
}

pub fn restore_autologin(prev: Option<String>) -> AppResult<()> {
    if let Some(p) = prev {
        write_autologin(&p)?;
    }
    Ok(())
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SwitchResult {
    pub vdf_backup: Option<PathBuf>,
    pub reg_backup: PathBuf,
    pub steam_pid: u32,
    pub previous_autologin: Option<String>,
}

pub fn switch_to(
    workspace: &Path,
    main: &MainSteamInfo,
    target_login: &str,
) -> AppResult<SwitchResult> {
    let prev_autologin = main.autologin_user.clone();
    let vdf_backup = backup_loginusers(workspace, main)?;
    let reg_backup = backup_registry(workspace)?;
    steam_process::graceful_shutdown(&main.steam_exe)?;
    // Wait for steam to fully release file handles before patching.
    std::thread::sleep(std::time::Duration::from_millis(700));
    // Diagnostic: log accounts present in loginusers.vdf with their flags,
    // plus presence of ssfn* and ConnectCache in config.vdf. This is the
    // information we need to understand why auto-login fails.
    diagnose_state(main, target_login).ok();
    patch_loginusers(main, target_login)?;
    // Save the patched VDF copy for inspection.
    if let Ok(b) = fs::read(main.install_dir.join("config").join("loginusers.vdf")) {
        let dir = backups_dir(workspace);
        let ts = now_ts();
        let _ = fs::write(dir.join(format!("loginusers-PATCHED-{ts}.vdf")), &b);
    }
    write_autologin(target_login)?;
    // IMPORTANT: do NOT pass `-login <user>` here. Modern Steam uses two
    // distinct login paths:
    //   (a) auto-login via stored token (ssfn + ConnectCache in config.vdf)
    //   (b) credential login (`-login user pass`)
    // Passing `-login user` *without* a password forces (b) and Steam shows
    // the password prompt even if a valid token exists for that account.
    // The token path triggers automatically when:
    //   * `AutoLoginUser` matches the user (we just wrote it)
    //   * MostRecent=1 in loginusers.vdf for that user (our patch)
    //   * RememberPassword=1 in loginusers.vdf (preserved by patch)
    // This is exactly how Steam Account Manager (SAM) works.
    // NOTE: do NOT pass `-silent` here. Steam interprets `-silent` literally
    // as "start minimised to tray, no main window", which is exactly the
    // behaviour we don't want — user reported having to dig the window out
    // of the tray every time. Plain `steam.exe` with no args opens the
    // library window after auto-login completes.
    let child = Command::new(&main.steam_exe)
        .current_dir(&main.install_dir)
        .creation_flags(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS)
        .spawn()
        .map_err(|e| AppError::Process(format!("spawn steam.exe: {e}")))?;
    tracing::info!("switch_to({}): spawned steam pid={}", target_login, child.id());
    Ok(SwitchResult {
        vdf_backup,
        reg_backup,
        steam_pid: child.id(),
        previous_autologin: prev_autologin,
    })
}

pub fn revert_last(workspace: &Path, main: &MainSteamInfo) -> AppResult<()> {
    let dir = backups_dir(workspace);
    if !dir.exists() {
        return Err(AppError::NotFound("no backups".into()));
    }
    // pick newest vdf backup
    let mut entries: Vec<_> = fs::read_dir(&dir)?
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("loginusers-")
        })
        .collect();
    entries.sort_by_key(|e| std::cmp::Reverse(e.metadata().and_then(|m| m.modified()).ok()));
    if let Some(latest) = entries.first() {
        let dst = main.install_dir.join("config").join("loginusers.vdf");
        steam_process::graceful_shutdown(&main.steam_exe)?;
        fs::copy(latest.path(), &dst)?;
    }
    // restore registry
    let mut reg_entries: Vec<_> = fs::read_dir(&dir)?
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("registry-"))
        .collect();
    reg_entries.sort_by_key(|e| std::cmp::Reverse(e.metadata().and_then(|m| m.modified()).ok()));
    if let Some(latest) = reg_entries.first() {
        let txt = fs::read_to_string(latest.path())?;
        let bk: RegistryBackup = serde_json::from_str(&txt)?;
        if let Some(u) = bk.auto_login_user {
            write_autologin(&u)?;
        }
    }
    let _ = restore_autologin;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isolates_target_account() {
        let sample = "\"users\"\r\n{\r\n\t\"76561198000000001\"\r\n\t{\r\n\t\t\"AccountName\"\t\t\"alice\"\r\n\t\t\"PersonaName\"\t\t\"Alice\"\r\n\t\t\"RememberPassword\"\t\t\"1\"\r\n\t\t\"MostRecent\"\t\t\"1\"\r\n\t\t\"AllowAutoLogin\"\t\t\"1\"\r\n\t}\r\n\t\"76561198000000002\"\r\n\t{\r\n\t\t\"AccountName\"\t\t\"bob\"\r\n\t\t\"PersonaName\"\t\t\"Bob\"\r\n\t\t\"RememberPassword\"\t\t\"1\"\r\n\t\t\"MostRecent\"\t\t\"0\"\r\n\t\t\"AllowAutoLogin\"\t\t\"0\"\r\n\t}\r\n}\r\n";
        let out = isolate_target_account(sample, "bob");
        // Both blocks preserved.
        assert!(out.contains("\"alice\""), "alice block must remain");
        assert!(out.contains("\"bob\""), "bob block must remain");
        // alice: MostRecent flipped to 0
        let alice_idx = out.find("\"alice\"").unwrap();
        let bob_idx = out.find("\"bob\"").unwrap();
        let alice_seg = &out[alice_idx..bob_idx];
        let bob_seg = &out[bob_idx..];
        assert!(
            alice_seg.contains("\"MostRecent\"\t\t\"0\""),
            "alice MostRecent must be 0, got: {}",
            alice_seg
        );
        // bob: MostRecent=1, AllowAutoLogin=1, RememberPassword=1
        assert!(
            bob_seg.contains("\"MostRecent\"\t\t\"1\""),
            "bob MostRecent must be 1"
        );
        assert!(
            bob_seg.contains("\"AllowAutoLogin\"\t\t\"1\""),
            "bob AllowAutoLogin must be 1"
        );
        assert!(
            bob_seg.contains("\"RememberPassword\"\t\t\"1\""),
            "bob RememberPassword must be 1"
        );
    }
}
