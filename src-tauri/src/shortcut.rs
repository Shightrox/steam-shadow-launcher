use crate::error::{AppError, AppResult};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

/// Create a `.lnk` on the current user's Desktop that re-launches this exe
/// with `--launch=<login>` so the chosen account starts immediately.
/// Returns the absolute path to the created shortcut.
pub fn create_desktop_shortcut(login: &str) -> AppResult<PathBuf> {
    let exe = std::env::current_exe()
        .map_err(|e| AppError::Other(format!("current_exe: {e}")))?;
    let desktop = desktop_dir()
        .ok_or_else(|| AppError::NotFound("Desktop folder not found".into()))?;
    let safe: String = login
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect();
    let lnk = desktop.join(format!("Steam Shadow — {safe}.lnk"));
    let args = format!("--launch={}", login);
    write_shortcut(&exe, &args, &lnk, exe.parent())?;
    Ok(lnk)
}

fn desktop_dir() -> Option<PathBuf> {
    // Prefer SHGetKnownFolderPath(FOLDERID_Desktop) — handles OneDrive
    // redirect and roaming profiles that move Desktop away from %USERPROFILE%.
    if let Some(p) = known_folder_desktop() {
        if p.is_dir() {
            return Some(p);
        }
    }
    if let Ok(p) = std::env::var("USERPROFILE") {
        let d = PathBuf::from(p).join("Desktop");
        if d.is_dir() {
            return Some(d);
        }
    }
    None
}

fn known_folder_desktop() -> Option<PathBuf> {
    use windows::Win32::UI::Shell::{
        FOLDERID_Desktop, SHGetKnownFolderPath, KF_FLAG_DEFAULT,
    };
    unsafe {
        let pwstr =
            SHGetKnownFolderPath(&FOLDERID_Desktop, KF_FLAG_DEFAULT, None).ok()?;
        // Read until NUL.
        let mut len = 0usize;
        while *pwstr.0.add(len) != 0 {
            len += 1;
        }
        let slice = std::slice::from_raw_parts(pwstr.0, len);
        let s = String::from_utf16_lossy(slice);
        windows::Win32::System::Com::CoTaskMemFree(Some(pwstr.0 as *const _));
        Some(PathBuf::from(s))
    }
}

fn write_shortcut(
    target: &Path,
    args: &str,
    lnk_path: &Path,
    work_dir: Option<&Path>,
) -> AppResult<()> {
    use windows::core::{Interface, PCWSTR};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};

    let target_w: Vec<u16> = wide(target.as_os_str());
    let args_w: Vec<u16> = wide_str(args);
    let lnk_w: Vec<u16> = wide(lnk_path.as_os_str());
    let workdir_w: Option<Vec<u16>> = work_dir.map(|p| wide(p.as_os_str()));

    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let result: AppResult<()> = (|| {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
                .map_err(|e| AppError::Other(format!("CoCreateInstance(ShellLink): {e}")))?;
            link.SetPath(PCWSTR(target_w.as_ptr()))
                .map_err(|e| AppError::Other(format!("SetPath: {e}")))?;
            link.SetArguments(PCWSTR(args_w.as_ptr()))
                .map_err(|e| AppError::Other(format!("SetArguments: {e}")))?;
            if let Some(w) = workdir_w.as_ref() {
                let _ = link.SetWorkingDirectory(PCWSTR(w.as_ptr()));
            }
            let _ = link.SetIconLocation(PCWSTR(target_w.as_ptr()), 0);
            let pf: IPersistFile = link
                .cast()
                .map_err(|e| AppError::Other(format!("QI IPersistFile: {e}")))?;
            pf.Save(PCWSTR(lnk_w.as_ptr()), true)
                .map_err(|e| AppError::Other(format!("IPersistFile::Save: {e}")))?;
            Ok(())
        })();
        CoUninitialize();
        result
    }
}

fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
    s.encode_wide().chain(std::iter::once(0)).collect()
}

fn wide_str(s: &str) -> Vec<u16> {
    std::ffi::OsString::from(s)
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}
