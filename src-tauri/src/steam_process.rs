use crate::error::{AppError, AppResult};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const DETACHED_PROCESS: u32 = 0x0000_0008;
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

#[derive(Debug, Clone, serde::Serialize)]
pub struct SteamProcess {
    pub pid: u32,
    pub exe: PathBuf,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RunningGame {
    pub pid: u32,
    pub exe_name: String,
    pub exe_path: PathBuf,
}

#[derive(Debug, Clone)]
struct ProcInfo {
    pid: u32,
    ppid: u32,
    name: String,
    exe_path: PathBuf,
}

/// Enumerate processes via Win32 ToolHelp32 snapshot (wmic is deprecated on W11 24H2+).
fn enumerate_processes() -> Vec<ProcInfo> {
    use windows::Win32::Foundation::{CloseHandle, HANDLE, MAX_PATH};
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    let mut out = Vec::new();
    unsafe {
        let snap: HANDLE = match CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
            Ok(h) => h,
            Err(_) => return out,
        };
        if snap.is_invalid() {
            return out;
        }
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..std::mem::zeroed()
        };
        if Process32FirstW(snap, &mut entry).is_ok() {
            loop {
                let name = {
                    let len = entry
                        .szExeFile
                        .iter()
                        .position(|&c| c == 0)
                        .unwrap_or(entry.szExeFile.len());
                    String::from_utf16_lossy(&entry.szExeFile[..len])
                };
                let pid = entry.th32ProcessID;
                let ppid = entry.th32ParentProcessID;
                // Resolve full path (may fail for protected processes)
                let exe_path = if pid == 0 {
                    PathBuf::new()
                } else {
                    match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                        Ok(h) if !h.is_invalid() => {
                            let mut buf = vec![0u16; (MAX_PATH as usize) * 2];
                            let mut sz: u32 = buf.len() as u32;
                            let res = QueryFullProcessImageNameW(
                                h,
                                PROCESS_NAME_WIN32,
                                windows::core::PWSTR(buf.as_mut_ptr()),
                                &mut sz,
                            );
                            let _ = CloseHandle(h);
                            if res.is_ok() && sz > 0 {
                                PathBuf::from(String::from_utf16_lossy(&buf[..sz as usize]))
                            } else {
                                PathBuf::new()
                            }
                        }
                        _ => PathBuf::new(),
                    }
                };
                out.push(ProcInfo { pid, ppid, name, exe_path });
                if Process32NextW(snap, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
    }
    out
}

pub fn find_steam_processes() -> Vec<SteamProcess> {
    enumerate_processes()
        .into_iter()
        .filter(|p| p.name.eq_ignore_ascii_case("steam.exe"))
        .map(|p| SteamProcess { pid: p.pid, exe: p.exe_path })
        .collect()
}

/// Find games launched by Steam: process whose path lies inside `<MainSteam>/steamapps/common/`
/// AND whose ancestry chain includes `steam.exe`.
pub fn find_running_games(main_steamapps: &Path) -> Vec<RunningGame> {
    let procs = enumerate_processes();
    let mut by_pid: std::collections::HashMap<u32, ProcInfo> =
        std::collections::HashMap::with_capacity(procs.len());
    for p in &procs {
        by_pid.insert(p.pid, p.clone());
    }
    let common = main_steamapps.join("common");
    let common_norm = common.to_string_lossy().to_lowercase();
    let mut out = Vec::new();
    for p in &procs {
        let path_norm = p.exe_path.to_string_lossy().to_lowercase();
        if path_norm.is_empty() || !path_norm.starts_with(&common_norm) {
            continue;
        }
        // Walk ancestry up to 16 levels
        let mut cursor = p.ppid;
        let mut depth = 0;
        let mut steam_ancestor = false;
        while depth < 16 && cursor != 0 {
            let Some(parent) = by_pid.get(&cursor) else { break };
            if parent.name.eq_ignore_ascii_case("steam.exe") {
                steam_ancestor = true;
                break;
            }
            cursor = parent.ppid;
            depth += 1;
        }
        if steam_ancestor {
            out.push(RunningGame {
                pid: p.pid,
                exe_name: p.name.clone(),
                exe_path: p.exe_path.clone(),
            });
        }
    }
    out
}

/// Graceful shutdown of all Steam processes. Returns `true` if at least one was killed.
pub fn graceful_shutdown(steam_exe: &Path) -> AppResult<bool> {
    let initial = find_steam_processes();
    tracing::info!("graceful_shutdown: found {} steam processes", initial.len());
    if initial.is_empty() {
        return Ok(false);
    }

    // Step 1: ask steam to shut down via its own flag
    let _ = Command::new(steam_exe)
        .arg("-shutdown")
        .creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
        .spawn();

    let deadline = Instant::now() + Duration::from_secs(8);
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(400));
        if find_steam_processes().is_empty() {
            tracing::info!("graceful_shutdown: steam shut down via -shutdown");
            return Ok(true);
        }
    }

    // Step 2: WM_CLOSE via taskkill
    let _ = Command::new("taskkill")
        .args(["/IM", "steam.exe"])
        .creation_flags(CREATE_NO_WINDOW)
        .status();

    let deadline = Instant::now() + Duration::from_secs(4);
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(300));
        if find_steam_processes().is_empty() {
            tracing::info!("graceful_shutdown: steam closed via taskkill");
            return Ok(true);
        }
    }

    // Step 3: force
    let status = Command::new("taskkill")
        .args(["/F", "/IM", "steam.exe", "/T"])
        .creation_flags(CREATE_NO_WINDOW)
        .status();
    if let Ok(s) = status {
        if !s.success() {
            return Err(AppError::Process(format!(
                "taskkill /F failed: {:?}",
                s.code()
            )));
        }
    }
    std::thread::sleep(Duration::from_millis(800));
    tracing::info!("graceful_shutdown: steam force-killed");
    Ok(true)
}
