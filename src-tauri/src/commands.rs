use crate::error::{AppError, AppResult};
use crate::junction::CleanupReport;
use crate::launcher::{self, LaunchMode, LaunchOutcome};
use crate::library::{self, InstalledGame};
use crate::sandboxie::{self, SandboxieInfo};
use crate::sda;
use crate::settings::Settings;
use crate::shortcut;
use crate::steam_paths::MainSteamInfo;
use crate::steam_process::{self, RunningGame};
use crate::switcher;
use crate::vdf::{AccountHealth, DiscoveredAccount};
use crate::workspace::{Account, ChangeStrategy};
use crate::{download, junction, settings, steam_paths, vdf, workspace};
use serde::Serialize;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter};

fn require_workspace(
    s: &Settings,
    expected: Option<&std::path::Path>,
) -> AppResult<crate::lifecycle::Workspace> {
    let ws = s
        .workspace
        .clone()
        .ok_or_else(|| AppError::Workspace("workspace not configured".into()))?;
    let ws = crate::lifecycle::Workspace::new(ws);
    if let Some(expected) = expected {
        if crate::storage::path_key(expected)? != crate::storage::path_key(&ws)? {
            return Err(AppError::Workspace("WORKSPACE_CHANGED".into()));
        }
    }
    if settings::load()?.workspace.as_ref() != Some(&*ws) {
        return Err(AppError::Workspace("WORKSPACE_CHANGED".into()));
    }
    sda::vault::initialize(&ws, s.auth_master_password_enabled)?;
    Ok(ws)
}

fn require_main(s: &Settings) -> AppResult<MainSteamInfo> {
    steam_paths::detect(s.main_steam_path_override.clone())
}

#[tauri::command(async)]
pub fn detect_main_steam() -> AppResult<MainSteamInfo> {
    let s = settings::load()?;
    steam_paths::detect(s.main_steam_path_override)
}

#[tauri::command(async)]
pub fn get_settings() -> AppResult<Settings> {
    let mut s = settings::load()?;
    if let Some(ws) = &s.workspace {
        s.auth_master_password_enabled =
            sda::vault::initialize(ws, s.auth_master_password_enabled)?;
    }
    Ok(s)
}

#[tauri::command(async)]
pub fn save_settings(settings: Settings) -> AppResult<()> {
    settings::update(|current| {
        current.language = if settings.language == "en" {
            "en".into()
        } else {
            "ru".into()
        };
        current.default_launch_mode = settings.default_launch_mode;
    })
}

#[tauri::command(async)]
pub fn list_accounts(expected_workspace: Option<PathBuf>) -> AppResult<Vec<Account>> {
    let s = settings::load()?;
    let ws = require_workspace(&s, expected_workspace.as_deref())?;
    workspace::list_accounts(&ws)
}

#[tauri::command(async)]
pub fn add_account(
    login: String,
    display: Option<String>,
    expected_workspace: Option<PathBuf>,
) -> AppResult<Account> {
    let s = settings::load()?;
    let ws = require_workspace(&s, expected_workspace.as_deref())?;
    let main = require_main(&s)?;
    workspace::add_account(&ws, &main.steamapps_dir, &login, display)
}

#[tauri::command(async)]
pub fn remove_account(
    login: String,
    delete_files: bool,
    expected_workspace: Option<PathBuf>,
) -> AppResult<()> {
    let s = settings::load()?;
    let ws = require_workspace(&s, expected_workspace.as_deref())?;
    let gate = sda::session::account_gate(&ws, &login)?;
    let _guard = gate.lock().unwrap();
    sda::reauth::forget(&ws, &login)?;
    sda::add::drop_session(&login);
    // also try to remove sandbox box
    let sb = sandboxie::detect();
    if sb.installed {
        let _ = sandboxie::remove_box(&sb, &login);
    }
    workspace::remove_account(&ws, &login, delete_files)
}

#[tauri::command(async)]
pub fn verify_account(
    login: String,
    expected_workspace: Option<PathBuf>,
) -> AppResult<AccountHealth> {
    let s = settings::load()?;
    let ws = require_workspace(&s, expected_workspace.as_deref())?;
    let main = require_main(&s)?;
    let dir = workspace::checked_account_dir(&ws, &login)?;
    vdf::is_account_ready(&dir, &main.steamapps_dir)
}

#[tauri::command(async)]
pub fn repair_account(login: String, expected_workspace: Option<PathBuf>) -> AppResult<()> {
    let s = settings::load()?;
    let ws = require_workspace(&s, expected_workspace.as_deref())?;
    let main = require_main(&s)?;
    let dir = workspace::ensure_account_dirs(&ws, &login)?;
    let link = dir.join("steamapps");
    junction::repair(&link, &main.steamapps_dir)?;
    Ok(())
}

#[tauri::command(async)]
pub fn launch_shadow(
    login: String,
    mode: Option<String>,
    expected_workspace: Option<PathBuf>,
) -> AppResult<LaunchOutcome> {
    let s = settings::load()?;
    let ws = require_workspace(&s, expected_workspace.as_deref())?;
    let main = require_main(&s)?;
    let sb = sandboxie::detect();
    let accounts = workspace::list_accounts(&ws)?;
    let account = accounts
        .into_iter()
        .find(|a| a.login == login)
        .ok_or_else(|| AppError::NotFound(format!("account {login}")))?;
    let mode_str = mode.unwrap_or(s.default_launch_mode.clone());
    let m = LaunchMode::parse(&mode_str)?;
    let outcome = launcher::launch(&ws, &main, &sb, &account, m)?;
    workspace::touch_last_launch(&ws, &login).ok();
    Ok(outcome)
}

#[tauri::command(async)]
pub fn change_workspace(new_path: PathBuf, strategy: String) -> AppResult<()> {
    let _exclusive = crate::lifecycle::write();
    let mut s = settings::load()?;
    let main = require_main(&s)?;
    workspace::validate_workspace(&new_path, &main.install_dir)?;
    let strat = match strategy.as_str() {
        "Move" => ChangeStrategy::Move,
        "Relink" => ChangeStrategy::Relink,
        "Cancel" => return Ok(()),
        _ => return Err(AppError::Workspace("unknown strategy".into())),
    };
    if let Some(old) = s.workspace.clone() {
        sda::vault::initialize(&old, s.auth_master_password_enabled)?;
        if old != new_path {
            workspace::change_workspace(&old, &new_path, strat, &main.steamapps_dir)?;
        }
    }
    let old = s.workspace.clone();
    s.auth_master_password_enabled = sda::vault::initialize(&new_path, false)?;
    s.workspace = Some(new_path.clone());
    settings::update(|current| {
        current.workspace = s.workspace;
        current.auth_master_password_enabled = s.auth_master_password_enabled;
    })?;
    sda::vault::lock();
    sda::add::clear_sessions();
    sda::reauth::clear_pending();
    sda::relogin_flag::clear_all();
    if matches!(strat, ChangeStrategy::Move) {
        if let Some(old) = old.filter(|old| old != &new_path) {
            if let Err(e) = workspace::finish_move(&old, &new_path) {
                tracing::warn!("Source archive deferred: {e}");
            }
        }
    }
    Ok(())
}

#[tauri::command(async)]
pub fn set_workspace_initial(new_path: PathBuf) -> AppResult<()> {
    let _exclusive = crate::lifecycle::write();
    let mut s = settings::load()?;
    let main = require_main(&s)?;
    workspace::validate_workspace(&new_path, &main.install_dir)?;
    s.workspace = Some(new_path);
    s.first_run_completed = true;
    settings::save(&s)
}

#[tauri::command(async)]
pub fn set_main_steam_override(new_path: Option<PathBuf>) -> AppResult<()> {
    settings::update(|s| s.main_steam_path_override = new_path)
}

#[tauri::command(async)]
pub fn cleanup_stale_junctions(expected_workspace: Option<PathBuf>) -> AppResult<CleanupReport> {
    let s = settings::load()?;
    let ws = require_workspace(&s, expected_workspace.as_deref())?;
    let main = require_main(&s)?;
    junction::cleanup_stale(&ws, &main.steamapps_dir)
}

#[tauri::command(async)]
pub fn discover_steam_accounts() -> AppResult<Vec<DiscoveredAccount>> {
    let s = settings::load()?;
    let main = require_main(&s)?;
    vdf::parse_loginusers(&main.install_dir)
}

#[tauri::command(async)]
pub fn import_discovered_accounts(
    logins: Vec<String>,
    personas: std::collections::HashMap<String, String>,
    expected_workspace: Option<PathBuf>,
) -> AppResult<Vec<Account>> {
    let s = settings::load()?;
    let ws = require_workspace(&s, expected_workspace.as_deref())?;
    let main = require_main(&s)?;
    let mut out = Vec::new();
    for login in logins {
        let display = personas.get(&login).cloned();
        let acc = workspace::add_account(&ws, &main.steamapps_dir, &login, display)?;
        out.push(acc);
    }
    Ok(out)
}

#[tauri::command(async)]
pub fn default_workspace() -> AppResult<Option<PathBuf>> {
    Ok(settings::default_workspace_path())
}

#[tauri::command(async)]
pub fn detect_sandboxie() -> AppResult<SandboxieInfo> {
    Ok(sandboxie::detect())
}

#[tauri::command(async)]
pub fn install_sandboxie(installer_path: PathBuf) -> AppResult<SandboxieInfo> {
    sandboxie::install_silent(&installer_path)?;
    let mut s = settings::load()?;
    s.sandboxie_install_attempted = true;
    settings::save(&s).ok();
    Ok(sandboxie::detect())
}

#[derive(Debug, Clone, Serialize)]
struct ProgressEvent {
    phase: &'static str,
    downloaded: u64,
    total: Option<u64>,
    percent: Option<u32>,
    name: Option<String>,
}

fn emit_progress(
    app: &AppHandle,
    phase: &'static str,
    downloaded: u64,
    total: Option<u64>,
    name: Option<String>,
) {
    let percent = total.map(|t| {
        if t == 0 {
            0
        } else {
            ((downloaded * 100) / t).min(100) as u32
        }
    });
    let _ = app.emit(
        "sandboxie-download-progress",
        ProgressEvent {
            phase,
            downloaded,
            total,
            percent,
            name,
        },
    );
}

#[tauri::command]
pub async fn download_and_install_sandboxie(app: AppHandle) -> AppResult<SandboxieInfo> {
    tauri::async_runtime::spawn_blocking(move || -> AppResult<SandboxieInfo> {
        emit_progress(&app, "resolving", 0, None, None);
        let asset = download::fetch_latest_sandboxie_asset()?;
        let dst = download::downloads_dir()?.join(&asset.name);
        emit_progress(&app, "downloading", 0, asset.size, Some(asset.name.clone()));
        let app_for_cb = app.clone();
        let asset_name = asset.name.clone();
        download::download_with_progress(&asset.url, &dst, move |d, t| {
            emit_progress(&app_for_cb, "downloading", d, t, Some(asset_name.clone()));
        })?;
        emit_progress(&app, "installing", 0, None, Some(asset.name.clone()));
        sandboxie::install_silent(&dst)?;
        let mut s = settings::load()?;
        s.sandboxie_install_attempted = true;
        settings::save(&s).ok();
        let info = sandboxie::detect();
        emit_progress(
            &app,
            if info.installed { "done" } else { "failed" },
            0,
            None,
            Some(asset.name),
        );
        Ok(info)
    })
    .await
    .map_err(|e| AppError::Other(format!("join: {e}")))?
}

#[tauri::command(async)]
pub fn list_running_games() -> AppResult<Vec<RunningGame>> {
    let s = settings::load()?;
    let main = require_main(&s)?;
    Ok(steam_process::find_running_games(&main.steamapps_dir))
}

#[tauri::command(async)]
pub fn revert_last_switch(expected_workspace: Option<PathBuf>) -> AppResult<()> {
    let s = settings::load()?;
    let ws = require_workspace(&s, expected_workspace.as_deref())?;
    let main = require_main(&s)?;
    switcher::revert_last(&ws, &main)
}

#[tauri::command(async)]
pub fn close_window(app: tauri::AppHandle) -> AppResult<()> {
    use tauri::Manager;
    if let Some(w) = app.get_webview_window("main") {
        w.close()
            .map_err(|e| AppError::Other(format!("close: {e}")))?;
    }
    Ok(())
}

// ── Self-update ──────────────────────────────────────────────────────────
//
// `check_update` hits GitHub (cached at the HTTP layer) and reports back
// whether a newer release is available. `apply_update` swaps the live exe
// with the freshly-downloaded portable build and exits the app — the
// detached batch script then re-launches the new version.

#[tauri::command]
pub async fn check_update() -> AppResult<crate::updater::UpdateInfo> {
    tauri::async_runtime::spawn_blocking(crate::updater::check_update)
        .await
        .map_err(|e| AppError::Other(format!("check_update join: {e}")))?
}

#[tauri::command]
pub async fn apply_update(app: tauri::AppHandle, url: String) -> AppResult<()> {
    tauri::async_runtime::spawn_blocking(move || crate::updater::apply_update(&url))
        .await
        .map_err(|e| AppError::Other(format!("apply_update join: {e}")))??;
    // Give the renderer ~300ms to flush the "restarting" toast before we
    // pull the rug. The detached cmd will wait for our PID to disappear
    // before promoting the new exe.
    let app_clone = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(300));
        app_clone.exit(0);
    });
    Ok(())
}

#[tauri::command(async)]
pub fn minimize_window(app: tauri::AppHandle) -> AppResult<()> {
    use tauri::Manager;
    if let Some(w) = app.get_webview_window("main") {
        w.minimize()
            .map_err(|e| AppError::Other(format!("minimize: {e}")))?;
    }
    Ok(())
}

#[tauri::command(async)]
pub fn start_drag(app: tauri::AppHandle) -> AppResult<()> {
    use tauri::Manager;
    if let Some(w) = app.get_webview_window("main") {
        w.start_dragging()
            .map_err(|e| AppError::Other(format!("drag: {e}")))?;
    }
    Ok(())
}

#[tauri::command(async)]
pub fn is_elevated() -> AppResult<bool> {
    Ok(sandboxie::is_elevated_pub())
}

#[tauri::command(async)]
pub fn relaunch_as_admin(app: AppHandle) -> AppResult<()> {
    sandboxie::relaunch_self_as_admin()?;
    // Give the new process a moment to spawn before we exit.
    std::thread::sleep(std::time::Duration::from_millis(300));
    app.exit(0);
    Ok(())
}

#[tauri::command(async)]
pub fn set_account_favorite(
    login: String,
    value: bool,
    expected_workspace: Option<PathBuf>,
) -> AppResult<()> {
    let s = settings::load()?;
    let ws = require_workspace(&s, expected_workspace.as_deref())?;
    workspace::set_favorite(&ws, &login, value)
}

#[tauri::command(async)]
pub fn refresh_account_avatar(
    login: String,
    expected_workspace: Option<PathBuf>,
) -> AppResult<Option<PathBuf>> {
    let s = settings::load()?;
    let ws = require_workspace(&s, expected_workspace.as_deref())?;
    workspace::refresh_avatar(&ws, &login)
}

#[tauri::command(async)]
pub fn list_running_sandboxes() -> AppResult<Vec<sandboxie::RunningSandbox>> {
    let info = sandboxie::detect();
    if !info.installed {
        return Ok(vec![]);
    }
    Ok(sandboxie::list_running(&info))
}

#[tauri::command(async)]
pub fn stop_sandbox(login: String) -> AppResult<()> {
    let info = sandboxie::detect();
    if !info.installed {
        return Err(AppError::NotReady("Sandboxie not installed".into()));
    }
    sandboxie::stop_box(&info, &login)
}

#[tauri::command(async)]
pub fn list_account_games(_login: String) -> AppResult<Vec<InstalledGame>> {
    // NOTE: The `_login` parameter is kept for forward compatibility — later
    // we may filter by owned-apps once we wire up the Steam Web API. Right
    // now we return every app installed in any registered Steam library on
    // this machine, which is a superset of what any account can launch.
    let s = settings::load()?;
    let main = require_main(&s)?;
    Ok(library::list_installed_games(&main))
}

#[tauri::command(async)]
pub fn launch_game(
    login: String,
    appid: u32,
    mode: Option<String>,
    expected_workspace: Option<PathBuf>,
) -> AppResult<LaunchOutcome> {
    let s = settings::load()?;
    let ws = require_workspace(&s, expected_workspace.as_deref())?;
    let main = require_main(&s)?;
    let sb = sandboxie::detect();
    let accounts = workspace::list_accounts(&ws)?;
    let account = accounts
        .into_iter()
        .find(|a| a.login == login)
        .ok_or_else(|| AppError::NotFound(format!("account {login}")))?;
    let mode_str = mode.unwrap_or(s.default_launch_mode.clone());
    let m = LaunchMode::parse(&mode_str)?;
    let outcome = launcher::launch_game(&ws, &main, &sb, &account, m, appid)?;
    workspace::touch_last_launch(&ws, &login).ok();
    Ok(outcome)
}

#[tauri::command(async)]
pub fn open_url(url: String) -> AppResult<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::{ShellExecuteExW, SHELLEXECUTEINFOW};
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    // Whitelist schemes to avoid opening arbitrary launcher-controlled URLs.
    let lower = url.to_lowercase();
    if !(lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("steam://"))
    {
        return Err(AppError::Other(format!("unsupported url scheme: {url}")));
    }

    let verb: Vec<u16> = "open\0".encode_utf16().collect();
    let file: Vec<u16> = std::ffi::OsString::from(&url)
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut sei = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..unsafe { std::mem::zeroed() }
    };
    unsafe {
        ShellExecuteExW(&mut sei as *mut _)
            .map_err(|e| AppError::Process(format!("ShellExecuteExW(open): {e}")))?;
    }
    Ok(())
}

#[tauri::command(async)]
pub fn create_account_shortcut(login: String) -> AppResult<PathBuf> {
    shortcut::create_desktop_shortcut(&login)
}

/// Reveal an account's auth folder (where maFile.json / maFile.enc lives) in
/// the system file explorer. Returns the resolved path so the caller can show
/// it to the user even if launching Explorer fails.
#[tauri::command(async)]
pub fn auth_open_folder(login: String, expected_workspace: Option<PathBuf>) -> AppResult<PathBuf> {
    let s = settings::load()?;
    let ws = require_workspace(&s, expected_workspace.as_deref())?;
    let dir = workspace::auth_dir(&ws, &login)?;
    // Best-effort `explorer.exe <path>`. Don't surface failure as an error —
    // the user still has the path.
    let _ = std::process::Command::new("explorer.exe").arg(&dir).spawn();
    Ok(dir)
}

// ─────────────────────────────────────────────────────────────────────────
// P11 — Steam Desktop Authenticator
// ─────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct GuardCode {
    pub code: String,
    /// Unix-seconds when this code was generated (server-aligned).
    #[serde(rename = "generatedAt")]
    pub generated_at: i64,
    /// Seconds remaining in the current 30s TOTP window.
    #[serde(rename = "periodRemaining")]
    pub period_remaining: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AccountAuthStatus {
    pub login: String,
    #[serde(rename = "hasAuthenticator")]
    pub has_authenticator: bool,
    #[serde(rename = "hasSavedPassword")]
    pub has_saved_password: bool,
    #[serde(rename = "autoLogin")]
    pub auto_login: sda::credentials::AutoLoginStatus,
    #[serde(rename = "steamId")]
    pub steam_id: Option<String>,
    pub enrollment: &'static str,
    #[serde(rename = "identityMismatch")]
    pub identity_mismatch: bool,
    #[serde(rename = "accountName")]
    pub account_name: Option<String>,
    #[serde(rename = "importedAt")]
    pub imported_at: Option<String>,
    #[serde(rename = "sessionState")]
    pub session_state: sda::session_state::SessionState,
}

/// Return per-account authenticator presence. Does NOT load shared_secret.
#[tauri::command(async)]
pub fn auth_status(expected_workspace: Option<PathBuf>) -> AppResult<Vec<AccountAuthStatus>> {
    let s = settings::load()?;
    let ws = require_workspace(&s, expected_workspace.as_deref())?;
    let accounts = workspace::list_accounts(&ws)?;
    let now = sda::session_state::now_secs();
    let mut out = Vec::with_capacity(accounts.len());
    for a in accounts {
        let has = sda::vault::has_any(&ws, &a.login);
        let mf = sda::vault::load_plain(&ws, &a.login).ok().flatten();
        let mismatch = mf
            .as_ref()
            .is_some_and(|mf| workspace::validate_auth_identity(&ws, &a.login, mf).is_err());
        let session_state = mf
            .as_ref()
            .map(|mf| sda::session_state::classify(mf, &a.login, now))
            .unwrap_or(sda::session_state::SessionState::NoSession);
        let meta = workspace::read_meta(&a.path);
        let account_name = mf
            .as_ref()
            .map(|mf| mf.account_name.clone())
            .or(meta.authenticator_account_name);
        let steam_id = mf
            .as_ref()
            .and_then(|mf| mf.session.as_ref().map(|s| s.steam_id.to_string()))
            .or(a.steam_id.clone());
        let recovery_exists = workspace::auth_dir(&ws, &a.login)?.join("enrollment.dpapi").is_file();
        let enrollment = mf
            .as_ref()
            .map(|mf| {
                if mf.fully_enrolled == Some(false) {
                    "pending"
                } else if mf.recovery_pending {
                    "recovery"
                } else {
                    "none"
                }
            })
            .unwrap_or(if recovery_exists { "pending" } else { "none" });
        out.push(AccountAuthStatus {
            login: a.login.clone(),
            has_authenticator: has && a.has_authenticator,
            has_saved_password: sda::credentials::has_saved(&ws, &a.login),
            account_name,
            steam_id,
            enrollment,
            identity_mismatch: mismatch,
            auto_login: sda::credentials::status(&ws, &a.login),
            imported_at: a.authenticator_imported_at.clone(),
            session_state,
        });
    }
    Ok(out)
}

/// Return the current SessionState for a single account. Cheap; safe to
/// poll on UI mount or after operations that may have changed the session.
#[tauri::command(async)]
pub fn auth_session_state(
    login: String,
    expected_workspace: Option<PathBuf>,
) -> AppResult<sda::session_state::SessionState> {
    let s = settings::load()?;
    let ws = require_workspace(&s, expected_workspace.as_deref())?;
    let now = sda::session_state::now_secs();
    match sda::vault::load_plain(&ws, &login)? {
        Some(mf) => Ok(sda::session_state::classify(&mf, &login, now)),
        None => Ok(sda::session_state::SessionState::NoSession),
    }
}

/// Import a `.maFile` into the given shadow account. Accepts either a JSON
/// body string or a filesystem path (we pick by checking path existence).
/// For SDA-encrypted imports pass the unlock password as `encryption_password`.
#[tauri::command(async)]
pub fn auth_import_mafile(
    login: String,
    source: String,
    encryption_password: Option<String>,
    expected_workspace: Option<PathBuf>,
) -> AppResult<AccountAuthStatus> {
    let s = settings::load()?;
    let ws = require_workspace(&s, expected_workspace.as_deref())?;
    let gate = sda::session::account_gate(&ws, &login)?;
    let _guard = gate.lock().unwrap();
    // Ensure the shadow account exists up-front — we want a clear error,
    // not a silent dir creation, if the user typo'd the login.
    let dir = workspace::checked_account_dir(&ws, &login)?;
    if !dir.exists() {
        return Err(AppError::NotFound(format!("account {login}")));
    }

    // Distinguish path vs inline JSON: a path-ish string with an existing file.
    let trimmed = source.trim();
    let as_path = std::path::Path::new(trimmed);
    let bytes: Vec<u8> = if !trimmed.starts_with('{') && as_path.exists() && as_path.is_file() {
        std::fs::read(as_path)?
    } else if trimmed.starts_with('{') {
        trimmed.as_bytes().to_vec()
    } else {
        return Err(AppError::Other(format!(
            "MAFILE_NOT_FOUND: {}",
            as_path.display()
        )));
    };

    // Detect SDA-encrypted: its top-level JSON has `"Encrypted": true` with
    // `encryption_iv` / `encryption_salt` siblings. (Manifest-free import
    // path — user just points at the single file.)
    let mf = match serde_json::from_slice::<serde_json::Value>(&bytes) {
        Ok(v)
            if v.get("Encrypted")
                .and_then(|e| e.as_bool())
                .unwrap_or(false)
                || v.get("encryption_iv").is_some() =>
        {
            let pw = encryption_password
                .ok_or_else(|| AppError::NotReady("MAFILE_NEEDS_PASSWORD".into()))?;
            let iv = v
                .get("encryption_iv")
                .and_then(|s| s.as_str())
                .ok_or_else(|| AppError::Other("MAFILE_NO_IV".into()))?;
            let salt = v
                .get("encryption_salt")
                .and_then(|s| s.as_str())
                .ok_or_else(|| AppError::Other("MAFILE_NO_SALT".into()))?;
            // SDA wraps ciphertext in a field or the whole file. We expect the
            // `EncryptedData` / `encrypted` field to hold base64 of AES-CBC.
            let ct = v
                .get("EncryptedData")
                .or_else(|| v.get("encrypted_data"))
                .or_else(|| v.get("encrypted"))
                .and_then(|s| s.as_str())
                .ok_or_else(|| AppError::Other("MAFILE_NO_BODY".into()))?;
            let plain = sda::crypto::sda_decrypt(&pw, salt, iv, ct)?;
            sda::mafile::MaFile::from_json_bytes(&plain)?
        }
        _ => sda::mafile::MaFile::from_json_bytes(&bytes)?,
    };

    workspace::validate_auth_identity(&ws, &login, &mf)?;
    sda::vault::save_plain(&ws, &login, &mf)?;
    workspace::set_authenticator_meta(
        &ws,
        &login,
        mf.fully_enrolled != Some(false),
        Some(mf.account_name.clone()),
    )?;
    tracing::info!("sda: imported maFile for login={}", login);
    let now = sda::session_state::now_secs();
    let session_state = sda::session_state::classify(&mf, &login, now);
    Ok(AccountAuthStatus {
        login: login.clone(),
        has_authenticator: true,
        has_saved_password: sda::credentials::has_saved(&ws, &login),
        account_name: Some(mf.account_name.clone()),
        steam_id: mf.session.as_ref().map(|s| s.steam_id.to_string()),
        enrollment: if mf.fully_enrolled == Some(false) {
            "pending"
        } else if mf.recovery_pending {
            "recovery"
        } else {
            "none"
        },
        identity_mismatch: false,
        auto_login: sda::credentials::status(&ws, &login),
        imported_at: Some(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs().to_string())
                .unwrap_or_default(),
        ),
        session_state,
    })
}

/// Export a `.maFile` for the given login to the chosen path (plain JSON,
/// SDA-compatible).
#[tauri::command(async)]
pub fn auth_export_mafile(
    login: String,
    target_path: PathBuf,
    expected_workspace: Option<PathBuf>,
) -> AppResult<()> {
    let s = settings::load()?;
    let ws = require_workspace(&s, expected_workspace.as_deref())?;
    let mf = sda::vault::load_plain(&ws, &login)?
        .ok_or_else(|| AppError::NotFound(format!("no maFile for {login}")))?;
    if let Some(parent) = target_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&target_path, mf.to_json_pretty()?)?;
    Ok(())
}

/// Wipe the authenticator data for a shadow account.
#[tauri::command(async)]
pub fn auth_remove(login: String, expected_workspace: Option<PathBuf>) -> AppResult<()> {
    let s = settings::load()?;
    let ws = require_workspace(&s, expected_workspace.as_deref())?;
    let gate = sda::session::account_gate(&ws, &login)?;
    let _guard = gate.lock().unwrap();
    sda::reauth::forget(&ws, &login)?;
    sda::vault::remove(&ws, &login)?;
    let recovery = workspace::auth_dir(&ws, &login)?.join("enrollment.dpapi");
    if recovery.exists() {
        std::fs::remove_file(recovery)?;
    }
    sda::add::drop_session(&login);
    workspace::set_authenticator_meta(&ws, &login, false, None)?;
    Ok(())
}

/// Generate the current Steam Guard code. Cheap enough to call every second
/// from the UI — the time-offset cache avoids network traffic.
#[tauri::command(async)]
pub fn auth_generate_code(
    login: String,
    expected_workspace: Option<PathBuf>,
) -> AppResult<GuardCode> {
    let s = settings::load()?;
    let ws = require_workspace(&s, expected_workspace.as_deref())?;
    let mf = sda::vault::load_plain(&ws, &login)?
        .ok_or_else(|| AppError::NotFound(format!("no maFile for {login}")))?;
    workspace::validate_auth_identity(&ws, &login, &mf)?;
    let (code, at) = sda::totp::generate_code_now(&mf.shared_secret)?;
    let remaining = 30 - (at % 30);
    Ok(GuardCode {
        code,
        generated_at: sda::session_state::now_secs(),
        period_remaining: remaining,
    })
}

/// Force-refresh the Steam server-time offset.
#[tauri::command(async)]
pub fn auth_sync_time() -> AppResult<()> {
    sda::totp::sync_time()
}

/// Fetch the current list of pending mobile confirmations for this account.
#[tauri::command]
pub async fn auth_confirmations_list(
    login: String,
    expected_workspace: Option<PathBuf>,
) -> AppResult<Vec<sda::confirmations::Confirmation>> {
    tauri::async_runtime::spawn_blocking(move || {
        let s = settings::load()?;
        let ws = require_workspace(&s, expected_workspace.as_deref())?;
        sda::session::with_ready(&ws, &login, |mf| sda::confirmations::list(mf, &login))
    })
    .await
    .map_err(|e| AppError::Other(format!("Authenticator worker: {e}")))?
}

/// Fetch the item descriptions for a pending trade on the selected account.
#[tauri::command]
pub async fn auth_confirmation_details(
    login: String,
    id: String,
    expected_workspace: Option<PathBuf>,
) -> AppResult<sda::confirmations::TradeDetails> {
    tauri::async_runtime::spawn_blocking(move || {
        let s = settings::load()?;
        let ws = require_workspace(&s, expected_workspace.as_deref())?;
        sda::session::with_ready(&ws, &login, |mf| sda::confirmations::trade_details(mf, &login, &id))
    }).await.map_err(|e| AppError::Other(format!("Authenticator worker: {e}")))?
}

/// Allow or reject a batch of confirmations.
#[tauri::command]
pub async fn auth_confirmations_respond(
    login: String,
    ids: Vec<String>,
    op: String,
    expected_workspace: Option<PathBuf>,
) -> AppResult<Vec<sda::confirmations::RespondResult>> {
    tauri::async_runtime::spawn_blocking(move || {
        let s = settings::load()?;
        let ws = require_workspace(&s, expected_workspace.as_deref())?;
        let op = match op.as_str() {
            "allow" => sda::confirmations::Op::Allow,
            "reject" | "cancel" | "deny" => sda::confirmations::Op::Reject,
            _ => return Err(AppError::Other(format!("unknown op: {op}"))),
        };
        sda::session::with_ready(&ws, &login, |mf| {
            sda::confirmations::respond(mf, &login, &ids, op)
        })
    })
    .await
    .map_err(|e| AppError::Other(format!("Authenticator worker: {e}")))?
}

/// Step 1+2 of mobile login: fetch RSA key, encrypt password, open session.
/// Returns { clientId, requestId, steamId, allowedConfirmations, interval }.
#[tauri::command]
pub async fn auth_login_begin(
    login: String,
    account_name: String,
    password: String,
    remember_password: Option<bool>,
    expected_workspace: Option<PathBuf>,
) -> AppResult<sda::login::BeginOutcome> {
    let password = zeroize::Zeroizing::new(password);
    tauri::async_runtime::spawn_blocking(move || {
        let s = settings::load()?;
        let ws = require_workspace(&s, expected_workspace.as_deref())?;
        sda::reauth::begin_manual(
            &ws,
            &login,
            &account_name,
            password,
            remember_password.unwrap_or(false),
        )
    })
    .await
    .map_err(|e| AppError::Other(format!("Login worker: {e}")))?
}

#[tauri::command(async)]
pub fn auth_login_cancel(client_id: String) {
    sda::reauth::cancel(&client_id);
}

#[tauri::command(async)]
pub fn auth_password_forget(login: String, expected_workspace: Option<PathBuf>) -> AppResult<()> {
    let s = settings::load()?;
    let ws = require_workspace(&s, expected_workspace.as_deref())?;
    sda::reauth::forget(&ws, &login)
}

/// Step 3: submit a Steam Guard code (device-code or email-code) to the
/// currently-open auth session.
#[tauri::command]
pub async fn auth_login_submit_code(
    client_id: String,
    steam_id: String,
    code: String,
    code_type: i64,
) -> AppResult<()> {
    tauri::async_runtime::spawn_blocking(move || {
        sda::login::submit_code(&client_id, &steam_id, &code, code_type)
    })
    .await
    .map_err(|e| AppError::Other(format!("Login worker: {e}")))?
}

/// Step 4: poll for tokens. Returns a state enum; Done = tokens have been
/// stored into the maFile and UI can close.
///
/// `allowed_confirmations` is the list of `confirmation_type` values from the
/// matching `auth_login_begin` call. We persist it into the AddSession so the
/// AddAuthenticator wizard can use it as the authoritative "is Email Guard on?"
/// signal (QueryStatus only reports mobile-authenticator state, it can't tell
/// "Email Guard on" apart from "no Guard").
#[tauri::command]
pub async fn auth_login_poll(
    login: String,
    client_id: String,
    request_id: String,
    allowed_confirmations: Option<Vec<i64>>,
    expected_workspace: Option<PathBuf>,
) -> AppResult<sda::login::PollState> {
    tauri::async_runtime::spawn_blocking(move || {
        let s = settings::load()?;
        let ws = require_workspace(&s, expected_workspace.as_deref())?;
        sda::reauth::validate_pending(&ws, &login, &client_id, &request_id)?;
        let state = sda::login::poll(&client_id, &request_id)?;
        sda::reauth::complete_manual(
            &ws,
            &login,
            &client_id,
            &state,
            allowed_confirmations.unwrap_or_default(),
        )?;
        if matches!(&state, sda::login::PollState::Failed { .. }) {
            sda::reauth::cancel(&client_id);
        }
        Ok(state)
    })
    .await
    .map_err(|e| AppError::Other(format!("Login worker: {e}")))?
}

/// Force-refresh the access_token for an account (no re-login needed as long
/// as refresh_token is still valid, which is ~200 days).
#[tauri::command]
pub async fn auth_login_refresh(
    login: String,
    expected_workspace: Option<PathBuf>,
) -> AppResult<()> {
    tauri::async_runtime::spawn_blocking(move || {
        let s = settings::load()?;
        let ws = require_workspace(&s, expected_workspace.as_deref())?;
        sda::session::with_ready(&ws, &login, |_| Ok(()))
    })
    .await
    .map_err(|e| AppError::Other(format!("Authenticator worker: {e}")))?
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthLockStatus {
    pub enabled: bool,
    pub unlocked: bool,
    /// True iff at least one encrypted maFile exists on disk.
    #[serde(rename = "hasEncryptedFiles")]
    pub has_encrypted_files: bool,
}

/// Report whether a master password is currently configured and/or unlocked.
#[tauri::command(async)]
pub fn auth_lock_status() -> AppResult<AuthLockStatus> {
    let s = settings::load()?;
    let has_encrypted = match s.workspace.as_ref() {
        Some(ws) => workspace::list_accounts(ws)
            .ok()
            .map(|accs| {
                accs.iter().any(|a| {
                    sda::vault::mafile_enc_path(ws, &a.login)
                        .map(|p| p.exists())
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false),
        None => false,
    };
    Ok(AuthLockStatus {
        enabled: s
            .workspace
            .as_ref()
            .map(|ws| sda::vault::initialize(ws, s.auth_master_password_enabled))
            .transpose()?
            .unwrap_or(false),
        unlocked: s
            .workspace
            .as_ref()
            .is_some_and(|ws| sda::vault::has_master_password_unlocked(ws)),
        has_encrypted_files: has_encrypted,
    })
}

/// Submit the master password for the current session. Fails with
/// `VAULT_BAD_PASSWORD` if it doesn't decrypt the first maFile.enc found.
#[tauri::command(async)]
pub fn auth_unlock(password: String, expected_workspace: Option<PathBuf>) -> AppResult<()> {
    let s = settings::load()?;
    let ws = require_workspace(&s, expected_workspace.as_deref())?;
    sda::vault::unlock(&ws, password)
}

/// Drop the master password from memory (re-lock). Files stay encrypted.
#[tauri::command(async)]
pub fn auth_lock() -> AppResult<()> {
    sda::vault::lock();
    Ok(())
}

/// Enable / change / disable the master password. `new_password = None`
/// disables encryption entirely (all files are rewritten as plain JSON).
#[tauri::command(async)]
pub fn auth_set_master_password(
    old_password: Option<String>,
    new_password: Option<String>,
    expected_workspace: Option<PathBuf>,
) -> AppResult<()> {
    let s = settings::load()?;
    let ws = require_workspace(&s, expected_workspace.as_deref())?;

    sda::vault::rekey_all(&ws, old_password.as_deref(), new_password.as_deref())?;
    settings::update(|current| current.auth_master_password_enabled = new_password.is_some())?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct PollerConfig {
    pub enabled: bool,
    pub interval: u32,
    #[serde(rename = "autoConfirmTrades")]
    pub auto_confirm_trades: bool,
    #[serde(rename = "autoConfirmMarket")]
    pub auto_confirm_market: bool,
}

/// Apply poller configuration. Persists to settings, then kicks the poller
/// so the new state takes effect immediately (instead of waiting up to N
/// seconds for the current sleep to end).
#[tauri::command(async)]
pub fn auth_poller_configure(cfg: PollerConfig) -> AppResult<()> {
    settings::update(|s| {
        s.auth_poller_enabled = cfg.enabled;
        s.auth_poller_interval = cfg.interval.clamp(15, 600);
        s.auth_auto_confirm_trades = cfg.auto_confirm_trades;
        s.auth_auto_confirm_market = cfg.auto_confirm_market;
    })?;
    sda::poller::poke();
    Ok(())
}

#[tauri::command(async)]
pub fn auth_poller_poke(app: AppHandle) -> AppResult<()> {
    sda::poller::check_now(&app)
}

// ── P12: AddAuthenticator wizard ──────────────────────────────────────────

/// Diagnose post-login state so the wizard can pick the right path without
/// guess-and-check. Combines QueryStatus + AccountPhoneStatus.
#[tauri::command(async)]
pub fn auth_add_diagnose(
    login: String,
    expected_workspace: Option<PathBuf>,
) -> AppResult<sda::add::AddDiagnostic> {
    let s = settings::load()?;
    let ws = require_workspace(&s, expected_workspace.as_deref())?;
    let gate = sda::session::account_gate(&ws, &login)?;
    let _guard = gate.lock().unwrap();
    sda::add::diagnose(&login)
}

/// Phase A.1: attach a phone number to the account. Steam will email the
/// account owner to confirm; the UI polls `auth_add_check_email` afterwards.
#[tauri::command(async)]
pub fn auth_add_set_phone(
    login: String,
    phone_number: String,
    phone_country_code: String,
    expected_workspace: Option<PathBuf>,
) -> AppResult<sda::add::SetPhoneResult> {
    let s = settings::load()?;
    let ws = require_workspace(&s, expected_workspace.as_deref())?;
    let gate = sda::session::account_gate(&ws, &login)?;
    let _guard = gate.lock().unwrap();
    sda::add::add_set_phone(&login, &phone_number, &phone_country_code)
}

/// Phase A.2: poll whether we're still waiting on the email-confirmation step.
#[tauri::command(async)]
pub fn auth_add_check_email(
    login: String,
    expected_workspace: Option<PathBuf>,
) -> AppResult<sda::add::PhoneState> {
    let s = settings::load()?;
    let ws = require_workspace(&s, expected_workspace.as_deref())?;
    let gate = sda::session::account_gate(&ws, &login)?;
    let _guard = gate.lock().unwrap();
    sda::add::add_check_email(&login)
}

/// Phase A.3: ask Steam to send the SMS verification code.
#[tauri::command(async)]
pub fn auth_add_send_sms(login: String, expected_workspace: Option<PathBuf>) -> AppResult<()> {
    let s = settings::load()?;
    let ws = require_workspace(&s, expected_workspace.as_deref())?;
    let gate = sda::session::account_gate(&ws, &login)?;
    let _guard = gate.lock().unwrap();
    sda::add::add_send_sms(&login)
}

/// Phase A.4: submit the SMS code to verify the phone number.
#[tauri::command(async)]
pub fn auth_add_verify_phone(
    login: String,
    code: String,
    expected_workspace: Option<PathBuf>,
) -> AppResult<()> {
    let s = settings::load()?;
    let ws = require_workspace(&s, expected_workspace.as_deref())?;
    let gate = sda::session::account_gate(&ws, &login)?;
    let _guard = gate.lock().unwrap();
    sda::add::add_verify_phone(&login, &code)
}

/// Phase B: ask Steam to issue a fresh authenticator. Returns the phone hint;
/// secrets stay in the Rust process until `auth_add_finalize` succeeds.
///
/// **Data-loss safety**: as soon as `add_create` returns, Steam has bound a
/// pending authenticator on its side and we hold the *only* copy of the
/// shared/identity secrets and revocation code. If the wizard dies before
/// `auth_add_persist` runs we'd lock the user out forever. So we eagerly
/// stash a `fully_enrolled=false` maFile immediately, which the persist step
/// later overwrites with the activated record.
#[tauri::command(async)]
pub fn auth_add_create(
    login: String,
    expected_workspace: Option<PathBuf>,
) -> AppResult<sda::add::AddCreatePublic> {
    let s = settings::load()?;
    let ws = require_workspace(&s, expected_workspace.as_deref())?;
    sda::vault::ensure_writable(&ws)?;
    let gate = sda::session::account_gate(&ws, &login)?;
    let _guard = gate.lock().unwrap();
    if let Some(existing) = sda::vault::load_plain(&ws, &login)? {
        if existing.fully_enrolled != Some(false) { return Err(AppError::NotReady("ADD_AUTH_ALREADY_HAS_AUTHENTICATOR".into())); }
        sda::add::resume(&ws, &login)?;
    }
    let r = sda::add::add_create(&login)?;
    sda::add::add_persist_partial(&login, &ws)?;
    Ok(r)
}

/// Phase C: Finalize. Surface back the revocation code to the UI ONCE so the
/// user can copy/screenshot it before we persist anything to disk.
#[tauri::command(async)]
pub fn auth_add_finalize(
    login: String,
    sms_code: String,
    try_number: u32,
    validate_sms: bool,
    expected_workspace: Option<PathBuf>,
) -> AppResult<sda::add::AddFinalizePublic> {
    let s = settings::load()?;
    let ws = require_workspace(&s, expected_workspace.as_deref())?;
    sda::vault::ensure_writable(&ws)?;
    let gate = sda::session::account_gate(&ws, &login)?;
    let _guard = gate.lock().unwrap();
    let r = sda::add::add_finalize(&login, &sms_code, try_number, validate_sms)?;
    if r.success {
        sda::add::add_persist(&login, &ws)?;
    }
    Ok(r)
}

/// Final commit: write the maFile to disk + update workspace meta. The UI
/// MUST gate this on the user explicitly confirming they wrote down the
/// revocation code.
#[tauri::command(async)]
pub fn auth_add_persist(login: String, expected_workspace: Option<PathBuf>) -> AppResult<()> {
    let s = settings::load()?;
    let ws = require_workspace(&s, expected_workspace.as_deref())?;
    let gate = sda::session::account_gate(&ws, &login)?;
    let _guard = gate.lock().unwrap();
    sda::add::acknowledge(&ws, &login)?;
    sda::poller::poke();
    Ok(())
}

/// Abort the wizard, drop in-memory secrets without touching disk.
#[tauri::command(async)]
pub fn auth_add_cancel(login: String, expected_workspace: Option<PathBuf>) -> AppResult<()> {
    let s = settings::load()?;
    let ws = require_workspace(&s, expected_workspace.as_deref())?;
    let gate = sda::session::account_gate(&ws, &login)?;
    let _guard = gate.lock().unwrap();
    sda::add::cancel(&ws, &login)
}

#[tauri::command(async)]
pub fn auth_add_resume(
    login: String,
    expected_workspace: Option<PathBuf>,
) -> AppResult<sda::add::Resume> {
    let s = settings::load()?;
    let ws = require_workspace(&s, expected_workspace.as_deref())?;
    let gate = sda::session::account_gate(&ws, &login)?;
    let _guard = gate.lock().unwrap();
    sda::add::resume(&ws, &login)
}

#[tauri::command(async)]
pub fn recover_settings() -> AppResult<Settings> {
    let _exclusive = crate::lifecycle::write();
    settings::recover_settings()
}

#[tauri::command(async)]
pub fn cleanup_backups(expected_workspace: Option<PathBuf>) -> AppResult<crate::backups::Cleanup> {
    let s = settings::load()?;
    let ws = require_workspace(&s, expected_workspace.as_deref())?;
    crate::backups::maintain(&ws)
}
