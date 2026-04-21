use crate::error::{AppError, AppResult};
use crate::steam_paths::MainSteamInfo;
use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct InstalledGame {
    pub appid: u32,
    pub name: String,
    pub installdir: String,
    #[serde(rename = "libraryPath")]
    pub library_path: PathBuf,
    #[serde(rename = "iconPath")]
    pub icon_path: Option<PathBuf>,
}

/// Parse `<MainSteam>/config/libraryfolders.vdf` and return paths to all
/// `steamapps` directories registered as Steam libraries.
pub fn library_steamapps(main: &MainSteamInfo) -> Vec<PathBuf> {
    let mut out = Vec::new();
    // Always include the host install's own steamapps even if libraryfolders.vdf
    // is missing or unreadable.
    let primary = main.install_dir.join("steamapps");
    out.push(primary.clone());
    let path = main.install_dir.join("config").join("libraryfolders.vdf");
    if let Ok(txt) = fs::read_to_string(&path) {
        for line in txt.lines() {
            let t = line.trim();
            if let Some(rest) = t.strip_prefix("\"path\"") {
                let parts: Vec<&str> = rest.split('"').collect();
                if parts.len() >= 2 {
                    let raw = parts[1].replace("\\\\", "\\");
                    let lib = PathBuf::from(raw).join("steamapps");
                    if !out.iter().any(|p| p == &lib) {
                        out.push(lib);
                    }
                }
            }
        }
    }
    out
}

/// Locate the most useful icon/cover for an appid in `<MainSteam>/appcache/librarycache`.
/// New Steam (2024+) keeps `<appid>/library_600x900.jpg` per app; older versions
/// kept `<appid>_library_600x900.jpg` flat. Try both.
fn find_game_icon(main: &MainSteamInfo, appid: u32) -> Option<PathBuf> {
    let cache = main.install_dir.join("appcache").join("librarycache");
    // Prefer per-app subfolder layout (current Steam).
    let dir = cache.join(appid.to_string());
    if dir.is_dir() {
        for name in [
            "library_600x900.jpg",
            "library_capsule.jpg",
            "header.jpg",
            "icon.jpg",
        ] {
            let p = dir.join(name);
            if p.exists() {
                return Some(p);
            }
        }
        // Anything with `library_600x900` in name (some installs have hashed prefix).
        if let Ok(rd) = fs::read_dir(&dir) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().to_string();
                if n.contains("library_600x900") || n.contains("header") {
                    return Some(e.path());
                }
            }
        }
    }
    // Legacy flat layout.
    for suffix in [
        "_library_600x900.jpg",
        "_library_capsule.jpg",
        "_header.jpg",
        "_icon.jpg",
    ] {
        let p = cache.join(format!("{appid}{suffix}"));
        if p.exists() {
            return Some(p);
        }
    }
    None
}

fn parse_appmanifest(path: &Path) -> Option<(u32, String, String)> {
    let txt = fs::read_to_string(path).ok()?;
    let mut appid: Option<u32> = None;
    let mut name: Option<String> = None;
    let mut installdir: Option<String> = None;
    for line in txt.lines() {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix("\"appid\"") {
            let parts: Vec<&str> = rest.split('"').collect();
            if parts.len() >= 2 {
                appid = parts[1].parse().ok();
            }
        } else if let Some(rest) = t.strip_prefix("\"name\"") {
            let parts: Vec<&str> = rest.split('"').collect();
            if parts.len() >= 2 {
                name = Some(parts[1].to_string());
            }
        } else if let Some(rest) = t.strip_prefix("\"installdir\"") {
            let parts: Vec<&str> = rest.split('"').collect();
            if parts.len() >= 2 {
                installdir = Some(parts[1].to_string());
            }
        }
        if appid.is_some() && name.is_some() && installdir.is_some() {
            break;
        }
    }
    Some((appid?, name?, installdir?))
}

pub fn list_installed_games(main: &MainSteamInfo) -> Vec<InstalledGame> {
    let libs = library_steamapps(main);
    let mut by_appid: HashMap<u32, InstalledGame> = HashMap::new();
    for lib in &libs {
        let Ok(rd) = fs::read_dir(lib) else { continue };
        for entry in rd.flatten() {
            let p = entry.path();
            let n = entry.file_name().to_string_lossy().to_string();
            if !n.starts_with("appmanifest_") || !n.ends_with(".acf") {
                continue;
            }
            let Some((appid, name, installdir)) = parse_appmanifest(&p) else {
                continue;
            };
            // Skip Steamworks Common Redistributables, Proton tooling, etc.
            // These are appids in well-known ranges (228980 = Steamworks Common, 1391110 = Proton 6.3-8, ...).
            if matches!(appid, 228980 | 250820 | 228983 | 1493710 | 1391110 | 1420170 | 1245040 | 1070560) {
                continue;
            }
            let icon = find_game_icon(main, appid);
            by_appid
                .entry(appid)
                .or_insert(InstalledGame {
                    appid,
                    name,
                    installdir,
                    library_path: lib.clone(),
                    icon_path: icon,
                });
        }
    }
    let mut out: Vec<InstalledGame> = by_appid.into_values().collect();
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

#[allow(dead_code)]
pub fn must_have_install_dir(games: &[InstalledGame]) -> AppResult<()> {
    if games.is_empty() {
        return Err(AppError::NotFound("no installed games found".into()));
    }
    Ok(())
}
