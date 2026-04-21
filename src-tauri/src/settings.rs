use crate::error::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub workspace: Option<PathBuf>,
    #[serde(default, rename = "mainSteamPathOverride")]
    pub main_steam_path_override: Option<PathBuf>,
    #[serde(default, rename = "firstRunCompleted")]
    pub first_run_completed: bool,
    #[serde(default = "default_lang")]
    pub language: String,
    #[serde(default = "default_mode", rename = "defaultLaunchMode")]
    pub default_launch_mode: String,
    #[serde(default, rename = "sandboxieInstallAttempted")]
    pub sandboxie_install_attempted: bool,
    /// P11 M4: iff true, maFiles are stored as `maFile.enc` (Argon2id+AES-GCM).
    /// Purely a *presence* hint for the UI — the vault also auto-detects the
    /// extension on disk.
    #[serde(default, rename = "authMasterPasswordEnabled")]
    pub auth_master_password_enabled: bool,
    /// P11 M5: enable background polling of mobile confirmations.
    #[serde(default, rename = "authPollerEnabled")]
    pub auth_poller_enabled: bool,
    /// P11 M5: interval between polls (seconds). Default 60s, min 15s.
    #[serde(default = "default_poll_interval", rename = "authPollerInterval")]
    pub auth_poller_interval: u32,
    /// P11 M5: auto-allow outgoing trade confirmations (dangerous).
    #[serde(default, rename = "authAutoConfirmTrades")]
    pub auth_auto_confirm_trades: bool,
    /// P11 M5: auto-allow market-listing confirmations.
    #[serde(default, rename = "authAutoConfirmMarket")]
    pub auth_auto_confirm_market: bool,
}

fn default_version() -> u32 {
    1
}

fn default_lang() -> String {
    "ru".to_string()
}

fn default_mode() -> String {
    "switch".to_string()
}

fn default_poll_interval() -> u32 {
    60
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            workspace: None,
            main_steam_path_override: None,
            first_run_completed: false,
            language: default_lang(),
            default_launch_mode: default_mode(),
            sandboxie_install_attempted: false,
            auth_master_password_enabled: false,
            auth_poller_enabled: false,
            auth_poller_interval: default_poll_interval(),
            auth_auto_confirm_trades: false,
            auth_auto_confirm_market: false,
        }
    }
}

/// Returns `%APPDATA%\SteamShadowLauncher\` (Roaming). Creates it on demand.
pub fn config_dir() -> AppResult<PathBuf> {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| {
            // Fallback for the rare case where APPDATA is not set: use user profile.
            std::env::var_os("USERPROFILE")
                .map(|p| PathBuf::from(p).join("AppData").join("Roaming"))
        })
        .ok_or_else(|| AppError::Config("cannot resolve %APPDATA%".into()))?;
    let dir = base.join("SteamShadowLauncher");
    fs::create_dir_all(&dir)?;

    // One-shot migration: older builds stored data under
    // `%APPDATA%\kilocode\SteamShadowLauncher\config\`. If we find that layout
    // and the new layout is empty, move contents in so users don't lose settings.
    let legacy = base.join("kilocode").join("SteamShadowLauncher");
    if legacy.exists() && !dir.join("settings.json").exists() {
        let legacy_cfg = legacy.join("config");
        if legacy_cfg.exists() {
            if let Ok(entries) = fs::read_dir(&legacy_cfg) {
                for e in entries.flatten() {
                    let from = e.path();
                    let to = dir.join(e.file_name());
                    let _ = fs::rename(&from, &to).or_else(|_| {
                        if from.is_file() {
                            fs::copy(&from, &to).map(|_| ())
                        } else {
                            Ok(())
                        }
                    });
                }
            }
        }
        // Best-effort cleanup; ignore failures.
        let _ = fs::remove_dir_all(&legacy);
        let parent = base.join("kilocode");
        if parent.read_dir().map(|mut r| r.next().is_none()).unwrap_or(false) {
            let _ = fs::remove_dir(parent);
        }
    }

    Ok(dir)
}

pub fn default_workspace_path() -> Option<PathBuf> {
    let userdirs = directories::UserDirs::new()?;
    let docs = userdirs.document_dir()?;
    Some(docs.join("SteamShadow"))
}

pub fn settings_path() -> AppResult<PathBuf> {
    Ok(config_dir()?.join("settings.json"))
}

pub fn load() -> AppResult<Settings> {
    let path = settings_path()?;
    if !path.exists() {
        return Ok(Settings::default());
    }
    let txt = fs::read_to_string(&path)?;
    let s: Settings = serde_json::from_str(&txt)?;
    Ok(s)
}

pub fn save(s: &Settings) -> AppResult<()> {
    let path = settings_path()?;
    let txt = serde_json::to_string_pretty(s)?;
    fs::write(&path, txt)?;
    Ok(())
}
