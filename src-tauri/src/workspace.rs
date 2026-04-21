use crate::error::{AppError, AppResult};
use crate::junction;
use crate::steam_paths::MainSteamInfo;
use crate::vdf;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ShadowMeta {
    #[serde(default, rename = "displayName")]
    pub display_name: Option<String>,
    #[serde(default, rename = "createdAt")]
    pub created_at: Option<String>,
    #[serde(default, rename = "lastLaunchAt")]
    pub last_launch_at: Option<String>,
    #[serde(default, rename = "steamId")]
    pub steam_id: Option<String>,
    #[serde(default, rename = "avatarFile")]
    pub avatar_file: Option<String>, // relative filename inside account dir
    #[serde(default)]
    pub favorite: bool,
    #[serde(default, rename = "launchCount")]
    pub launch_count: u64,
    // ── P11: Authenticator metadata ──────────────────────────────────────
    /// True iff a maFile is stored under `<account>/auth/maFile.json`.
    #[serde(default, rename = "hasAuthenticator")]
    pub has_authenticator: bool,
    /// SDA `account_name` field — the Steam login the maFile is bound to.
    /// May differ from the shadow `login` (e.g. case differences) but is
    /// usually identical.
    #[serde(default, rename = "authenticatorAccountName")]
    pub authenticator_account_name: Option<String>,
    /// ISO unix-seconds timestamp.
    #[serde(default, rename = "authenticatorImportedAt")]
    pub authenticator_imported_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Account {
    pub login: String,
    #[serde(rename = "displayName")]
    pub display_name: Option<String>,
    pub path: PathBuf,
    #[serde(rename = "lastLaunchAt")]
    pub last_launch_at: Option<String>,
    #[serde(rename = "steamId")]
    pub steam_id: Option<String>,
    #[serde(rename = "avatarPath")]
    pub avatar_path: Option<PathBuf>,
    pub favorite: bool,
    #[serde(rename = "launchCount")]
    pub launch_count: u64,
    #[serde(rename = "hasAuthenticator")]
    pub has_authenticator: bool,
    #[serde(rename = "authenticatorImportedAt")]
    pub authenticator_imported_at: Option<String>,
}

fn now_iso() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{}", secs)
}

fn validate_login(login: &str) -> AppResult<()> {
    if login.is_empty() || login.len() > 64 {
        return Err(AppError::Workspace("login length 1..64".into()));
    }
    if !login
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        return Err(AppError::Workspace(
            "login must be alphanumeric (with _ - .)".into(),
        ));
    }
    Ok(())
}

pub fn validate_workspace(path: &Path, main_steam: &Path) -> AppResult<()> {
    if path.as_os_str().is_empty() {
        return Err(AppError::Workspace("workspace path is empty".into()));
    }
    let abs = path.to_path_buf();
    let main = main_steam.to_path_buf();
    if abs.starts_with(&main) || main.starts_with(&abs) {
        return Err(AppError::Workspace(
            "workspace must not overlap with main Steam install".into(),
        ));
    }
    fs::create_dir_all(&abs)?;
    let probe = abs.join(".sslwrite");
    fs::write(&probe, b"ok")?;
    fs::remove_file(&probe).ok();
    Ok(())
}

pub fn accounts_dir(workspace: &Path) -> PathBuf {
    workspace.join("accounts")
}

pub fn account_dir(workspace: &Path, login: &str) -> PathBuf {
    accounts_dir(workspace).join(login)
}

pub fn ensure_account_dirs(workspace: &Path, login: &str) -> AppResult<PathBuf> {
    validate_login(login)?;
    let dir = account_dir(workspace, login);
    fs::create_dir_all(dir.join("config"))?;
    fs::create_dir_all(dir.join("logs"))?;
    fs::create_dir_all(dir.join("userdata"))?;
    Ok(dir)
}

fn read_meta(dir: &Path) -> ShadowMeta {
    let p = dir.join("shadow.json");
    if let Ok(txt) = fs::read_to_string(&p) {
        if let Ok(m) = serde_json::from_str::<ShadowMeta>(&txt) {
            return m;
        }
    }
    ShadowMeta::default()
}

fn write_meta(dir: &Path, meta: &ShadowMeta) -> AppResult<()> {
    let txt = serde_json::to_string_pretty(meta)?;
    fs::write(dir.join("shadow.json"), txt)?;
    Ok(())
}

/// Try to find a SteamID64 in main loginusers.vdf for the given account name.
fn lookup_steamid_for(login: &str, main: &MainSteamInfo) -> Option<String> {
    let path = main.install_dir.join("config").join("loginusers.vdf");
    let bytes = fs::read(&path).ok()?;
    let txt = String::from_utf8_lossy(&bytes).to_string();
    // Walk the file as text: blocks look like `"<steamid>" { ... "AccountName" "..." ... }`.
    let lines: Vec<&str> = txt.lines().collect();
    let mut current_sid: Option<String> = None;
    for l in &lines {
        let t = l.trim();
        if t.starts_with('"') && t.ends_with('"') && t.len() > 2 && !t.contains("\"\t") {
            // Likely a "<steamid>" header line. Steam writes 64-bit ints.
            let inner = &t[1..t.len() - 1];
            if inner.chars().all(|c| c.is_ascii_digit()) && inner.len() >= 16 {
                current_sid = Some(inner.to_string());
            }
        } else if let Some(rest) = t.strip_prefix("\"AccountName\"") {
            let parts: Vec<&str> = rest.split('"').collect();
            if parts.len() >= 2 && parts[1].eq_ignore_ascii_case(login) {
                if let Some(sid) = current_sid.clone() {
                    return Some(sid);
                }
            }
        }
    }
    None
}

/// Copy `<MainSteam>/config/avatarcache/<steamid>.png` into the account
/// directory. Returns the relative file name on success.
fn copy_avatar(account_dir: &Path, steam_id: &str, main: &MainSteamInfo) -> Option<String> {
    let src = main
        .install_dir
        .join("config")
        .join("avatarcache")
        .join(format!("{steam_id}.png"));
    if !src.exists() {
        return None;
    }
    let dst = account_dir.join("avatar.png");
    if let Err(e) = fs::copy(&src, &dst) {
        tracing::warn!("copy_avatar({}): {}", src.display(), e);
        return None;
    }
    Some("avatar.png".to_string())
}

/// Lazy backfill: if meta is missing steam_id / avatar_file, try to populate
/// them now from the host Steam install. Persists to shadow.json on change.
/// Best-effort — failures are logged but never abort.
fn backfill_meta(dir: &Path, login: &str, meta: &mut ShadowMeta) {
    let main = match crate::steam_paths::detect(None) {
        Ok(m) => m,
        Err(_) => return, // No host Steam — nothing to backfill from.
    };
    let mut changed = false;
    if meta.steam_id.is_none() {
        if let Some(sid) = lookup_steamid_for(login, &main) {
            meta.steam_id = Some(sid);
            changed = true;
        }
    }
    if meta.avatar_file.is_none() {
        if let Some(sid) = meta.steam_id.as_deref() {
            if let Some(rel) = copy_avatar(dir, sid, &main) {
                meta.avatar_file = Some(rel);
                changed = true;
            }
        }
    }
    if changed {
        let _ = write_meta(dir, meta);
    }
}

fn meta_to_account(login: String, dir: PathBuf, meta: ShadowMeta) -> Account {
    let avatar_path = meta
        .avatar_file
        .as_ref()
        .map(|f| dir.join(f))
        .filter(|p| p.exists());
    Account {
        login,
        display_name: meta.display_name,
        last_launch_at: meta.last_launch_at,
        steam_id: meta.steam_id,
        avatar_path,
        favorite: meta.favorite,
        launch_count: meta.launch_count,
        has_authenticator: meta.has_authenticator,
        authenticator_imported_at: meta.authenticator_imported_at,
        path: dir,
    }
}

pub fn list_accounts(workspace: &Path) -> AppResult<Vec<Account>> {
    let dir = accounts_dir(workspace);
    let mut out = Vec::new();
    if !dir.exists() {
        return Ok(out);
    }
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let p = entry.path();
        if !p.is_dir() {
            continue;
        }
        let login = entry.file_name().to_string_lossy().to_string();
        let mut meta = read_meta(&p);
        // P9.1 lazy backfill: try to enrich older accounts in place.
        if meta.steam_id.is_none() || meta.avatar_file.is_none() {
            backfill_meta(&p, &login, &mut meta);
        }
        out.push(meta_to_account(login, p, meta));
    }
    // Favorites first, then by last_launch_at desc, then by login.
    out.sort_by(|a, b| {
        b.favorite
            .cmp(&a.favorite)
            .then_with(|| b.last_launch_at.cmp(&a.last_launch_at))
            .then_with(|| a.login.cmp(&b.login))
    });
    Ok(out)
}

pub fn add_account(
    workspace: &Path,
    main_steamapps: &Path,
    login: &str,
    display: Option<String>,
) -> AppResult<Account> {
    validate_login(login)?;
    let dir = ensure_account_dirs(workspace, login)?;
    let link = dir.join("steamapps");
    match junction::verify(&link, main_steamapps)? {
        junction::JunctionHealth::Healthy => {}
        junction::JunctionHealth::Missing => junction::create(&link, main_steamapps)?,
        junction::JunctionHealth::Stale { .. } | junction::JunctionHealth::NotAJunction => {
            junction::repair(&link, main_steamapps)?;
        }
    }
    let mut meta = ShadowMeta {
        display_name: display.clone(),
        created_at: Some(now_iso()),
        last_launch_at: None,
        steam_id: None,
        avatar_file: None,
        favorite: false,
        launch_count: 0,
        has_authenticator: false,
        authenticator_account_name: None,
        authenticator_imported_at: None,
    };
    // Best-effort: pick up SteamID + avatar straight away.
    backfill_meta(&dir, login, &mut meta);
    write_meta(&dir, &meta)?;
    Ok(meta_to_account(login.to_string(), dir, meta))
}

pub fn set_favorite(workspace: &Path, login: &str, value: bool) -> AppResult<()> {
    validate_login(login)?;
    let dir = account_dir(workspace, login);
    if !dir.exists() {
        return Err(AppError::NotFound(format!("account {login}")));
    }
    let mut meta = read_meta(&dir);
    meta.favorite = value;
    write_meta(&dir, &meta)
}

pub fn refresh_avatar(workspace: &Path, login: &str) -> AppResult<Option<PathBuf>> {
    validate_login(login)?;
    let dir = account_dir(workspace, login);
    if !dir.exists() {
        return Err(AppError::NotFound(format!("account {login}")));
    }
    let main = crate::steam_paths::detect(None)?;
    let mut meta = read_meta(&dir);
    // Force a fresh lookup of SteamID too — user may have re-logged-in to the
    // host Steam since the account was added.
    if let Some(sid) = lookup_steamid_for(login, &main) {
        meta.steam_id = Some(sid.clone());
        if let Some(rel) = copy_avatar(&dir, &sid, &main) {
            meta.avatar_file = Some(rel);
        }
    }
    write_meta(&dir, &meta)?;
    Ok(meta.avatar_file.map(|f| dir.join(f)))
}

// `vdf` import to keep editor happy if not otherwise used here.
#[allow(dead_code)]
fn _vdf_link() {
    let _ = vdf::AccountHealth {
        junction: junction::JunctionHealth::Missing,
        config_dir_exists: false,
        has_loginusers_vdf: false,
        ready: false,
    };
}

pub fn remove_account(workspace: &Path, login: &str, delete_files: bool) -> AppResult<()> {
    validate_login(login)?;
    let dir = account_dir(workspace, login);
    if !dir.exists() {
        return Ok(());
    }
    let link = dir.join("steamapps");
    // CRITICAL: must remove the junction before any recursive delete, otherwise
    // remove_dir_all will follow the reparse point and wipe the user's actual
    // Steam libraries. If junction removal fails we MUST abort.
    if junction::is_junction(&link) {
        if let Err(e) = junction::remove(&link) {
            return Err(AppError::Junction(format!(
                "refusing to delete account dir while junction is still present at {}: {e}",
                link.display()
            )));
        }
        // Wait briefly for the OS to release the reparse point handle.
        std::thread::sleep(std::time::Duration::from_millis(120));
        if junction::is_junction(&link) {
            return Err(AppError::Junction(format!(
                "junction still present after removal at {}",
                link.display()
            )));
        }
    }
    if delete_files {
        // Tolerate transient AV/Explorer locks with a few retries.
        let mut last_err: Option<std::io::Error> = None;
        for i in 0..5 {
            match fs::remove_dir_all(&dir) {
                Ok(()) => {
                    last_err = None;
                    break;
                }
                Err(e) => {
                    tracing::warn!("remove_dir_all attempt {}: {}", i + 1, e);
                    last_err = Some(e);
                    std::thread::sleep(std::time::Duration::from_millis(180));
                }
            }
        }
        if let Some(e) = last_err {
            return Err(AppError::Io(format!(
                "remove account dir {}: {}",
                dir.display(),
                e
            )));
        }
    }
    Ok(())
}

pub fn touch_last_launch(workspace: &Path, login: &str) -> AppResult<()> {
    let dir = account_dir(workspace, login);
    let mut meta = read_meta(&dir);
    meta.last_launch_at = Some(now_iso());
    meta.launch_count = meta.launch_count.saturating_add(1);
    write_meta(&dir, &meta)
}

/// P11: mark the account as having (or no longer having) a maFile attached.
/// Also fills in the `accountName` field for UI/debug. Best-effort: if the
/// account directory does not exist yet, fails with NotFound.
pub fn set_authenticator_meta(
    workspace: &Path,
    login: &str,
    has_auth: bool,
    account_name: Option<String>,
) -> AppResult<()> {
    validate_login(login)?;
    let dir = account_dir(workspace, login);
    if !dir.exists() {
        return Err(AppError::NotFound(format!("account {login}")));
    }
    let mut meta = read_meta(&dir);
    meta.has_authenticator = has_auth;
    meta.authenticator_account_name = account_name;
    meta.authenticator_imported_at = if has_auth { Some(now_iso()) } else { None };
    write_meta(&dir, &meta)
}

/// P11: returns `<account>/auth/` (creating it). Used by `sda::vault`.
pub fn auth_dir(workspace: &Path, login: &str) -> AppResult<PathBuf> {
    validate_login(login)?;
    let dir = account_dir(workspace, login).join("auth");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub enum ChangeStrategy {
    Move,
    Relink,
    Cancel,
}

pub fn change_workspace(
    old: &Path,
    new: &Path,
    strategy: ChangeStrategy,
    main_steamapps: &Path,
) -> AppResult<()> {
    if matches!(strategy, ChangeStrategy::Cancel) {
        return Ok(());
    }
    fs::create_dir_all(new)?;
    match strategy {
        ChangeStrategy::Move => {
            let old_accounts = accounts_dir(old);
            let new_accounts = accounts_dir(new);
            fs::create_dir_all(&new_accounts)?;
            if old_accounts.exists() {
                for entry in fs::read_dir(&old_accounts)? {
                    let entry = entry?;
                    let from = entry.path();
                    let name = entry.file_name();
                    let to = new_accounts.join(&name);
                    // Remove junction first (so we don't move into shared steamapps)
                    let link = from.join("steamapps");
                    junction::remove(&link).ok();
                    fs::rename(&from, &to).or_else(|_| copy_dir_all(&from, &to))?;
                    // Re-create junction at new location
                    let new_link = to.join("steamapps");
                    junction::create(&new_link, main_steamapps).ok();
                }
                fs::remove_dir_all(&old_accounts).ok();
            }
        }
        ChangeStrategy::Relink => {
            // Leave old as-is; ensure new accounts dir exists; user will re-add.
            fs::create_dir_all(accounts_dir(new))?;
        }
        ChangeStrategy::Cancel => unreachable!(),
    }
    Ok(())
}

fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}
