use crate::error::{AppError, AppResult};
use serde::Serialize;
use std::path::PathBuf;
use winreg::enums::*;
use winreg::RegKey;

fn read_hkcu(sub: &str, value: &str) -> Option<String> {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(sub, KEY_READ)
        .ok()?
        .get_value::<String, _>(value)
        .ok()
}

fn read_hklm(sub: &str, value: &str) -> Option<String> {
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey_with_flags(sub, KEY_READ)
        .ok()?
        .get_value::<String, _>(value)
        .ok()
}

#[derive(Debug, Clone, Serialize)]
pub struct MainSteamInfo {
    #[serde(rename = "installDir")]
    pub install_dir: PathBuf,
    #[serde(rename = "steamExe")]
    pub steam_exe: PathBuf,
    #[serde(rename = "steamappsDir")]
    pub steamapps_dir: PathBuf,
    #[serde(rename = "autologinUser")]
    pub autologin_user: Option<String>,
}

fn try_paths_from_registry() -> Option<PathBuf> {
    if let Some(p) = read_hkcu(r"Software\Valve\Steam", "SteamPath") {
        return Some(PathBuf::from(p.replace('/', "\\")));
    }
    if let Some(p) = read_hklm(r"SOFTWARE\WOW6432Node\Valve\Steam", "InstallPath") {
        return Some(PathBuf::from(p));
    }
    if let Some(p) = read_hklm(r"SOFTWARE\Valve\Steam", "InstallPath") {
        return Some(PathBuf::from(p));
    }
    None
}

fn try_paths_from_filesystem() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    for env in ["ProgramFiles(x86)", "ProgramFiles", "ProgramW6432"] {
        if let Ok(base) = std::env::var(env) {
            candidates.push(PathBuf::from(base).join("Steam"));
        }
    }
    // Last-resort literals — only if env vars somehow missing.
    candidates.push(PathBuf::from(r"C:\Program Files (x86)\Steam"));
    candidates.push(PathBuf::from(r"C:\Program Files\Steam"));
    for c in candidates {
        if c.join("steam.exe").exists() {
            return Some(c);
        }
    }
    None
}

pub fn read_autologin_user() -> Option<String> {
    read_hkcu(r"Software\Valve\Steam", "AutoLoginUser")
}

pub fn detect(override_path: Option<PathBuf>) -> AppResult<MainSteamInfo> {
    let install_dir = override_path
        .or_else(try_paths_from_registry)
        .or_else(try_paths_from_filesystem)
        .ok_or_else(|| AppError::Registry("STEAM_NOT_FOUND".into()))?;

    if !install_dir.exists() {
        return Err(AppError::Registry(format!(
            "Steam install dir does not exist: {}",
            install_dir.display()
        )));
    }

    let steam_exe = install_dir.join("steam.exe");
    if !steam_exe.exists() {
        return Err(AppError::Registry(format!(
            "steam.exe not found at {}",
            steam_exe.display()
        )));
    }

    let steamapps_dir = install_dir.join("steamapps");

    Ok(MainSteamInfo {
        install_dir,
        steam_exe,
        steamapps_dir,
        autologin_user: read_autologin_user(),
    })
}
