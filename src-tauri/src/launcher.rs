use crate::error::{AppError, AppResult};
use crate::sandboxie::{self, SandboxieInfo};
use crate::steam_paths::MainSteamInfo;
use crate::switcher::{self, SwitchResult};
use crate::workspace::Account;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;

const DETACHED_PROCESS: u32 = 0x0000_0008;
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

#[derive(Debug, Clone, Copy)]
pub enum LaunchMode {
    Switch,
    Sandbox,
}

impl LaunchMode {
    pub fn parse(s: &str) -> AppResult<Self> {
        match s {
            "switch" => Ok(LaunchMode::Switch),
            "sandbox" => Ok(LaunchMode::Sandbox),
            other => Err(AppError::Other(format!("unknown mode: {other}"))),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum LaunchOutcome {
    Switch {
        pid: u32,
        #[serde(rename = "previousAutologin")]
        previous_autologin: Option<String>,
    },
    Sandbox {
        pid: u32,
    },
}

pub fn launch(
    workspace: &Path,
    main: &MainSteamInfo,
    sandboxie: &SandboxieInfo,
    account: &Account,
    mode: LaunchMode,
) -> AppResult<LaunchOutcome> {
    launch_inner(workspace, main, sandboxie, account, mode, None)
}

/// Launch Steam under the given account and then hand off `-applaunch <appid>`
/// so Steam starts that specific game after auto-login completes. Same
/// semantics as `launch` otherwise (switch vs sandbox).
pub fn launch_game(
    workspace: &Path,
    main: &MainSteamInfo,
    sandboxie: &SandboxieInfo,
    account: &Account,
    mode: LaunchMode,
    appid: u32,
) -> AppResult<LaunchOutcome> {
    launch_inner(workspace, main, sandboxie, account, mode, Some(appid))
}

fn launch_inner(
    workspace: &Path,
    main: &MainSteamInfo,
    sandboxie: &SandboxieInfo,
    account: &Account,
    mode: LaunchMode,
    appid: Option<u32>,
) -> AppResult<LaunchOutcome> {
    match mode {
        LaunchMode::Switch => {
            let SwitchResult {
                steam_pid,
                previous_autologin,
                ..
            } = switcher::switch_to(workspace, main, &account.login)?;
            if let Some(id) = appid {
                // Hand off the applaunch request via a secondary steam.exe
                // invocation. The already-running Steam picks it up over IPC
                // once auto-login finishes and starts the game.
                spawn_applaunch_host(main, id);
            }
            Ok(LaunchOutcome::Switch {
                pid: steam_pid,
                previous_autologin,
            })
        }
        LaunchMode::Sandbox => {
            if !sandboxie.installed {
                return Err(AppError::NotReady(
                    "Sandboxie-Plus not installed".into(),
                ));
            }
            if !sandboxie::is_elevated_pub() {
                return Err(AppError::NotReady("NEED_ADMIN".into()));
            }
            sandboxie::ensure_steam_client_service(main).ok();
            let _ = switcher::backup_loginusers(workspace, main)?;
            let _ = switcher::backup_registry(workspace)?;
            switcher::patch_loginusers(main, &account.login)?;
            switcher::write_autologin(&account.login)?;
            let pid = sandboxie::launch_in_box(sandboxie, main, &account.login)?;
            if let Some(id) = appid {
                sandboxie::spawn_applaunch_in_box(sandboxie, main, &account.login, id);
            }
            Ok(LaunchOutcome::Sandbox { pid })
        }
    }
}

fn spawn_applaunch_host(main: &MainSteamInfo, appid: u32) {
    // Small delay so the primary Steam has time to bring up its IPC window.
    std::thread::spawn({
        let exe = main.steam_exe.clone();
        let cwd = main.install_dir.clone();
        move || {
            std::thread::sleep(std::time::Duration::from_millis(1500));
            let _ = Command::new(&exe)
                .arg("-applaunch")
                .arg(appid.to_string())
                .current_dir(&cwd)
                .creation_flags(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS)
                .spawn();
        }
    });
}
