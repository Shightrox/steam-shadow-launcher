#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod download;
mod error;
mod http;
mod junction;
mod launcher;
mod library;
mod sandboxie;
mod sda;
mod settings;
mod shortcut;
mod steam_paths;
mod steam_process;
mod switcher;
mod vdf;
mod workspace;

fn main() {
    tracing_subscriber::fmt().init();

    // Headless launch mode: if invoked with `--launch=<login>` we run the
    // chosen account through the normal launcher pipeline and exit, without
    // showing any UI. This is what the desktop `.lnk` shortcuts created via
    // `create_account_shortcut` use.
    let args: Vec<String> = std::env::args().collect();
    if let Some(login) = args.iter().find_map(|a| a.strip_prefix("--launch=")) {
        let login = login.to_string();
        let res = headless_launch(&login);
        match res {
            Ok(_) => std::process::exit(0),
            Err(e) => {
                eprintln!("headless launch failed: {e}");
                std::process::exit(1);
            }
        }
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // Kick the background confirmation poller once the app is up.
            crate::sda::poller::start(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::detect_main_steam,
            commands::get_settings,
            commands::save_settings,
            commands::list_accounts,
            commands::add_account,
            commands::remove_account,
            commands::verify_account,
            commands::repair_account,
            commands::launch_shadow,
            commands::change_workspace,
            commands::set_workspace_initial,
            commands::set_main_steam_override,
            commands::cleanup_stale_junctions,
            commands::discover_steam_accounts,
            commands::import_discovered_accounts,
            commands::default_workspace,
            commands::detect_sandboxie,
            commands::install_sandboxie,
            commands::download_and_install_sandboxie,
            commands::is_elevated,
            commands::relaunch_as_admin,
            commands::list_running_games,
            commands::revert_last_switch,
            commands::close_window,
            commands::minimize_window,
            commands::start_drag,
            commands::set_account_favorite,
            commands::refresh_account_avatar,
            commands::list_running_sandboxes,
            commands::stop_sandbox,
            commands::list_account_games,
            commands::launch_game,
            commands::open_url,
            commands::create_account_shortcut,
            commands::auth_open_folder,
            commands::auth_status,
            commands::auth_import_mafile,
            commands::auth_export_mafile,
            commands::auth_remove,
            commands::auth_generate_code,
            commands::auth_sync_time,
            commands::auth_confirmations_list,
            commands::auth_confirmations_respond,
            commands::auth_login_begin,
            commands::auth_login_submit_code,
            commands::auth_login_poll,
            commands::auth_login_refresh,
            commands::auth_lock_status,
            commands::auth_unlock,
            commands::auth_lock,
            commands::auth_set_master_password,
            commands::auth_poller_configure,
            commands::auth_poller_poke,
            commands::auth_add_set_phone,
            commands::auth_add_check_email,
            commands::auth_add_send_sms,
            commands::auth_add_verify_phone,
            commands::auth_add_create,
            commands::auth_add_finalize,
            commands::auth_add_persist,
            commands::auth_add_cancel,
            commands::auth_add_diagnose,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn headless_launch(login: &str) -> Result<(), String> {
    let s = settings::load().map_err(|e| e.to_string())?;
    let ws = s.workspace.clone().ok_or_else(|| "workspace not configured".to_string())?;
    let main = steam_paths::detect(s.main_steam_path_override.clone())
        .map_err(|e| e.to_string())?;
    let sb = sandboxie::detect();
    let accounts = workspace::list_accounts(&ws).map_err(|e| e.to_string())?;
    let account = accounts
        .into_iter()
        .find(|a| a.login == login)
        .ok_or_else(|| format!("account '{login}' not found"))?;
    let mode = launcher::LaunchMode::parse(&s.default_launch_mode)
        .map_err(|e| e.to_string())?;

    // SANDBOX requires admin. If we're not elevated, prompt the user (via
    // MessageBoxW since we have no UI) and re-launch self with --launch=...
    // under UAC. Without this the .lnk shortcut would silently exit(1).
    if matches!(mode, launcher::LaunchMode::Sandbox) && !sandboxie::is_elevated_pub() {
        let proceed = message_box_yesno(
            "Steam Shadow Launcher",
            &format!(
                "Sandbox mode requires administrator rights.\n\nLaunch '{login}' with elevated privileges?"
            ),
        );
        if !proceed {
            return Err("user canceled UAC pre-prompt".into());
        }
        relaunch_self_with_args(&format!("--launch={login}"))
            .map_err(|e| format!("relaunch as admin failed: {e}"))?;
        return Ok(());
    }

    let _ = launcher::launch(&ws, &main, &sb, &account, mode)
        .map_err(|e| e.to_string())?;
    let _ = workspace::touch_last_launch(&ws, login);
    Ok(())
}

fn message_box_yesno(title: &str, body: &str) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, IDYES, MB_ICONQUESTION, MB_SETFOREGROUND, MB_TOPMOST, MB_YESNO,
    };
    let title_w: Vec<u16> = std::ffi::OsStr::new(title)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let body_w: Vec<u16> = std::ffi::OsStr::new(body)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let r = MessageBoxW(
            None,
            PCWSTR(body_w.as_ptr()),
            PCWSTR(title_w.as_ptr()),
            MB_YESNO | MB_ICONQUESTION | MB_TOPMOST | MB_SETFOREGROUND,
        );
        r == IDYES
    }
}

fn relaunch_self_with_args(args: &str) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::{ShellExecuteExW, SHELLEXECUTEINFOW};
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let exe = std::env::current_exe()?;
    let verb: Vec<u16> = "runas\0".encode_utf16().collect();
    let file: Vec<u16> = exe
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let params: Vec<u16> = args.encode_utf16().chain(std::iter::once(0)).collect();
    let mut sei = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..unsafe { std::mem::zeroed() }
    };
    unsafe {
        ShellExecuteExW(&mut sei as *mut _)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
    }
    Ok(())
}
