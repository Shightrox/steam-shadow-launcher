use crate::error::{AppError, AppResult};
use directories::ProjectDirs;
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
        }
    }
}

pub fn default_workspace_path() -> Option<PathBuf> {
    let userdirs = directories::UserDirs::new()?;
    let docs = userdirs.document_dir()?;
    Some(docs.join("SteamShadow"))
}

pub fn config_dir() -> AppResult<PathBuf> {
    let pd = ProjectDirs::from("io", "kilocode", "SteamShadowLauncher")
        .ok_or_else(|| AppError::Config("cannot resolve config dir".into()))?;
    let dir = pd.config_dir().to_path_buf();
    fs::create_dir_all(&dir)?;
    Ok(dir)
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
