//! Steam passwords protected by Windows DPAPI (current Windows user).
//! This file is separate from maFile and is never included in its export.
use crate::error::{AppError, AppResult};
use crate::workspace;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use windows::Win32::Foundation::{LocalFree, HLOCAL};
use windows::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

static STORE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Serialize, Deserialize, ZeroizeOnDrop)]
pub struct Credentials {
    pub account_name: String,
    pub password: String,
    pub steam_id: u64,
    generation: u64,
    retry_after: i64,
    disabled: bool,
}

impl Credentials {
    pub fn new(account_name: String, password: String, steam_id: u64) -> Self {
        Self {
            account_name,
            password,
            steam_id,
            generation: rand::random(),
            retry_after: 0,
            disabled: false,
        }
    }
}

fn path(workspace: &Path, login: &str) -> AppResult<PathBuf> {
    workspace::validate_login(login)?;
    Ok(workspace::auth_dir(workspace, login)?.join("credentials.dpapi"))
}

pub fn has_saved(workspace: &Path, login: &str) -> bool {
    path(workspace, login).is_ok_and(|p| p.is_file())
}

pub(crate) fn protect(data: &[u8], decrypt: bool) -> AppResult<Zeroizing<Vec<u8>>> {
    let input = CRYPT_INTEGER_BLOB {
        cbData: data
            .len()
            .try_into()
            .map_err(|_| AppError::Config("Credential data too large".into()))?,
        pbData: data.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    // Do NOT use CRYPTPROTECT_LOCAL_MACHINE: other Windows users must not be
    // able to decrypt the password. DPAPI authenticates the encrypted blob too.
    unsafe {
        let result = if decrypt {
            CryptUnprotectData(
                &input,
                None,
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptProtectData(
                &input,
                windows::core::w!("Steam Shadow Launcher"),
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        result.map_err(|_| {
            AppError::NotReady(
                if decrypt {
                    "AUTH_PASSWORD_UNAVAILABLE"
                } else {
                    "AUTH_PASSWORD_SAVE_FAILED"
                }
                .into(),
            )
        })?;
        let raw = std::slice::from_raw_parts_mut(output.pbData, output.cbData as usize);
        let bytes = Zeroizing::new(raw.to_vec());
        raw.zeroize();
        let _ = LocalFree(HLOCAL(output.pbData as *mut _));
        Ok(bytes)
    }
}

pub fn seal(credentials: &Credentials) -> AppResult<Vec<u8>> {
    let json = Zeroizing::new(serde_json::to_vec(credentials)?);
    Ok(protect(&json, false)?.to_vec())
}

fn read(path: &Path) -> AppResult<Option<Credentials>> {
    let sealed = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let plain = protect(&sealed, true)?;
    serde_json::from_slice(&plain)
        .map(Some)
        .map_err(|_| AppError::NotReady("AUTH_PASSWORD_UNAVAILABLE".into()))
}

fn write(path: &Path, sealed: &[u8]) -> AppResult<()> {
    crate::storage::atomic_write(path, sealed)
}

#[derive(Debug, Clone, Serialize)]
pub struct AutoLoginStatus {
    pub state: &'static str,
    pub retry_at: Option<i64>,
}
pub fn status(workspace: &Path, login: &str) -> AutoLoginStatus {
    let _guard = STORE_LOCK.lock().unwrap();
    match path(workspace, login).and_then(|p| read(&p)) {
        Ok(None) => AutoLoginStatus {
            state: "none",
            retry_at: None,
        },
        Err(_) => AutoLoginStatus {
            state: "unavailable",
            retry_at: None,
        },
        Ok(Some(c)) if c.disabled => AutoLoginStatus {
            state: "disabled",
            retry_at: None,
        },
        Ok(Some(c)) if c.retry_after > super::session_state::now_secs() => AutoLoginStatus {
            state: "cooldown",
            retry_at: Some(c.retry_after),
        },
        Ok(Some(_)) => AutoLoginStatus {
            state: "ready",
            retry_at: None,
        },
    }
}

/// Called only after Steam returns tokens for the matching login attempt.
pub fn save_sealed(workspace: &Path, login: &str, sealed: &[u8]) -> AppResult<()> {
    let _guard = STORE_LOCK.lock().unwrap();
    write(&path(workspace, login)?, sealed)
}

pub fn forget(workspace: &Path, login: &str) -> AppResult<()> {
    let _guard = STORE_LOCK.lock().unwrap();
    match std::fs::remove_file(path(workspace, login)?) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Persist a cooldown before attempting a password login, including across
/// restarts. Wrong credentials/challenges disable retries until manual login.
pub fn claim_attempt(workspace: &Path, login: &str, now: i64) -> AppResult<Credentials> {
    let _guard = STORE_LOCK.lock().unwrap();
    let path = path(workspace, login)?;
    let mut credentials =
        read(&path)?.ok_or_else(|| AppError::NotReady("CONF_NEEDS_RELOGIN".into()))?;
    if credentials.disabled {
        return Err(AppError::NotReady("AUTH_AUTO_LOGIN_NEEDS_INPUT".into()));
    }
    if credentials.retry_after > now {
        return Err(AppError::NotReady("AUTH_AUTO_LOGIN_COOLDOWN".into()));
    }
    credentials.retry_after = now + 300;
    write(&path, &seal(&credentials)?)?;
    Ok(credentials)
}

pub fn disable_attempt(workspace: &Path, login: &str, attempted: &Credentials) -> AppResult<()> {
    let _guard = STORE_LOCK.lock().unwrap();
    let path = path(workspace, login)?;
    if let Some(mut current) = read(&path)? {
        // Don't disable credentials replaced by a concurrent successful login,
        // or restore credentials deleted with the Forget button.
        if current.generation == attempted.generation {
            current.disabled = true;
            write(&path, &seal(&current)?)?;
        }
    }
    Ok(())
}

pub fn is_current(workspace: &Path, login: &str, attempted: &Credentials) -> AppResult<bool> {
    let _guard = STORE_LOCK.lock().unwrap();
    Ok(read(&path(workspace, login)?)?
        .is_some_and(|current| current.generation == attempted.generation && !current.disabled))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dpapi_roundtrip_rejects_tampering_and_never_stores_plain_password() {
        let credentials = Credentials::new("test".into(), "unique-test-password-42".into(), 1);
        let mut encrypted = seal(&credentials).unwrap();
        assert!(!encrypted
            .windows(credentials.password.len())
            .any(|w| w == credentials.password.as_bytes()));
        let plain = protect(&encrypted, true).unwrap();
        let loaded: Credentials = serde_json::from_slice(&plain).unwrap();
        assert_eq!(loaded.password, credentials.password);
        let last = encrypted.len() - 1;
        encrypted[last] ^= 0x40;
        assert!(protect(&encrypted, true).is_err());
    }

    #[test]
    fn cooldown_disable_replace_and_forget_are_persistent() {
        let ws =
            std::env::temp_dir().join(format!("shadow-credentials-test-{}", rand::random::<u64>()));
        let first = Credentials::new("test".into(), "test-password".into(), 1);
        save_sealed(&ws, "test", &seal(&first).unwrap()).unwrap();
        assert!(has_saved(&ws, "test"));
        let attempt = claim_attempt(&ws, "test", 1000).unwrap();
        assert!(claim_attempt(&ws, "test", 1001).is_err());
        disable_attempt(&ws, "test", &attempt).unwrap();
        assert!(claim_attempt(&ws, "test", 2000).is_err());
        let replacement = Credentials::new("test".into(), "new-password".into(), 1);
        save_sealed(&ws, "test", &seal(&replacement).unwrap()).unwrap();
        disable_attempt(&ws, "test", &attempt).unwrap();
        assert!(claim_attempt(&ws, "test", 2000).is_ok());
        forget(&ws, "test").unwrap();
        disable_attempt(&ws, "test", &attempt).unwrap();
        assert!(!has_saved(&ws, "test"));
        for p in [
            ws.join("accounts/test/auth"),
            ws.join("accounts/test"),
            ws.join("accounts"),
            ws,
        ] {
            std::fs::remove_dir(p).unwrap();
        }
    }
}
