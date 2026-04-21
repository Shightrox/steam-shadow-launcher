use crate::error::{AppError, AppResult};
use crate::settings;
use serde::Deserialize;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

const REPO_RELEASES: &str =
    "https://api.github.com/repos/sandboxie-plus/Sandboxie/releases/latest";
const UA: &str = "SteamShadowLauncher/0.1 (+https://github.com/kilo-org)";

#[derive(Debug, Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
    size: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Clone)]
pub struct InstallerAsset {
    pub url: String,
    pub name: String,
    pub size: Option<u64>,
    #[allow(dead_code)]
    pub tag: String,
}

pub fn fetch_latest_sandboxie_asset() -> AppResult<InstallerAsset> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(15))
        .timeout_read(Duration::from_secs(60))
        .build();
    let resp = agent
        .get(REPO_RELEASES)
        .set("User-Agent", UA)
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| AppError::Other(format!("github releases: {e}")))?;
    let rel: Release = resp
        .into_json()
        .map_err(|e| AppError::Other(format!("parse releases json: {e}")))?;
    // Prefer x64 NSIS installer
    let asset = rel
        .assets
        .iter()
        .find(|a| {
            let n = a.name.to_ascii_lowercase();
            n.starts_with("sandboxie-plus-x64") && n.ends_with(".exe")
        })
        .or_else(|| {
            rel.assets.iter().find(|a| {
                let n = a.name.to_ascii_lowercase();
                n.contains("sandboxie-plus") && n.ends_with(".exe")
            })
        })
        .ok_or_else(|| AppError::NotFound("no Sandboxie-Plus installer asset".into()))?;
    Ok(InstallerAsset {
        url: asset.browser_download_url.clone(),
        name: asset.name.clone(),
        size: asset.size,
        tag: rel.tag_name.clone(),
    })
}

pub fn downloads_dir() -> AppResult<PathBuf> {
    let dir = settings::config_dir()?
        .parent()
        .ok_or_else(|| AppError::Config("no parent of config dir".into()))?
        .join("downloads");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Download URL into `dst`, calling `on_progress(downloaded, total)` periodically.
pub fn download_with_progress<F: FnMut(u64, Option<u64>)>(
    url: &str,
    dst: &std::path::Path,
    mut on_progress: F,
) -> AppResult<()> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(20))
        .timeout_read(Duration::from_secs(120))
        .build();
    let resp = agent
        .get(url)
        .set("User-Agent", UA)
        .call()
        .map_err(|e| AppError::Other(format!("download: {e}")))?;
    let total = resp
        .header("Content-Length")
        .and_then(|s| s.parse::<u64>().ok());
    let mut reader = resp.into_reader();
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = dst.with_extension("part");
    let mut file = File::create(&tmp)?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut downloaded: u64 = 0;
    let mut last_emit = Instant::now() - Duration::from_secs(1);
    on_progress(0, total);
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| AppError::Io(format!("read chunk: {e}")))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        downloaded += n as u64;
        if last_emit.elapsed() >= Duration::from_millis(150) {
            on_progress(downloaded, total);
            last_emit = Instant::now();
        }
    }
    file.flush()?;
    drop(file);
    if dst.exists() {
        let _ = fs::remove_file(dst);
    }
    fs::rename(&tmp, dst)?;
    on_progress(downloaded, total.or(Some(downloaded)));
    Ok(())
}
