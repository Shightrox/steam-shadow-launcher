//! Persistence for authenticator data.
//!
//! Layout under `<workspace>/accounts/<login>/auth/`:
//!   maFile.json  — plain JSON (when master password is disabled).
//!   maFile.enc   — Argon2id+AES-GCM sealed container (master password on).
//!
//! The master password, if set, is held in a process-wide mutex for the
//! duration of the session. Front-end calls `auth_unlock` once to populate
//! it; after that all subsequent auth_* commands transparently decrypt.

use crate::error::{AppError, AppResult};
use crate::sda::crypto;
use crate::sda::mafile::MaFile;
use crate::workspace;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// Holds the decrypted master password for the current process lifetime.
/// None = either (a) encryption is off, or (b) user hasn't unlocked yet.
fn key_cell() -> &'static Mutex<Option<String>> {
    static CELL: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(None))
}

pub fn set_master_password(pw: Option<String>) {
    *key_cell().lock().unwrap() = pw;
}

pub fn has_master_password_unlocked() -> bool {
    key_cell().lock().unwrap().is_some()
}

pub fn mafile_plain_path(workspace: &Path, login: &str) -> AppResult<PathBuf> {
    Ok(workspace::auth_dir(workspace, login)?.join("maFile.json"))
}

pub fn mafile_enc_path(workspace: &Path, login: &str) -> AppResult<PathBuf> {
    Ok(workspace::auth_dir(workspace, login)?.join("maFile.enc"))
}

/// True iff at least one of (plain | encrypted) maFile is on disk.
pub fn has_any(workspace: &Path, login: &str) -> bool {
    let auth = match workspace::auth_dir(workspace, login) {
        Ok(d) => d,
        Err(_) => return false,
    };
    auth.join("maFile.json").exists() || auth.join("maFile.enc").exists()
}

/// True iff ONLY the encrypted form is on disk — in which case the caller
/// must unlock first.
#[allow(dead_code)] // exposed for diagnostics / future UI gating
pub fn is_locked(workspace: &Path, login: &str) -> bool {
    let auth = match workspace::auth_dir(workspace, login) {
        Ok(d) => d,
        Err(_) => return false,
    };
    auth.join("maFile.enc").exists() && !auth.join("maFile.json").exists()
}

/// Load maFile for the given login. Decrypts transparently if .enc is present
/// and master password is unlocked. Returns Ok(None) if no file exists.
pub fn load_plain(workspace: &Path, login: &str) -> AppResult<Option<MaFile>> {
    let plain = mafile_plain_path(workspace, login)?;
    if plain.exists() {
        return Ok(Some(MaFile::read_file(&plain)?));
    }
    let enc = mafile_enc_path(workspace, login)?;
    if enc.exists() {
        let pw = key_cell()
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| AppError::NotReady("AUTH_LOCKED".into()))?;
        let blob = fs::read(&enc)?;
        let decrypted = crypto::vault_decrypt(&pw, &blob)?;
        let mf = MaFile::from_json_bytes(&decrypted)?;
        return Ok(Some(mf));
    }
    Ok(None)
}

/// Write maFile. Picks encrypted form iff master password is unlocked;
/// otherwise plain. Atomic via temp-rename.
pub fn save_plain(workspace: &Path, login: &str, mf: &MaFile) -> AppResult<()> {
    let pw = key_cell().lock().unwrap().clone();
    if let Some(pw) = pw {
        let enc_path = mafile_enc_path(workspace, login)?;
        let plain_path = mafile_plain_path(workspace, login)?;
        let json = mf.to_json_pretty()?;
        let sealed = crypto::vault_encrypt(&pw, json.as_bytes())?;
        atomic_write(&enc_path, &sealed)?;
        // Remove any stale plain file (migration / previously-plain account).
        if plain_path.exists() {
            let _ = shred_file(&plain_path);
        }
        return Ok(());
    }
    let path = mafile_plain_path(workspace, login)?;
    let json = mf.to_json_pretty()?;
    atomic_write(&path, json.as_bytes())?;
    Ok(())
}

/// Wipe maFiles for login. Best-effort overwrite-with-zeros before unlink.
pub fn remove(workspace: &Path, login: &str) -> AppResult<()> {
    let auth = workspace::auth_dir(workspace, login)?;
    for name in ["maFile.json", "maFile.enc"] {
        let p = auth.join(name);
        if p.exists() {
            shred_file(&p)?;
        }
    }
    Ok(())
}

/// Re-encrypt every existing maFile on disk with a new password (or migrate
/// between plain and encrypted). Called when the user changes/enables/disables
/// the master password.
///
/// `new_password = None` → decrypt everything to `maFile.json`.
/// `new_password = Some(pw)` → seal everything to `maFile.enc`.
pub fn rekey_all(workspace: &Path, new_password: Option<&str>) -> AppResult<()> {
    let accounts = workspace::list_accounts(workspace)?;
    for a in accounts {
        rekey_one(workspace, &a.login, new_password)?;
    }
    // Finally update the in-memory cache.
    set_master_password(new_password.map(|s| s.to_string()));
    Ok(())
}

fn rekey_one(workspace: &Path, login: &str, new_pw: Option<&str>) -> AppResult<()> {
    let mf = match load_plain(workspace, login)? {
        Some(m) => m,
        None => return Ok(()), // nothing to rekey
    };
    let plain_path = mafile_plain_path(workspace, login)?;
    let enc_path = mafile_enc_path(workspace, login)?;
    match new_pw {
        Some(pw) => {
            let json = mf.to_json_pretty()?;
            let sealed = crypto::vault_encrypt(pw, json.as_bytes())?;
            atomic_write(&enc_path, &sealed)?;
            if plain_path.exists() {
                shred_file(&plain_path)?;
            }
        }
        None => {
            let json = mf.to_json_pretty()?;
            atomic_write(&plain_path, json.as_bytes())?;
            if enc_path.exists() {
                shred_file(&enc_path)?;
            }
        }
    }
    Ok(())
}

fn atomic_write(path: &Path, bytes: &[u8]) -> AppResult<()> {
    let tmp = path.with_extension(format!(
        "{}.tmp",
        path.extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
    ));
    fs::write(&tmp, bytes)?;
    match fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(_) => {
            fs::remove_file(path).ok();
            fs::rename(&tmp, path).map_err(|e| AppError::Io(format!("rename: {e}")))
        }
    }
}

fn shred_file(p: &Path) -> AppResult<()> {
    if let Ok(meta) = fs::metadata(p) {
        let sz = meta.len() as usize;
        let wipe = sz.min(64 * 1024);
        if wipe > 0 {
            let zeros = vec![0u8; wipe];
            let _ = fs::write(p, &zeros);
        }
    }
    fs::remove_file(p).map_err(|e| AppError::Io(format!("wipe: {e}")))
}
