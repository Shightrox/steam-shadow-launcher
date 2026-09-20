use crate::error::{AppError, AppResult};
use crate::steam_paths::MainSteamInfo;
use serde::Serialize;
use std::collections::HashMap;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
use winreg::enums::*;
use winreg::RegKey;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[allow(dead_code)]
const DETACHED_PROCESS: u32 = 0x0000_0008;
#[allow(dead_code)]
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

#[derive(Debug, Clone, Serialize)]
pub struct SandboxieInfo {
    pub installed: bool,
    #[serde(rename = "installDir")]
    pub install_dir: Option<PathBuf>,
    #[serde(rename = "startExe")]
    pub start_exe: Option<PathBuf>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunningSandbox {
    pub login: String,
    #[serde(rename = "boxName")]
    pub box_name: String,
    #[serde(rename = "startedAt")]
    pub started_at: u64,
    pub pids: Vec<u32>,
}

/// Map of `box_name -> unix_seconds_of_launch`. Keeps lightweight uptime data
/// across multiple command invocations in the same launcher session.
static LAUNCH_TIMES: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();

fn launch_times() -> &'static Mutex<HashMap<String, u64>> {
    LAUNCH_TIMES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn mark_launched(box_name: &str) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if let Ok(mut m) = launch_times().lock() {
        m.insert(box_name.to_string(), now);
    }
}

fn read_install_dir() -> Option<PathBuf> {
    for sub in [
        r"SOFTWARE\Sandboxie-Plus",
        r"SOFTWARE\WOW6432Node\Sandboxie-Plus",
        r"SOFTWARE\Sandboxie",
        r"SOFTWARE\WOW6432Node\Sandboxie",
    ] {
        if let Ok(k) = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey_with_flags(sub, KEY_READ) {
            if let Ok(v) = k.get_value::<String, _>("Install_Dir") {
                let p = PathBuf::from(v);
                if p.exists() {
                    return Some(p);
                }
            }
            if let Ok(v) = k.get_value::<String, _>("InstallLocation") {
                let p = PathBuf::from(v);
                if p.exists() {
                    return Some(p);
                }
            }
        }
    }
    // Env-based fallbacks (works on D:\ / non-default ProgramFiles).
    let mut bases: Vec<PathBuf> = Vec::new();
    for env in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"] {
        if let Ok(b) = std::env::var(env) {
            bases.push(PathBuf::from(b));
        }
    }
    bases.push(PathBuf::from(r"C:\Program Files"));
    bases.push(PathBuf::from(r"C:\Program Files (x86)"));
    for base in bases {
        for name in ["Sandboxie-Plus", "Sandboxie"] {
            let p = base.join(name);
            if p.join("Start.exe").exists() {
                return Some(p);
            }
        }
    }
    None
}

pub fn detect() -> SandboxieInfo {
    let dir = read_install_dir();
    let start = dir.as_ref().map(|d| d.join("Start.exe"));
    let installed = start.as_ref().map(|p| p.exists()).unwrap_or(false);
    SandboxieInfo {
        installed,
        install_dir: dir,
        start_exe: start,
        version: None,
    }
}

/// Properly query the process token for elevation status. The previous
/// heuristic (try to open HKLM\SOFTWARE with KEY_WRITE) returns true for
/// standard users on most Windows installs because HKLM\SOFTWARE grants
/// write access to "Authenticated Users" — that's why the launcher
/// happily reported "elevated" while Sandboxie still failed.
fn is_elevated() -> bool {
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::{
        GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut size = std::mem::size_of::<TOKEN_ELEVATION>() as u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut _),
            size,
            &mut size,
        )
        .is_ok();
        let _ = CloseHandle(token);
        ok && elevation.TokenIsElevated != 0
    }
}

pub fn is_elevated_pub() -> bool {
    is_elevated()
}

/// Ensure the host's Steam Client Service (steamservice.exe) is up. When
/// Steam is launched for the first time after login, it starts this service
/// automatically; but if the user never started Steam, a sandboxed Steam
/// will hang forever on "Connecting" because it cannot spin the service up
/// from inside the sandbox (insufficient rights across the sandbox boundary).
///
/// We side-step this by spawning the host `steam.exe -silent` for a moment,
/// waiting until steamservice.exe appears in the process list, then shutting
/// host Steam down again. The sandboxed Steam can then talk to the already-
/// running service.
pub fn ensure_steam_client_service(main: &MainSteamInfo) -> AppResult<()> {
    if is_steamservice_running() {
        tracing::info!("ensure_steam_client_service: already running");
        return Ok(());
    }
    tracing::info!(
        "ensure_steam_client_service: spawning host steam.exe to bootstrap steamservice"
    );
    // -silent makes Steam start to tray only — enough to spin up the service
    // without stealing focus.
    Command::new(&main.steam_exe)
        .arg("-silent")
        .current_dir(&main.install_dir)
        .creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS)
        .spawn()
        .map_err(|e| AppError::Process(format!("spawn host steam: {e}")))?;
    // Wait up to ~15s for steamservice.exe to appear (first boot is slow).
    for i in 0..150 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if is_steamservice_running() {
            tracing::info!(
                "ensure_steam_client_service: steamservice up after {}ms",
                (i + 1) * 100
            );
            // Give the service a couple hundred ms to finish initialising
            // IPC endpoints before we hand off to the sandboxed client.
            std::thread::sleep(std::time::Duration::from_millis(500));
            return Ok(());
        }
    }
    tracing::warn!(
        "ensure_steam_client_service: steamservice.exe did not appear within 15s; \
         continuing anyway, sandboxed Steam may hang on 'Connecting'."
    );
    Ok(())
}

fn is_steamservice_running() -> bool {
    is_process_running("steamservice.exe")
}

fn is_process_running(name: &str) -> bool {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    unsafe {
        let snap = match CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
            Ok(h) => h,
            Err(_) => return false,
        };
        if snap.is_invalid() {
            return false;
        }
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..std::mem::zeroed()
        };
        let mut found = false;
        if Process32FirstW(snap, &mut entry).is_ok() {
            loop {
                let len = entry
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(entry.szExeFile.len());
                let pn = String::from_utf16_lossy(&entry.szExeFile[..len]);
                if pn.eq_ignore_ascii_case(name) {
                    found = true;
                    break;
                }
                if Process32NextW(snap, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
        found
    }
}

pub fn relaunch_self_as_admin() -> AppResult<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::{ShellExecuteExW, SHELLEXECUTEINFOW};
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let exe = std::env::current_exe().map_err(|e| AppError::Other(format!("current_exe: {e}")))?;
    let verb: Vec<u16> = "runas\0".encode_utf16().collect();
    let file: Vec<u16> = exe
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
            .map_err(|e| AppError::Process(format!("ShellExecuteExW(self runas): {e}")))?;
    }
    Ok(())
}

/// Use ShellExecuteExW with verb="runas" to trigger UAC, then WaitForSingleObject
/// for reliable completion. Returns process exit code.
fn shell_exec_runas_wait(installer: &std::path::Path, args: &str) -> std::io::Result<u32> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
    use windows::Win32::UI::Shell::{
        ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SEE_MASK_NO_CONSOLE, SHELLEXECUTEINFOW,
    };
    use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;

    let verb: Vec<u16> = "runas\0".encode_utf16().collect();
    let file: Vec<u16> = installer
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let params: Vec<u16> = args.encode_utf16().chain(std::iter::once(0)).collect();

    let mut sei = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NO_CONSOLE,
        hwnd: Default::default(),
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        lpDirectory: PCWSTR::null(),
        nShow: SW_HIDE.0,
        hInstApp: Default::default(),
        lpIDList: std::ptr::null_mut(),
        lpClass: PCWSTR::null(),
        hkeyClass: Default::default(),
        dwHotKey: 0,
        Anonymous: Default::default(),
        hProcess: HANDLE::default(),
    };

    unsafe {
        ShellExecuteExW(&mut sei as *mut _)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        if sei.hProcess.is_invalid() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "ShellExecuteExW: no process handle (user canceled UAC?)",
            ));
        }
        let wait = WaitForSingleObject(sei.hProcess, INFINITE);
        if wait != WAIT_OBJECT_0 {
            let _ = CloseHandle(sei.hProcess);
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("WaitForSingleObject: 0x{:x}", wait.0),
            ));
        }
        let mut code: u32 = 1;
        let _ = GetExitCodeProcess(sei.hProcess, &mut code);
        let _ = CloseHandle(sei.hProcess);
        Ok(code)
    }
}

/// Install Sandboxie silently. The installer is expected to be a sidecar/resource.
pub fn install_silent(installer_path: &std::path::Path) -> AppResult<()> {
    if !installer_path.exists() {
        return Err(AppError::NotFound(format!(
            "installer not found: {}",
            installer_path.display()
        )));
    }
    // Sandboxie-Plus v1.17+ uses **Inno Setup** (not NSIS), so /VERYSILENT is the
    // correct flag and /S is ignored — which is why /S used to pop up the wizard.
    // We try Inno first, then NSIS, then Qt-IFW as a fallback for any future repackaging.
    let attempts: &[&str] = &[
        "/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /SP-",
        "/S",
        "--silent",
    ];
    let elevated = is_elevated();
    tracing::info!(
        "install_silent: elevated={} installer={}",
        elevated,
        installer_path.display()
    );
    let mut last_err: Option<String> = None;
    for args in attempts {
        tracing::info!("install_silent: trying args: {}", args);
        let result = if elevated {
            Command::new(installer_path)
                .args(args.split_whitespace())
                .creation_flags(CREATE_NO_WINDOW)
                .status()
                .map(|s| s.code().unwrap_or(-1) as u32)
        } else {
            shell_exec_runas_wait(installer_path, args)
        };
        match result {
            Ok(code) if code == 0 => {
                std::thread::sleep(std::time::Duration::from_millis(1500));
                let info = detect();
                tracing::info!(
                    "install_silent: exit=0 after args {:?}, detected installed={}",
                    args,
                    info.installed
                );
                if info.installed {
                    return Ok(());
                }
                last_err = Some(format!("args '{}' exit=0 but Sandboxie not detected", args));
            }
            Ok(code) => {
                tracing::warn!("install_silent: args {:?} exit={}", args, code);
                last_err = Some(format!("args '{}' exit={}", args, code));
            }
            Err(e) => {
                tracing::warn!("install_silent: args {:?} err={}", args, e);
                last_err = Some(format!("args '{}' err: {}", args, e));
                // UAC cancelled - abort cascade, no point prompting again.
                if e.to_string().contains("canceled UAC") {
                    break;
                }
            }
        }
    }
    Err(AppError::Process(format!(
        "silent install failed: {}",
        last_err.unwrap_or_else(|| "unknown".into())
    )))
}

fn box_name(login: &str) -> String {
    use sha1::{Digest, Sha1};
    let hash = format!("{:x}", Sha1::digest(login.to_ascii_lowercase().as_bytes()));
    format!("SteamShadow_{}", &hash[..20])
}

/// Query sandbox membership for a PID. API failures fail closed before a kill.
pub fn process_box(pid: u32) -> AppResult<Option<String>> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::{
        Foundation::FreeLibrary,
        System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW},
    };
    unsafe {
        let loaded = GetModuleHandleW(windows::core::w!("SbieDll.dll")).ok();
        let info = detect();
        let module = if let Some(module) = loaded {
            module
        } else {
            let Some(dir) = info.install_dir else {
                return Ok(None);
            };
            let path: Vec<u16> = dir
                .join("SbieDll.dll")
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect();
            LoadLibraryW(windows::core::PCWSTR(path.as_ptr()))
                .map_err(|_| AppError::Process("SANDBOX_PROCESS_CHECK_FAILED".into()))?
        };
        type Query = unsafe extern "system" fn(
            *mut std::ffi::c_void,
            *mut u16,
            *mut u16,
            *mut u16,
            *mut u32,
        ) -> i32;
        let proc = GetProcAddress(module, windows::core::s!("SbieApi_QueryProcess"));
        let result = if let Some(proc) = proc {
            let query: Query = std::mem::transmute(proc);
            let mut name = [0u16; 34];
            let status = query(
                pid as usize as *mut _,
                name.as_mut_ptr(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );
            if status == 0 {
                let len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
                Ok(if len == 0 {
                    None
                } else {
                    Some(String::from_utf16_lossy(&name[..len]))
                })
            } else if matches!(status as u32, 0xc000000b | 0xc0000008 | 0xc0000034) {
                // Invalid CID/handle/not-found: PID is not registered as boxed.
                Ok(None)
            } else {
                Err(AppError::Process("SANDBOX_PROCESS_CHECK_FAILED".into()))
            }
        } else {
            Err(AppError::Process("SANDBOX_PROCESS_CHECK_FAILED".into()))
        };
        if loaded.is_none() {
            let _ = FreeLibrary(module);
        }
        result
    }
}

pub fn prepare_current_box(login: &str, steam_dir: PathBuf) -> AppResult<()> {
    crate::workspace::validate_login(login)?;
    if process_box(std::process::id())?.is_none() {
        return Err(AppError::Process("SANDBOX_HELPER_REQUIRES_BOX".into()));
    }
    let main = crate::steam_paths::detect(Some(steam_dir))?;
    match crate::switcher::patch_loginusers(&main, login) {
        Ok(()) => {}
        Err(AppError::NotFound(_)) => { /* New profile: Steam presents its login screen. */ }
        Err(e) => return Err(e),
    }
    crate::switcher::write_autologin(login)
}

fn resolved_box_name(info: &SandboxieInfo, login: &str) -> String {
    // Reuse the original box (and its virtual profile) only when its explicit
    // account title agrees. Collision owners never get silently reassigned.
    if let Some(path) = ini_path(info) {
        if let Ok(ini) = crate::sandbox_config::Ini::read(&path) {
            for (name, owner) in crate::sandbox_config::managed_boxes(&ini.text) {
                if owner.eq_ignore_ascii_case(login) {
                    return name;
                }
            }
        }
    }
    box_name(login)
}

/// Returns true if SandMan.exe (the Sandboxie tray UI / portable-mode driver
/// loader) is currently running.
fn is_sandman_running() -> bool {
    is_process_running("SandMan.exe")
}

/// Make sure SandMan.exe is running so the Sandboxie driver is loaded.
/// Spawn it detached + autorun behaviour (no UI popup) if not.
fn ensure_sandman_running(info: &SandboxieInfo) -> AppResult<()> {
    if is_sandman_running() {
        return Ok(());
    }
    let dir = info
        .install_dir
        .as_ref()
        .ok_or_else(|| AppError::NotFound("Sandboxie install dir unknown".into()))?;
    let sandman = dir.join("SandMan.exe");
    if !sandman.exists() {
        return Err(AppError::NotFound(format!(
            "SandMan.exe not found at {}",
            sandman.display()
        )));
    }
    tracing::info!("ensure_sandman_running: spawning {}", sandman.display());
    // `--autorun` makes SandMan start straight to tray without showing the
    // main window. Detach via DETACHED_PROCESS so it survives our exit.
    Command::new(&sandman)
        .arg("--autorun")
        .current_dir(dir)
        .creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS)
        .spawn()
        .map_err(|e| AppError::Process(format!("spawn SandMan.exe: {e}")))?;
    // Wait up to ~3s for the driver to come up.
    for i in 0..30 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if is_sandman_running() {
            // Brief extra wait so the driver is fully initialised before
            // Start.exe tries to use it.
            std::thread::sleep(std::time::Duration::from_millis(400));
            tracing::info!(
                "ensure_sandman_running: SandMan up after {}ms",
                (i + 1) * 100
            );
            return Ok(());
        }
    }
    Err(AppError::Process(
        "SandMan.exe failed to start within 3s".into(),
    ))
}

fn windir() -> PathBuf {
    if let Ok(v) = std::env::var("WINDIR") {
        return PathBuf::from(v);
    }
    if let Ok(v) = std::env::var("SystemRoot") {
        return PathBuf::from(v);
    }
    PathBuf::from(r"C:\Windows")
}

fn ini_path(info: &SandboxieInfo) -> Option<PathBuf> {
    // Match the driver's Conf_Read order: custom IniPath, installation, Windows.
    // Sandboxie-Plus.ini stores SandMan UI preferences, NOT sandbox definitions.
    let custom = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey_with_flags(r"SYSTEM\CurrentControlSet\Services\SbieDrv", KEY_READ)
        .ok()
        .and_then(|key| key.get_value::<String, _>("IniPath").ok())
        .map(|path| PathBuf::from(path.strip_prefix(r"\??\").unwrap_or(&path)));
    select_ini_path(custom, info.install_dir.as_deref(), &windir())
}

fn select_ini_path(
    custom: Option<PathBuf>,
    install: Option<&std::path::Path>,
    windows: &std::path::Path,
) -> Option<PathBuf> {
    custom
        .filter(|p| p.is_file())
        .or_else(|| {
            install
                .map(|p| p.join("Sandboxie.ini"))
                .filter(|p| p.is_file())
        })
        .or_else(|| install.map(|_| windows.join("Sandboxie.ini")))
}

#[cfg(test)]
mod config_path_tests {
    use super::*;

    #[test]
    fn box_names_fit_sdk_and_distinguish_punctuation() {
        let names: std::collections::HashSet<_> = ["foo-bar", "foo.bar", "foo_bar"]
            .map(box_name)
            .into_iter()
            .collect();
        assert_eq!(names.len(), 3);
        assert!(names.iter().all(|name| name.len() < 34));
        assert_eq!(box_name("DEMO"), box_name("demo"));
    }

    #[test]
    fn follows_driver_precedence_and_ignores_sandman_preferences() {
        let root =
            std::env::temp_dir().join(format!("steam-shadow-path-test-{}", rand::random::<u64>()));
        let home = root.join("home");
        let windows = root.join("windows");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir(&windows).unwrap();
        let custom = root.join("custom.ini");
        let system_ini = windows.join("Sandboxie.ini");
        let home_ini = home.join("Sandboxie.ini");
        let ui_ini = windows.join("Sandboxie-Plus.ini");
        std::fs::write(&ui_ini, "[Options]").unwrap();
        std::fs::write(&system_ini, "[DefaultBox]").unwrap();
        assert_eq!(
            select_ini_path(None, Some(&home), &windows),
            Some(system_ini.clone())
        );
        std::fs::write(&home_ini, "[DefaultBox]").unwrap();
        assert_eq!(
            select_ini_path(None, Some(&home), &windows),
            Some(home_ini.clone())
        );
        std::fs::write(&custom, "[DefaultBox]").unwrap();
        assert_eq!(
            select_ini_path(Some(custom.clone()), Some(&home), &windows),
            Some(custom.clone())
        );
        assert_eq!(
            select_ini_path(Some(root.join("missing.ini")), Some(&home), &windows),
            Some(home_ini.clone())
        );
        for path in [custom, system_ini, home_ini, ui_ini] {
            std::fs::remove_file(path).unwrap();
        }
        for path in [home, windows, root] {
            std::fs::remove_dir(path).unwrap();
        }
    }
}

pub fn ensure_box(info: &SandboxieInfo, main: &MainSteamInfo, login: &str) -> AppResult<String> {
    static CONFIG_LOCK: Mutex<()> = Mutex::new(());
    let _guard = CONFIG_LOCK.lock().unwrap();
    let name = resolved_box_name(info, login);
    let ini = ini_path(info).ok_or_else(|| AppError::NotFound("Sandboxie not installed".into()))?;
    tracing::info!("ensure_box: name={} ini={}", name, ini.display());
    let mut config = crate::sandbox_config::Ini::read(&ini)?;
    let templates = info
        .install_dir
        .as_ref()
        .map(|d| crate::sandbox_config::Ini::read(&d.join("Templates.ini")))
        .transpose()?
        .map(|i| i.text)
        .unwrap_or_default();
    config.text = crate::sandbox_config::repair_templates(&config.text, &templates);
    // Optional compatibility templates vary by Sandboxie version. There is no
    // built-in Steam template: use the explicit Steam rules below instead.
    let mut template_defaults = String::new();
    for template in ["OpenBluetooth", "SkipHook"] {
        if crate::sandbox_config::has_template(&templates, template)
            || crate::sandbox_config::has_template(&config.text, template)
        {
            template_defaults.push_str(&format!("Template={template}\n"));
        }
    }
    let main_dir = main.install_dir.display().to_string();
    config.text = crate::sandbox_config::isolate_steam(&config.text, &main_dir);
    let block = format!(
        "Enabled=y\n\
        BoxNameTitle=SS:{login}\n\
        BorderColor=#66ffcc,off,0\n\
        ConfigLevel=10\n\
        AutoRecover=n\n\
        BlockNetworkFiles=n\n\
        {template_defaults}\
        OpenFilePath={main_dir}\\steamapps\\*\n\
        OpenFilePath={main_dir}\\steamapps\n\
        OpenIpcPath=\\RPC Control\\steam*\n\
        OpenWinClass=SDL_app\n\
        CopyLimitKb=1048576\n\
        CopyLimitSilent=y\n\
        NotifyStartRunAccessDenied=n\n\
        NotifyDirectAccessAvailable=n\n\
        NotifyInternetAccessDenied=n\n\
        NotifyNoInternetAccess=n\n\
        AlertProcess=n\n\
        AlertFolder=n\n\
        AlertFile=n\n\
        AlertBeforeStart=n\n\
        ConfirmLowLabel=n\n\
        MsiInstallerExemptions=y\n\
        StartRunAlertDenied=n\n\
        AutoDelete=n\n\
        NeverDelete=y\n\
        ShowSandboxTip=n\n\
        ApplyElevateCreateProcessFix=y\n\
        DropAdminRights=n\n\
        UseRpcMonAsService=y\n\
        BlockSoftwareUpdaters=n\n\
        SeparateUserFolders=y\n\
        HideMessage=*\n\
        NoSbieDesk=y\n\
        NoSecurityCheck=n\n\
        NoUACProxy=y\n\
        SuppressMessage=*\n"
    );
    config.text = crate::sandbox_config::merge_section(&config.text, &name, &block);
    config.save(&ini)?;
    Ok(name)
}

fn reload_config(info: &SandboxieInfo) -> AppResult<()> {
    if let Some(start) = &info.start_exe {
        let status = Command::new(start)
            .arg("/reload")
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .map_err(|e| AppError::Process(format!("Sandboxie reload: {e}")))?;
        if !status.success() {
            return Err(AppError::Process(format!(
                "Sandboxie reload failed: {status}"
            )));
        }
    }
    Ok(())
}

pub fn launch_in_box(info: &SandboxieInfo, main: &MainSteamInfo, login: &str) -> AppResult<u32> {
    // Repair legacy templates before SandMan reads the config and raises SBIE1411.
    let name = ensure_box(info, main, login)?;
    // Portable mode: Sandboxie driver is only loaded while SandMan.exe runs.
    // Without it, Start.exe silently exits with code 1 and the box never opens.
    ensure_sandman_running(info)?;
    reload_config(info)?;
    let start = info
        .start_exe
        .clone()
        .ok_or_else(|| AppError::NotFound("Start.exe not found".into()))?;
    tracing::info!(
        "launch_in_box: start='{}' box={} steam='{}' login={}",
        start.display(),
        name,
        main.steam_exe.display(),
        login
    );
    let prepare = Command::new(&start)
        .arg(format!("/box:{name}"))
        .arg("/wait")
        .arg(std::env::current_exe()?)
        .arg(format!("--prepare-sandbox={login}"))
        .arg(format!("--steam-dir={}", main.install_dir.display()))
        .creation_flags(CREATE_NO_WINDOW)
        .status()?;
    if !prepare.success() {
        return Err(AppError::Process("SANDBOX_PREPARE_FAILED".into()));
    }
    let output = Command::new(&start)
        .arg(format!("/box:{name}"))
        .arg(&main.steam_exe)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| AppError::Process(format!("Sandboxie Start.exe spawn: {e}")))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let code = output.status.code();
    tracing::info!(
        "launch_in_box: Start.exe exit={:?} stdout='{}' stderr='{}'",
        code,
        stdout,
        stderr
    );
    if output.status.success() {
        mark_launched(&name);
    }
    if !output.status.success() {
        // Decode known Sandboxie SBIE error codes for readability.
        let hint = match code {
            Some(1608) => Some(
                "SBIE2204 / 1608: cannot open box. Likely the Sandboxie driver is not running. \
                 Try: open Sandboxie-Plus → Sandbox menu → Start Driver, then retry. \
                 If that fails, reboot once after install (driver requires kernel restart on first install).",
            ),
            Some(1707) => Some(
                "SBIE: program is restricted by box config. Check OpenFilePath rules.",
            ),
            Some(1314) => Some(
                "SBIE: not enough privileges. Run launcher as Administrator.",
            ),
            _ => None,
        };
        let msg = if let Some(h) = hint {
            format!("Sandboxie Start.exe exit={:?}\n{}", code, h)
        } else if !stderr.is_empty() {
            format!("Sandboxie Start.exe exit={:?}: {}", code, stderr)
        } else if !stdout.is_empty() {
            format!("Sandboxie Start.exe exit={:?}: {}", code, stdout)
        } else {
            format!(
                "Sandboxie Start.exe exit={:?} (no output). \
                 Common cause: Sandboxie driver not loaded. Open Sandboxie-Plus → \
                 Sandbox → Start Driver, or reboot once after install.",
                code
            )
        };
        return Err(AppError::Process(msg));
    }
    Ok(0)
}

/// List SteamShadow_* boxes that currently have at least one running process.
/// We use `Start.exe /box:NAME /list_pids` and check each SteamShadow_*
/// section in the ini.
pub fn list_running(info: &SandboxieInfo) -> Vec<RunningSandbox> {
    let mut out = Vec::new();
    let Some(start) = info.start_exe.as_ref() else {
        return out;
    };
    let Some(ini) = ini_path(info) else {
        return out;
    };
    let txt = match crate::sandbox_config::Ini::read(&ini) {
        Ok(t) => t.text,
        Err(_) => return out,
    };
    for (name, login) in crate::sandbox_config::managed_boxes(&txt) {
        let pids = list_box_pids(start, &name);
        if pids.is_empty() {
            continue;
        }
        let started = launch_times()
            .lock()
            .ok()
            .and_then(|m| m.get(&name).copied())
            .unwrap_or(0);
        out.push(RunningSandbox {
            login,
            box_name: name.to_string(),
            started_at: started,
            pids,
        });
    }
    out
}

fn list_box_pids(_start: &std::path::Path, box_name: &str) -> Vec<u32> {
    // Sandboxie's `Start.exe` does NOT accept a `/list_pids` switch (the
    // earlier code raised "Invalid command line parameter"). The supported
    // way to enumerate sandboxed PIDs is `SbieApi_EnumProcessEx` exported
    // from `SbieDll.dll`.
    enum_box_pids_via_sbiedll(box_name).unwrap_or_default()
}

fn enum_box_pids_via_sbiedll(box_name: &str) -> Option<Vec<u32>> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{FreeLibrary, HMODULE};
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

    // Build candidate list dynamically: install-dir (from registry) first,
    // then env-based ProgramFiles, then hardcoded C:\ as last resort.
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(dir) = read_install_dir() {
        candidates.push(dir.join("SbieDll.dll"));
    }
    for env in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"] {
        if let Ok(b) = std::env::var(env) {
            for name in ["Sandboxie-Plus", "Sandboxie"] {
                candidates.push(PathBuf::from(&b).join(name).join("SbieDll.dll"));
            }
        }
    }
    for base in [r"C:\Program Files", r"C:\Program Files (x86)"] {
        for name in ["Sandboxie-Plus", "Sandboxie"] {
            candidates.push(PathBuf::from(base).join(name).join("SbieDll.dll"));
        }
    }

    unsafe {
        let mut hmod: HMODULE = HMODULE::default();
        for c in &candidates {
            if !c.exists() {
                continue;
            }
            let w: Vec<u16> = c
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            if let Ok(h) = LoadLibraryW(PCWSTR(w.as_ptr())) {
                if !h.is_invalid() {
                    hmod = h;
                    break;
                }
            }
        }
        if hmod.is_invalid() {
            // Last resort: rely on PATH (Sandboxie install adds itself).
            let w: Vec<u16> = OsStr::new("SbieDll.dll")
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            if let Ok(h) = LoadLibraryW(PCWSTR(w.as_ptr())) {
                hmod = h;
            }
        }
        if hmod.is_invalid() {
            return None;
        }

        // LONG SbieApi_EnumProcessEx(
        //     const WCHAR* box_name,   // empty/NULL = all boxes
        //     BOOLEAN all_sessions,
        //     ULONG  which_session,    // -1 = all
        //     ULONG* pids,             // pids[0] in: max, out: count; pids[1..] PIDs
        //     ULONG* boxed_count_opt);
        type EnumProcEx = unsafe extern "system" fn(*const u16, u8, u32, *mut u32, *mut u32) -> i32;

        let proc = GetProcAddress(hmod, windows::core::s!("SbieApi_EnumProcessEx"));
        let func: EnumProcEx = match proc {
            Some(p) => std::mem::transmute(p),
            None => {
                let _ = FreeLibrary(hmod);
                return None;
            }
        };

        let box_w: Vec<u16> = OsStr::new(box_name)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let mut buf: [u32; 512] = [0; 512];
        buf[0] = 511; // capacity hint — convention used by Sandboxie SDK examples.
        let rc = func(
            box_w.as_ptr(),
            0,
            u32::MAX,
            buf.as_mut_ptr(),
            std::ptr::null_mut(),
        );
        let _ = FreeLibrary(hmod);
        if rc != 0 {
            return None;
        }
        let count = buf[0] as usize;
        if count == 0 || count >= buf.len() {
            // Either truly empty or buffer convention mismatch — return empty.
            if count == 0 {
                return Some(Vec::new());
            }
            return None;
        }
        Some(buf[1..=count].to_vec())
    }
}

/// Gracefully shut down a sandboxed Steam (`steam.exe -shutdown` inside the
/// box) and wait up to 3s; if still alive, terminate only that box.
pub fn stop_box(info: &SandboxieInfo, login: &str) -> AppResult<()> {
    let name = resolved_box_name(info, login);
    let start = info
        .start_exe
        .clone()
        .ok_or_else(|| AppError::NotFound("Start.exe not found".into()))?;
    let main = crate::steam_paths::detect(crate::settings::load()?.main_steam_path_override)?;
    tracing::info!("stop_box: graceful -shutdown into box={}", name);
    let _ = Command::new(&start)
        .arg(format!("/box:{name}"))
        .arg(&main.steam_exe)
        .arg("-shutdown")
        .creation_flags(CREATE_NO_WINDOW)
        .status();
    for _ in 0..30 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if list_box_pids(&start, &name).is_empty() {
            tracing::info!("stop_box: box {} idle after graceful", name);
            if let Ok(mut m) = launch_times().lock() {
                m.remove(&name);
            }
            return Ok(());
        }
    }
    tracing::warn!("stop_box: graceful timed out; terminate selected box {}", name);
    let status = Command::new(&start)
        .arg(format!("/box:{name}"))
        .arg("/terminate")
        .creation_flags(CREATE_NO_WINDOW)
        .status()?;
    if !status.success() { return Err(AppError::Process("SANDBOX_STOP_FAILED".into())); }
    if let Ok(mut m) = launch_times().lock() {
        m.remove(&name);
    }
    Ok(())
}

/// Send `-applaunch <appid>` into the running sandboxed Steam after a small
/// delay. Spawned on a background thread so the caller can return immediately.
pub fn spawn_applaunch_in_box(info: &SandboxieInfo, main: &MainSteamInfo, login: &str, appid: u32) {
    let Some(start) = info.start_exe.clone() else {
        return;
    };
    let steam = main.steam_exe.clone();
    let name = resolved_box_name(info, login);
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(2500));
        let _ = Command::new(&start)
            .arg(format!("/box:{name}"))
            .arg(&steam)
            .arg("-applaunch")
            .arg(appid.to_string())
            .creation_flags(CREATE_NO_WINDOW)
            .status();
    });
}

pub fn remove_box(info: &SandboxieInfo, login: &str) -> AppResult<()> {
    let name = resolved_box_name(info, login);
    if let Some(start) = &info.start_exe {
        let _ = Command::new(start)
            .arg(format!("/box:{name}"))
            .arg("delete_sandbox_silent")
            .creation_flags(CREATE_NO_WINDOW)
            .status();
    }
    if let Some(ini) = ini_path(info) {
        if let Ok(mut config) = crate::sandbox_config::Ini::read(&ini) {
            let txt = &config.text;
            let header = format!("[{name}]");
            if let Some(start_idx) = txt.find(&header) {
                let rest = &txt[start_idx..];
                // section ends at next "[" at line start
                let end_off = rest[1..]
                    .find("\n[")
                    .map(|i| start_idx + 1 + i + 1)
                    .unwrap_or(txt.len());
                let mut new = String::with_capacity(txt.len());
                new.push_str(&txt[..start_idx]);
                if end_off < txt.len() {
                    new.push_str(&txt[end_off..]);
                }
                config.text = new;
                config.save(&ini)?;
            }
        }
    }
    Ok(())
}
