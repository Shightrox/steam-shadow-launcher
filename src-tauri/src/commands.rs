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

fn require_workspace(s: &Settings) -> AppResult<PathBuf> {
    s.workspace
        .clone()
        .ok_or_else(|| AppError::Workspace("workspace not configured".into()))
}

fn require_main(s: &Settings) -> AppResult<MainSteamInfo> {
    steam_paths::detect(s.main_steam_path_override.clone())
}

#[tauri::command]
pub fn detect_main_steam() -> AppResult<MainSteamInfo> {
    let s = settings::load()?;
    steam_paths::detect(s.main_steam_path_override)
}

#[tauri::command]
pub fn get_settings() -> AppResult<Settings> {
    settings::load()
}

#[tauri::command]
pub fn save_settings(settings: Settings) -> AppResult<()> {
    settings::save(&settings)
}

#[tauri::command]
pub fn list_accounts() -> AppResult<Vec<Account>> {
    let s = settings::load()?;
    let ws = require_workspace(&s)?;
    workspace::list_accounts(&ws)
}

#[tauri::command]
pub fn add_account(login: String, display: Option<String>) -> AppResult<Account> {
    let s = settings::load()?;
    let ws = require_workspace(&s)?;
    let main = require_main(&s)?;
    workspace::add_account(&ws, &main.steamapps_dir, &login, display)
}

#[tauri::command]
pub fn remove_account(login: String, delete_files: bool) -> AppResult<()> {
    let s = settings::load()?;
    let ws = require_workspace(&s)?;
    // also try to remove sandbox box
    let sb = sandboxie::detect();
    if sb.installed {
        let _ = sandboxie::remove_box(&sb, &login);
    }
    workspace::remove_account(&ws, &login, delete_files)
}

#[tauri::command]
pub fn verify_account(login: String) -> AppResult<AccountHealth> {
    let s = settings::load()?;
    let ws = require_workspace(&s)?;
    let main = require_main(&s)?;
    let dir = workspace::account_dir(&ws, &login);
    vdf::is_account_ready(&dir, &main.steamapps_dir)
}

#[tauri::command]
pub fn repair_account(login: String) -> AppResult<()> {
    let s = settings::load()?;
    let ws = require_workspace(&s)?;
    let main = require_main(&s)?;
    let dir = workspace::ensure_account_dirs(&ws, &login)?;
    let link = dir.join("steamapps");
    junction::repair(&link, &main.steamapps_dir)?;
    Ok(())
}

#[tauri::command]
pub fn launch_shadow(login: String, mode: Option<String>) -> AppResult<LaunchOutcome> {
    let s = settings::load()?;
    let ws = require_workspace(&s)?;
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

#[tauri::command]
pub fn change_workspace(new_path: PathBuf, strategy: String) -> AppResult<()> {
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
        if old != new_path {
            workspace::change_workspace(&old, &new_path, strat, &main.steamapps_dir)?;
        }
    }
    s.workspace = Some(new_path);
    settings::save(&s)?;
    Ok(())
}

#[tauri::command]
pub fn set_workspace_initial(new_path: PathBuf) -> AppResult<()> {
    let mut s = settings::load()?;
    let main = require_main(&s)?;
    workspace::validate_workspace(&new_path, &main.install_dir)?;
    s.workspace = Some(new_path);
    s.first_run_completed = true;
    settings::save(&s)
}

#[tauri::command]
pub fn set_main_steam_override(new_path: Option<PathBuf>) -> AppResult<()> {
    let mut s = settings::load()?;
    s.main_steam_path_override = new_path;
    settings::save(&s)
}

#[tauri::command]
pub fn cleanup_stale_junctions() -> AppResult<CleanupReport> {
    let s = settings::load()?;
    let ws = require_workspace(&s)?;
    let main = require_main(&s)?;
    junction::cleanup_stale(&ws, &main.steamapps_dir)
}

#[tauri::command]
pub fn discover_steam_accounts() -> AppResult<Vec<DiscoveredAccount>> {
    let s = settings::load()?;
    let main = require_main(&s)?;
    vdf::parse_loginusers(&main.install_dir)
}

#[tauri::command]
pub fn import_discovered_accounts(
    logins: Vec<String>,
    personas: std::collections::HashMap<String, String>,
) -> AppResult<Vec<Account>> {
    let s = settings::load()?;
    let ws = require_workspace(&s)?;
    let main = require_main(&s)?;
    let mut out = Vec::new();
    for login in logins {
        let display = personas.get(&login).cloned();
        let acc = workspace::add_account(&ws, &main.steamapps_dir, &login, display)?;
        out.push(acc);
    }
    Ok(out)
}

#[tauri::command]
pub fn default_workspace() -> AppResult<Option<PathBuf>> {
    Ok(settings::default_workspace_path())
}

#[tauri::command]
pub fn detect_sandboxie() -> AppResult<SandboxieInfo> {
    Ok(sandboxie::detect())
}

#[tauri::command]
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

fn emit_progress(app: &AppHandle, phase: &'static str, downloaded: u64, total: Option<u64>, name: Option<String>) {
    let percent = total.map(|t| if t == 0 { 0 } else { ((downloaded * 100) / t).min(100) as u32 });
    let _ = app.emit(
        "sandboxie-download-progress",
        ProgressEvent { phase, downloaded, total, percent, name },
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

#[tauri::command]
pub fn list_running_games() -> AppResult<Vec<RunningGame>> {
    let s = settings::load()?;
    let main = require_main(&s)?;
    Ok(steam_process::find_running_games(&main.steamapps_dir))
}

#[tauri::command]
pub fn revert_last_switch() -> AppResult<()> {
    let s = settings::load()?;
    let ws = require_workspace(&s)?;
    let main = require_main(&s)?;
    switcher::revert_last(&ws, &main)
}

#[tauri::command]
pub fn close_window(app: tauri::AppHandle) -> AppResult<()> {
    use tauri::Manager;
    if let Some(w) = app.get_webview_window("main") {
        w.close()
            .map_err(|e| AppError::Other(format!("close: {e}")))?;
    }
    Ok(())
}

#[tauri::command]
pub fn minimize_window(app: tauri::AppHandle) -> AppResult<()> {
    use tauri::Manager;
    if let Some(w) = app.get_webview_window("main") {
        w.minimize()
            .map_err(|e| AppError::Other(format!("minimize: {e}")))?;
    }
    Ok(())
}

#[tauri::command]
pub fn start_drag(app: tauri::AppHandle) -> AppResult<()> {
    use tauri::Manager;
    if let Some(w) = app.get_webview_window("main") {
        w.start_dragging()
            .map_err(|e| AppError::Other(format!("drag: {e}")))?;
    }
    Ok(())
}

#[tauri::command]
pub fn is_elevated() -> AppResult<bool> {
    Ok(sandboxie::is_elevated_pub())
}

#[tauri::command]
pub fn relaunch_as_admin(app: AppHandle) -> AppResult<()> {
    sandboxie::relaunch_self_as_admin()?;
    // Give the new process a moment to spawn before we exit.
    std::thread::sleep(std::time::Duration::from_millis(300));
    app.exit(0);
    Ok(())
}

#[tauri::command]
pub fn set_account_favorite(login: String, value: bool) -> AppResult<()> {
    let s = settings::load()?;
    let ws = require_workspace(&s)?;
    workspace::set_favorite(&ws, &login, value)
}

#[tauri::command]
pub fn refresh_account_avatar(login: String) -> AppResult<Option<PathBuf>> {
    let s = settings::load()?;
    let ws = require_workspace(&s)?;
    workspace::refresh_avatar(&ws, &login)
}

#[tauri::command]
pub fn list_running_sandboxes() -> AppResult<Vec<sandboxie::RunningSandbox>> {
    let info = sandboxie::detect();
    if !info.installed {
        return Ok(vec![]);
    }
    Ok(sandboxie::list_running(&info))
}

#[tauri::command]
pub fn stop_sandbox(login: String) -> AppResult<()> {
    let info = sandboxie::detect();
    if !info.installed {
        return Err(AppError::NotReady("Sandboxie not installed".into()));
    }
    sandboxie::stop_box(&info, &login)
}

#[tauri::command]
pub fn list_account_games(_login: String) -> AppResult<Vec<InstalledGame>> {
    // NOTE: The `_login` parameter is kept for forward compatibility — later
    // we may filter by owned-apps once we wire up the Steam Web API. Right
    // now we return every app installed in any registered Steam library on
    // this machine, which is a superset of what any account can launch.
    let s = settings::load()?;
    let main = require_main(&s)?;
    Ok(library::list_installed_games(&main))
}

#[tauri::command]
pub fn launch_game(
    login: String,
    appid: u32,
    mode: Option<String>,
) -> AppResult<LaunchOutcome> {
    let s = settings::load()?;
    let ws = require_workspace(&s)?;
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

#[tauri::command]
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

#[tauri::command]
pub fn create_account_shortcut(login: String) -> AppResult<PathBuf> {
    shortcut::create_desktop_shortcut(&login)
}

/// Reveal an account's auth folder (where maFile.json / maFile.enc lives) in
/// the system file explorer. Returns the resolved path so the caller can show
/// it to the user even if launching Explorer fails.
#[tauri::command]
pub fn auth_open_folder(login: String) -> AppResult<PathBuf> {
    let s = settings::load()?;
    let ws = require_workspace(&s)?;
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
    #[serde(rename = "accountName")]
    pub account_name: Option<String>,
    #[serde(rename = "importedAt")]
    pub imported_at: Option<String>,
}

/// Return per-account authenticator presence. Does NOT load shared_secret.
#[tauri::command]
pub fn auth_status() -> AppResult<Vec<AccountAuthStatus>> {
    let s = settings::load()?;
    let ws = require_workspace(&s)?;
    let accounts = workspace::list_accounts(&ws)?;
    let mut out = Vec::with_capacity(accounts.len());
    for a in accounts {
        let has = sda::vault::has_any(&ws, &a.login);
        out.push(AccountAuthStatus {
            login: a.login.clone(),
            has_authenticator: has && a.has_authenticator,
            account_name: if has { Some(a.login.clone()) } else { None },
            imported_at: a.authenticator_imported_at.clone(),
        });
    }
    Ok(out)
}

/// Import a `.maFile` into the given shadow account. Accepts either a JSON
/// body string or a filesystem path (we pick by checking path existence).
/// For SDA-encrypted imports pass the unlock password as `encryption_password`.
#[tauri::command]
pub fn auth_import_mafile(
    login: String,
    source: String,
    encryption_password: Option<String>,
) -> AppResult<AccountAuthStatus> {
    let s = settings::load()?;
    let ws = require_workspace(&s)?;
    // Ensure the shadow account exists up-front — we want a clear error,
    // not a silent dir creation, if the user typo'd the login.
    let dir = workspace::account_dir(&ws, &login);
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
        Ok(v) if v.get("Encrypted").and_then(|e| e.as_bool()).unwrap_or(false)
              || v.get("encryption_iv").is_some() =>
        {
            let pw = encryption_password
                .ok_or_else(|| AppError::NotReady("MAFILE_NEEDS_PASSWORD".into()))?;
            let iv = v.get("encryption_iv")
                .and_then(|s| s.as_str())
                .ok_or_else(|| AppError::Other("MAFILE_NO_IV".into()))?;
            let salt = v.get("encryption_salt")
                .and_then(|s| s.as_str())
                .ok_or_else(|| AppError::Other("MAFILE_NO_SALT".into()))?;
            // SDA wraps ciphertext in a field or the whole file. We expect the
            // `EncryptedData` / `encrypted` field to hold base64 of AES-CBC.
            let ct = v.get("EncryptedData")
                .or_else(|| v.get("encrypted_data"))
                .or_else(|| v.get("encrypted"))
                .and_then(|s| s.as_str())
                .ok_or_else(|| AppError::Other("MAFILE_NO_BODY".into()))?;
            let plain = sda::crypto::sda_decrypt(&pw, salt, iv, ct)?;
            sda::mafile::MaFile::from_json_bytes(&plain)?
        }
        _ => sda::mafile::MaFile::from_json_bytes(&bytes)?,
    };

    sda::vault::save_plain(&ws, &login, &mf)?;
    workspace::set_authenticator_meta(&ws, &login, true, Some(mf.account_name.clone()))?;
    tracing::info!("sda: imported maFile for login={}", login);
    Ok(AccountAuthStatus {
        login: login.clone(),
        has_authenticator: true,
        account_name: Some(mf.account_name.clone()),
        imported_at: Some(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs().to_string())
                .unwrap_or_default(),
        ),
    })
}

/// Export a `.maFile` for the given login to the chosen path (plain JSON,
/// SDA-compatible).
#[tauri::command]
pub fn auth_export_mafile(login: String, target_path: PathBuf) -> AppResult<()> {
    let s = settings::load()?;
    let ws = require_workspace(&s)?;
    let mf = sda::vault::load_plain(&ws, &login)?
        .ok_or_else(|| AppError::NotFound(format!("no maFile for {login}")))?;
    if let Some(parent) = target_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&target_path, mf.to_json_pretty()?)?;
    Ok(())
}

/// Wipe the authenticator data for a shadow account.
#[tauri::command]
pub fn auth_remove(login: String) -> AppResult<()> {
    let s = settings::load()?;
    let ws = require_workspace(&s)?;
    sda::vault::remove(&ws, &login)?;
    workspace::set_authenticator_meta(&ws, &login, false, None)?;
    Ok(())
}

/// Generate the current Steam Guard code. Cheap enough to call every second
/// from the UI — the time-offset cache avoids network traffic.
#[tauri::command]
pub fn auth_generate_code(login: String) -> AppResult<GuardCode> {
    let s = settings::load()?;
    let ws = require_workspace(&s)?;
    let mf = sda::vault::load_plain(&ws, &login)?
        .ok_or_else(|| AppError::NotFound(format!("no maFile for {login}")))?;
    let (code, at) = sda::totp::generate_code_now(&mf.shared_secret)?;
    let remaining = 30 - (at % 30);
    Ok(GuardCode {
        code,
        generated_at: at,
        period_remaining: remaining,
    })
}

/// Force-refresh the Steam server-time offset.
#[tauri::command]
pub fn auth_sync_time() -> AppResult<()> {
    sda::totp::sync_time()
}

/// Fetch the current list of pending mobile confirmations for this account.
#[tauri::command]
pub fn auth_confirmations_list(login: String) -> AppResult<Vec<sda::confirmations::Confirmation>> {
    let s = settings::load()?;
    let ws = require_workspace(&s)?;
    let mf = sda::vault::load_plain(&ws, &login)?
        .ok_or_else(|| AppError::NotFound(format!("no maFile for {login}")))?;
    sda::confirmations::list(&mf)
}

/// Allow or reject a batch of confirmations.
#[tauri::command]
pub fn auth_confirmations_respond(
    login: String,
    ids: Vec<String>,
    op: String,
) -> AppResult<Vec<sda::confirmations::RespondResult>> {
    let s = settings::load()?;
    let ws = require_workspace(&s)?;
    let mf = sda::vault::load_plain(&ws, &login)?
        .ok_or_else(|| AppError::NotFound(format!("no maFile for {login}")))?;
    let op = match op.as_str() {
        "allow" => sda::confirmations::Op::Allow,
        "reject" | "cancel" | "deny" => sda::confirmations::Op::Reject,
        _ => return Err(AppError::Other(format!("unknown op: {op}"))),
    };
    sda::confirmations::respond(&mf, &ids, op)
}

/// Step 1+2 of mobile login: fetch RSA key, encrypt password, open session.
/// Returns { clientId, requestId, steamId, allowedConfirmations, interval }.
#[tauri::command]
pub fn auth_login_begin(
    account_name: String,
    password: String,
) -> AppResult<sda::login::BeginOutcome> {
    sda::login::begin(&account_name, &password)
}

/// Step 3: submit a Steam Guard code (device-code or email-code) to the
/// currently-open auth session.
#[tauri::command]
pub fn auth_login_submit_code(
    client_id: String,
    steam_id: String,
    code: String,
    code_type: i64,
) -> AppResult<()> {
    sda::login::submit_code(&client_id, &steam_id, &code, code_type)
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
pub fn auth_login_poll(
    login: String,
    client_id: String,
    request_id: String,
    allowed_confirmations: Option<Vec<i64>>,
) -> AppResult<sda::login::PollState> {
    let state = sda::login::poll(&client_id, &request_id)?;
    if let sda::login::PollState::Done {
        access_token,
        refresh_token,
        account_name,
        steam_id,
        ..
    } = &state
    {
        // Persist into the per-account maFile Session block. If the user has
        // no maFile yet (pure login with no pre-existing authenticator),
        // silently ignore — caller will surface it on first confirmation.
        let s = settings::load()?;
        let ws = require_workspace(&s)?;
        let sid = steam_id.parse::<u64>().unwrap_or(0);
        let session_id = random_session_id();
        // Always seed the add-authenticator registry so the "Add" wizard can
        // pick up where login left off without re-entering the password.
        sda::add::put_session(
            &login,
            access_token.clone(),
            refresh_token.clone(),
            sid,
            session_id.clone(),
            allowed_confirmations.clone().unwrap_or_default(),
        );
        if let Some(mut mf) = sda::vault::load_plain(&ws, &login)? {
            mf.session = Some(crate::sda::mafile::SessionData {
                steam_id: sid,
                access_token: access_token.clone(),
                refresh_token: refresh_token.clone(),
                session_id,
            });
            // If account_name arrived fresh, trust Steam over user input.
            if !account_name.is_empty() {
                mf.account_name = account_name.clone();
            }
            sda::vault::save_plain(&ws, &login, &mf)?;
            workspace::set_authenticator_meta(&ws, &login, true, Some(mf.account_name.clone()))?;
        }
    }
    Ok(state)
}

/// Force-refresh the access_token for an account (no re-login needed as long
/// as refresh_token is still valid, which is ~200 days).
#[tauri::command]
pub fn auth_login_refresh(login: String) -> AppResult<()> {
    let s = settings::load()?;
    let ws = require_workspace(&s)?;
    let mut mf = sda::vault::load_plain(&ws, &login)?
        .ok_or_else(|| AppError::NotFound(format!("no maFile for {login}")))?;
    let sess = mf
        .session
        .as_ref()
        .ok_or_else(|| AppError::NotReady("NO_SESSION".into()))?
        .clone();
    let new_tok = sda::login::refresh_access_token(&sess.refresh_token, &sess.steam_id.to_string())?;
    let mut updated = sess.clone();
    updated.access_token = new_tok;
    mf.session = Some(updated);
    sda::vault::save_plain(&ws, &login, &mf)?;
    Ok(())
}

fn random_session_id() -> String {
    use rand::RngCore;
    let mut buf = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut buf);
    // Steam's `sessionid` is 24-char hex in the mobile client.
    buf.iter().map(|b| format!("{:02x}", b)).collect()
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
#[tauri::command]
pub fn auth_lock_status() -> AppResult<AuthLockStatus> {
    let s = settings::load()?;
    let has_encrypted = match s.workspace.as_ref() {
        Some(ws) => workspace::list_accounts(ws)
            .ok()
            .map(|accs| {
                accs.iter()
                    .any(|a| sda::vault::mafile_enc_path(ws, &a.login)
                        .map(|p| p.exists())
                        .unwrap_or(false))
            })
            .unwrap_or(false),
        None => false,
    };
    Ok(AuthLockStatus {
        enabled: s.auth_master_password_enabled,
        unlocked: sda::vault::has_master_password_unlocked(),
        has_encrypted_files: has_encrypted,
    })
}

/// Submit the master password for the current session. Fails with
/// `VAULT_BAD_PASSWORD` if it doesn't decrypt the first maFile.enc found.
#[tauri::command]
pub fn auth_unlock(password: String) -> AppResult<()> {
    let s = settings::load()?;
    let ws = require_workspace(&s)?;
    // Probe against the first encrypted file we can find.
    let accounts = workspace::list_accounts(&ws)?;
    let mut probed = false;
    for a in &accounts {
        let enc = sda::vault::mafile_enc_path(&ws, &a.login)?;
        if enc.exists() {
            let blob = std::fs::read(&enc)?;
            sda::crypto::vault_decrypt(&password, &blob)
                .map_err(|_| AppError::Other("VAULT_BAD_PASSWORD".into()))?;
            probed = true;
            break;
        }
    }
    if !probed {
        // No encrypted file exists yet — that's fine, treat unlock as
        // accepting the password for future writes.
        tracing::info!("auth_unlock: no encrypted files; accepting password");
    }
    sda::vault::set_master_password(Some(password));
    Ok(())
}

/// Drop the master password from memory (re-lock). Files stay encrypted.
#[tauri::command]
pub fn auth_lock() -> AppResult<()> {
    sda::vault::set_master_password(None);
    Ok(())
}

/// Enable / change / disable the master password. `new_password = None`
/// disables encryption entirely (all files are rewritten as plain JSON).
#[tauri::command]
pub fn auth_set_master_password(
    old_password: Option<String>,
    new_password: Option<String>,
) -> AppResult<()> {
    let mut s = settings::load()?;
    let ws = require_workspace(&s)?;

    // To read existing encrypted files we need the old password loaded.
    if let Some(op) = old_password.as_ref() {
        sda::vault::set_master_password(Some(op.clone()));
    }
    // Validate old password by attempting to load anything encrypted.
    let accounts = workspace::list_accounts(&ws)?;
    for a in &accounts {
        let enc = sda::vault::mafile_enc_path(&ws, &a.login)?;
        if enc.exists() {
            let blob = std::fs::read(&enc)?;
            let pw = old_password
                .as_deref()
                .ok_or_else(|| AppError::NotReady("AUTH_NEEDS_OLD_PASSWORD".into()))?;
            sda::crypto::vault_decrypt(pw, &blob)
                .map_err(|_| AppError::Other("VAULT_BAD_PASSWORD".into()))?;
            break;
        }
    }

    sda::vault::rekey_all(&ws, new_password.as_deref())?;
    s.auth_master_password_enabled = new_password.is_some();
    settings::save(&s)?;
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
#[tauri::command]
pub fn auth_poller_configure(cfg: PollerConfig) -> AppResult<()> {
    let mut s = settings::load()?;
    s.auth_poller_enabled = cfg.enabled;
    s.auth_poller_interval = cfg.interval.clamp(15, 600);
    s.auth_auto_confirm_trades = cfg.auto_confirm_trades;
    s.auth_auto_confirm_market = cfg.auto_confirm_market;
    settings::save(&s)?;
    sda::poller::poke();
    Ok(())
}

#[tauri::command]
pub fn auth_poller_poke() -> AppResult<()> {
    sda::poller::poke();
    Ok(())
}

// ── P12: AddAuthenticator wizard ──────────────────────────────────────────

/// Diagnose post-login state so the wizard can pick the right path without
/// guess-and-check. Combines QueryStatus + AccountPhoneStatus.
#[tauri::command]
pub fn auth_add_diagnose(login: String) -> AppResult<sda::add::AddDiagnostic> {
    sda::add::diagnose(&login)
}

/// Phase A.1: attach a phone number to the account. Steam will email the
/// account owner to confirm; the UI polls `auth_add_check_email` afterwards.
#[tauri::command]
pub fn auth_add_set_phone(
    login: String,
    phone_number: String,
    phone_country_code: String,
) -> AppResult<sda::add::SetPhoneResult> {
    sda::add::add_set_phone(&login, &phone_number, &phone_country_code)
}

/// Phase A.2: poll whether we're still waiting on the email-confirmation step.
#[tauri::command]
pub fn auth_add_check_email(login: String) -> AppResult<sda::add::PhoneState> {
    sda::add::add_check_email(&login)
}

/// Phase A.3: ask Steam to send the SMS verification code.
#[tauri::command]
pub fn auth_add_send_sms(login: String) -> AppResult<()> {
    sda::add::add_send_sms(&login)
}

/// Phase A.4: submit the SMS code to verify the phone number.
#[tauri::command]
pub fn auth_add_verify_phone(login: String, code: String) -> AppResult<()> {
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
#[tauri::command]
pub fn auth_add_create(login: String) -> AppResult<sda::add::AddCreatePublic> {
    let r = sda::add::add_create(&login)?;
    if let Ok(s) = settings::load() {
        if let Ok(ws) = require_workspace(&s) {
            if let Err(e) = sda::add::add_persist_partial(&login, &ws) {
                tracing::error!("auth_add_create: partial persist failed: {e}");
            }
        }
    }
    Ok(r)
}

/// Phase C: Finalize. Surface back the revocation code to the UI ONCE so the
/// user can copy/screenshot it before we persist anything to disk.
#[tauri::command]
pub fn auth_add_finalize(
    login: String,
    sms_code: String,
    try_number: u32,
    validate_sms: bool,
) -> AppResult<sda::add::AddFinalizePublic> {
    sda::add::add_finalize(&login, &sms_code, try_number, validate_sms)
}

/// Final commit: write the maFile to disk + update workspace meta. The UI
/// MUST gate this on the user explicitly confirming they wrote down the
/// revocation code.
#[tauri::command]
pub fn auth_add_persist(login: String) -> AppResult<()> {
    let s = settings::load()?;
    let ws = require_workspace(&s)?;
    sda::add::add_persist(&login, &ws)?;
    sda::poller::poke();
    Ok(())
}

/// Abort the wizard, drop in-memory secrets without touching disk.
#[tauri::command]
pub fn auth_add_cancel(login: String) -> AppResult<()> {
    sda::add::drop_session(&login);
    Ok(())
}
