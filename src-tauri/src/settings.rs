use crate::error::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
static WRITE: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
        // Never recursively delete legacy data when a migration entry failed.
        // Empty directories can be removed; remaining files stay recoverable.
        let _ = fs::remove_dir(&legacy_cfg);
        let _ = fs::remove_dir(&legacy);
        let parent = base.join("kilocode");
        if parent
            .read_dir()
            .map(|mut r| r.next().is_none())
            .unwrap_or(false)
        {
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
    let _guard = WRITE.lock().unwrap();
    save_inner(s)
}

fn save_inner(s: &Settings) -> AppResult<()> {
    let path = settings_path()?;
    let txt = serde_json::to_string_pretty(s)?;
    if let Ok(previous) = fs::read(&path) {
        if serde_json::from_slice::<Settings>(&previous).is_ok() {
            crate::storage::atomic_write(&path.with_extension("json.backup"), &previous)?;
        }
    }
    crate::storage::atomic_write(&path, txt.as_bytes())?;
    Ok(())
}

pub fn update(f: impl FnOnce(&mut Settings)) -> AppResult<()> {
    let _guard = WRITE.lock().unwrap();
    let mut current = load()?;
    f(&mut current);
    save_inner(&current)
}

pub fn recover_settings() -> AppResult<Settings> {
    let _guard = WRITE.lock().unwrap();
    let path = settings_path()?;
    recover_at(&path)
}

fn recover_at(path: &std::path::Path) -> AppResult<Settings> {
    let recovered = fs::read(path.with_extension("json.backup"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Settings>(&b).ok())
        .unwrap_or_default();
    if path.exists() {
        fs::copy(
            &path,
            path.with_extension(format!("json.corrupt-{:016x}", rand::random::<u64>())),
        )?;
    }
    crate::storage::atomic_write(&path, &serde_json::to_vec_pretty(&recovered)?)?;
    Ok(recovered)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovery_restores_backup_and_preserves_damaged_settings() {
        let root = std::env::temp_dir().join(format!("shadow-settings-{}", rand::random::<u64>()));
        fs::create_dir(&root).unwrap();
        let path = root.join("settings.json");
        let mut original = Settings::default();
        original.workspace = Some(root.join("accounts-workspace")); original.language = "en".into();
        fs::write(path.with_extension("json.backup"), serde_json::to_vec(&original).unwrap()).unwrap();
        fs::write(&path, b"damaged-json").unwrap();
        let recovered = recover_at(&path).unwrap();
        assert_eq!(recovered.workspace, original.workspace); assert_eq!(recovered.language, "en");
        let copies: Vec<_> = fs::read_dir(&root).unwrap().map(Result::unwrap).filter(|e| e.file_name().to_string_lossy().contains(".corrupt-")).collect();
        assert_eq!(copies.len(), 1); assert_eq!(fs::read(copies[0].path()).unwrap(), b"damaged-json");
        assert!(fs::canonicalize(&root).unwrap().starts_with(fs::canonicalize(std::env::temp_dir()).unwrap()));
        fs::remove_dir_all(root).unwrap();
    }
}
