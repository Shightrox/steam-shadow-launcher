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

static META_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn now_iso() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{}", secs)
}

pub(crate) fn validate_login(login: &str) -> AppResult<()> {
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
    let stem = login.split('.').next().unwrap_or("").to_ascii_uppercase();
    if login == "."
        || login == ".."
        || login.ends_with('.')
        || matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
    {
        return Err(AppError::Workspace("ACCOUNT_INVALID_NAME".into()));
    }
    Ok(())
}

pub fn validate_workspace(path: &Path, main_steam: &Path) -> AppResult<()> {
    if path.as_os_str().is_empty() {
        return Err(AppError::Workspace("workspace path is empty".into()));
    }
    let abs = crate::storage::path_key(path)?;
    let main = crate::storage::path_key(main_steam)?;
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

pub fn checked_account_dir(workspace: &Path, login: &str) -> AppResult<PathBuf> {
    validate_login(login)?;
    let accounts = accounts_dir(workspace);
    let dir = accounts.join(login);
    if crate::storage::is_reparse(&accounts)
        || crate::storage::is_reparse(&dir)
        || !crate::storage::path_key(&dir)?.starts_with(crate::storage::path_key(&accounts)?)
    {
        return Err(AppError::Workspace("ACCOUNT_UNSAFE_PATH".into()));
    }
    Ok(dir)
}

pub fn ensure_account_dirs(workspace: &Path, login: &str) -> AppResult<PathBuf> {
    validate_login(login)?;
    let dir = checked_account_dir(workspace, login)?;
    for child in ["config", "logs", "userdata"] {
        if crate::storage::is_reparse(&dir.join(child)) {
            return Err(AppError::Workspace("ACCOUNT_UNSAFE_PATH".into()));
        }
    }
    fs::create_dir_all(dir.join("config"))?;
    fs::create_dir_all(dir.join("logs"))?;
    fs::create_dir_all(dir.join("userdata"))?;
    Ok(dir)
}

pub(crate) fn read_meta(dir: &Path) -> ShadowMeta {
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
    crate::storage::atomic_write(&dir.join("shadow.json"), txt.as_bytes())?;
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
    let _meta_guard = META_LOCK.lock().unwrap();
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
        if checked_account_dir(workspace, &login).is_err() {
            continue;
        }
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
    let _meta_guard = META_LOCK.lock().unwrap();
    validate_login(login)?;
    let existing = checked_account_dir(workspace, login)?;
    if existing.join("shadow.json").exists() {
        return Err(AppError::Workspace("ACCOUNT_ALREADY_EXISTS".into()));
    }
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
    let _meta_guard = META_LOCK.lock().unwrap();
    validate_login(login)?;
    let dir = checked_account_dir(workspace, login)?;
    if !dir.exists() {
        return Err(AppError::NotFound(format!("account {login}")));
    }
    let mut meta = read_meta(&dir);
    meta.favorite = value;
    write_meta(&dir, &meta)
}

pub fn refresh_avatar(workspace: &Path, login: &str) -> AppResult<Option<PathBuf>> {
    let _meta_guard = META_LOCK.lock().unwrap();
    validate_login(login)?;
    let dir = checked_account_dir(workspace, login)?;
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
    let dir = checked_account_dir(workspace, login)?;
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
    let _meta_guard = META_LOCK.lock().unwrap();
    let dir = checked_account_dir(workspace, login)?;
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
    let _meta_guard = META_LOCK.lock().unwrap();
    validate_login(login)?;
    let dir = checked_account_dir(workspace, login)?;
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
    let dir = checked_account_dir(workspace, login)?.join("auth");
    if crate::storage::is_reparse(&dir) {
        return Err(AppError::Workspace("ACCOUNT_UNSAFE_PATH".into()));
    }
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
    let old_key = crate::storage::path_key(old)?;
    let new_key = crate::storage::path_key(new)?;
    if old_key == new_key {
        return Ok(());
    }
    if old_key.starts_with(&new_key) || new_key.starts_with(&old_key) {
        return Err(AppError::Workspace("WORKSPACE_OVERLAP".into()));
    }
    fs::create_dir_all(new)?;
    match strategy {
        ChangeStrategy::Move => {
            let old_accounts = accounts_dir(old);
            let new_accounts = accounts_dir(new);
            if new_accounts.exists() && fs::read_dir(&new_accounts)?.next().is_some()
                || new.join("vault-policy.json").exists()
            {
                return Err(AppError::Workspace(
                    "WORKSPACE_DESTINATION_NOT_EMPTY".into(),
                ));
            }
            let stage = new.join(format!(".shadow-move-{:016x}", rand::random::<u64>()));
            fs::create_dir(&stage)?;
            // Copy first. The source remains usable even if copying, committing,
            // or the subsequent settings write fails. Never merge account trees.
            let result = (|| -> AppResult<()> {
                let staged_accounts = stage.join("accounts");
                fs::create_dir(&staged_accounts)?;
                if old_accounts.exists() {
                    for entry in fs::read_dir(&old_accounts)? {
                        let entry = entry?;
                        let login = entry.file_name().to_string_lossy().to_string();
                        checked_account_dir(old, &login)?;
                        if !entry.file_type()?.is_dir() {
                            continue;
                        }
                        let target = staged_accounts.join(&login);
                        copy_account_tree(&entry.path(), &target, true)?;
                        junction::create(&target.join("steamapps"), main_steamapps)?;
                    }
                }
                let policy = old.join("vault-policy.json");
                if policy.exists() {
                    fs::copy(&policy, stage.join("vault-policy.json"))?;
                }
                if new_accounts.exists() {
                    fs::remove_dir(&new_accounts)?;
                }
                fs::rename(staged_accounts, &new_accounts)?;
                if stage.join("vault-policy.json").exists() {
                    fs::rename(
                        stage.join("vault-policy.json"),
                        new.join("vault-policy.json"),
                    )?;
                }
                crate::storage::atomic_write(
                    &new.join("workspace-move.json"),
                    &serde_json::to_vec(&old_key.to_string_lossy())?,
                )?;
                Ok(())
            })();
            // A failed stage is retained for diagnosis/recovery, never deleted
            // recursively through an unverified junction.
            if result.is_ok() {
                fs::remove_dir(&stage)?;
            }
            result?;
        }
        ChangeStrategy::Relink => {
            // Leave old as-is; ensure new accounts dir exists; user will re-add.
            fs::create_dir_all(accounts_dir(new))?;
        }
        ChangeStrategy::Cancel => unreachable!(),
    }
    Ok(())
}

fn copy_account_tree(src: &Path, dst: &Path, root: bool) -> AppResult<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if root && entry.file_name() == "steamapps" {
            continue;
        }
        if crate::storage::is_reparse(&from) {
            return Err(AppError::Workspace("WORKSPACE_REPARSE_POINT".into()));
        }
        if ty.is_dir() {
            copy_account_tree(&from, &to, false)?;
        } else {
            fs::copy(&from, &to)?;
            if fs::read(&from)? != fs::read(&to)? {
                return Err(AppError::Io("workspace copy verification failed".into()));
            }
        }
    }
    Ok(())
}

/// Called only after settings durably point to the new, fully copied workspace.
/// Keep the old accounts as one recoverable archive rather than recursively
/// deleting secrets. It is excluded from account discovery.
pub fn finish_move(old: &Path, new: &Path) -> AppResult<()> {
    let marker = new.join("workspace-move.json");
    if !marker.exists() {
        return Ok(());
    }
    let expected: String = serde_json::from_slice(&fs::read(&marker)?)?;
    if PathBuf::from(expected) != crate::storage::path_key(old)? {
        return Err(AppError::Workspace("WORKSPACE_MOVE_MISMATCH".into()));
    }
    let source = accounts_dir(old);
    if source.exists() {
        let archive = old.join(format!("moved-accounts-{}", now_iso()));
        if archive.exists() {
            return Err(AppError::Workspace("WORKSPACE_ARCHIVE_EXISTS".into()));
        }
        fs::rename(&source, archive)?;
    }
    fs::remove_file(marker)?;
    Ok(())
}

pub fn validate_auth_identity(
    workspace: &Path,
    login: &str,
    mf: &crate::sda::mafile::MaFile,
) -> AppResult<()> {
    let dir = checked_account_dir(workspace, login)?;
    if !dir.is_dir() {
        return Err(AppError::NotFound(format!("account {login}")));
    }
    let meta = read_meta(&dir);
    let imported_id = mf
        .session
        .as_ref()
        .map(|s| s.steam_id)
        .filter(|id| *id != 0);
    if let Some(expected) = meta.steam_id.as_deref().filter(|id| !id.is_empty()) {
        if imported_id.is_some_and(|id| id.to_string() != expected) {
            return Err(AppError::NotReady("AUTH_ACCOUNT_MISMATCH".into()));
        }
    }
    // Folder login is a Steam login, displayName is the editable alias. An
    // export without SteamID must still match the account name.
    if !mf.account_name.eq_ignore_ascii_case(login) {
        return Err(AppError::NotReady("AUTH_ACCOUNT_MISMATCH".into()));
    }
    Ok(())
}

#[cfg(test)]
mod regression_tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let p =
                std::env::temp_dir().join(format!("shadow-workspace-{}", rand::random::<u64>()));
            fs::create_dir(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            assert!(fs::canonicalize(&self.0)
                .unwrap()
                .starts_with(fs::canonicalize(std::env::temp_dir()).unwrap()));
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    #[test]
    fn unsafe_names_and_nested_moves_are_rejected() {
        for login in [
            ".",
            "..",
            "user.",
            "CON",
            "nul.txt",
            "LPT1",
            "com9.file",
            "a/b",
            "a\\b",
        ] {
            assert!(validate_login(login).is_err(), "{login}");
        }
        for login in ["user_1", "user-name", "user.name", "com10"] {
            assert!(validate_login(login).is_ok());
        }
        let f = Fixture::new();
        assert!(change_workspace(
            &f.0,
            &f.0.join("nested"),
            ChangeStrategy::Move,
            &f.0.join("library")
        )
        .is_err());
    }
    #[test]
    fn move_and_duplicate_add_preserve_existing_data() {
        let f = Fixture::new();
        let g = Fixture::new();
        let dir = ensure_account_dirs(&f.0, "demo").unwrap();
        write_meta(
            &dir,
            &ShadowMeta {
                favorite: true,
                launch_count: 42,
                has_authenticator: true,
                ..Default::default()
            },
        )
        .unwrap();
        let bytes = fs::read(dir.join("shadow.json")).unwrap();
        assert!(add_account(&f.0, &f.0.join("library"), "demo", None).is_err());
        assert_eq!(fs::read(dir.join("shadow.json")).unwrap(), bytes);
        fs::create_dir_all(g.0.join("accounts/demo")).unwrap();
        fs::write(g.0.join("accounts/demo/keep"), b"target").unwrap();
        assert!(change_workspace(&f.0, &g.0, ChangeStrategy::Move, &f.0.join("library")).is_err());
        assert_eq!(fs::read(g.0.join("accounts/demo/keep")).unwrap(), b"target");
        assert_eq!(fs::read(dir.join("shadow.json")).unwrap(), bytes);
    }
    #[test]
    fn successful_move_copies_before_archiving_and_preserves_vault_policy() {
        let f = Fixture::new(); let target = f.0.join("new"); let source = f.0.join("old");
        let library = f.0.join("library"); fs::create_dir(&library).unwrap();
        let account = ensure_account_dirs(&source, "demo").unwrap();
        fs::write(account.join("config/preserved"), b"account-state").unwrap();
        fs::write(source.join("vault-policy.json"), b"encrypted-policy").unwrap();
        change_workspace(&source, &target, ChangeStrategy::Move, &library).unwrap();
        assert!(account.exists());
        assert_eq!(fs::read(target.join("accounts/demo/config/preserved")).unwrap(), b"account-state");
        assert_eq!(fs::read(target.join("vault-policy.json")).unwrap(), b"encrypted-policy");
        assert!(matches!(junction::verify(&target.join("accounts/demo/steamapps"), &library).unwrap(), junction::JunctionHealth::Healthy));
        finish_move(&source, &target).unwrap();
        assert!(!source.join("accounts").exists());
        assert!(fs::read_dir(&source).unwrap().map(Result::unwrap).any(|entry| entry.file_name().to_string_lossy().starts_with("moved-accounts-")));
        junction::remove(&target.join("accounts/demo/steamapps")).unwrap();
    }
    #[test]
    fn imported_identity_must_match_name_and_known_steam_id() {
        let f = Fixture::new();
        let dir = ensure_account_dirs(&f.0, "demo").unwrap();
        write_meta(
            &dir,
            &ShadowMeta {
                steam_id: Some("123".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let mut mf = crate::sda::mafile::MaFile::default();
        mf.account_name = "other".into();
        assert!(validate_auth_identity(&f.0, "demo", &mf).is_err());
        mf.account_name = "DEMO".into();
        mf.session = Some(crate::sda::mafile::SessionData {
            steam_id: 999,
            access_token: String::new(), refresh_token: String::new(), session_id: String::new()
        });
        assert!(validate_auth_identity(&f.0, "demo", &mf).is_err());
        mf.session.as_mut().unwrap().steam_id = 123;
        assert!(validate_auth_identity(&f.0, "demo", &mf).is_ok());
    }
}
