use crate::error::{AppError, AppResult};
use crate::junction::CleanupReport;
use crate::launcher::{self, LaunchMode, LaunchOutcome};
use crate::library::{self, InstalledGame};
use crate::sandboxie::{self, SandboxieInfo};
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
