//! Shared session renewal for manual confirmations and background polling.
use super::{login as steam_login, mafile::MaFile, relogin_flag, session_state, vault};
use crate::error::{AppError, AppResult};
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, Weak};

pub fn is_expired(error: &AppError) -> bool {
    matches!(error, AppError::NotReady(code) if code == "CONF_SESSION_EXPIRED")
}

pub fn refresh(mf: &mut MaFile, login: &str) -> AppResult<()> {
    let session = mf
        .session
        .as_mut()
        .ok_or_else(|| AppError::NotReady("CONF_NO_SESSION".into()))?;
    if session.refresh_token.trim().is_empty() {
        relogin_flag::mark(login);
        return Err(AppError::NotReady("CONF_NEEDS_RELOGIN".into()));
    }
    match steam_login::refresh_access_token(&session.refresh_token, &session.steam_id.to_string()) {
        Ok(token) => {
            session.access_token = token;
        }
        Err(error) => {
            // Network failures and rate limits must not permanently disable polling.
            if matches!(&error, AppError::NotReady(code) if code == "CONF_NEEDS_RELOGIN") {
                relogin_flag::mark(login);
            }
            return Err(error);
        }
    }
    relogin_flag::clear(login);
    Ok(())
}

pub fn account_gate(workspace: &Path, login: &str) -> AppResult<Arc<Mutex<()>>> {
    type Registry = HashMap<(PathBuf, String), Weak<Mutex<()>>>;
    static LOCKS: OnceLock<Mutex<Registry>> = OnceLock::new();
    let key = (
        crate::storage::path_key(workspace)?,
        login.to_ascii_lowercase(),
    );
    let mut locks = LOCKS.get_or_init(Default::default).lock().unwrap();
    locks.retain(|_, gate| gate.strong_count() > 0);
    if let Some(gate) = locks.get(&key).and_then(Weak::upgrade) {
        return Ok(gate);
    }
    let gate = Arc::new(Mutex::new(()));
    locks.insert(key, Arc::downgrade(&gate));
    Ok(gate)
}

pub fn with_ready<T>(
    workspace: &Path,
    login: &str,
    operation: impl Fn(&MaFile) -> AppResult<T>,
) -> AppResult<T> {
    let gate = account_gate(workspace, login)?;
    let _guard = gate.lock().unwrap();
    let mut mf = vault::load_plain(workspace, login)?
        .ok_or_else(|| AppError::NotFound(format!("no maFile for {login}")))?;
    crate::workspace::validate_auth_identity(workspace, login, &mf)?;
    let result = execute(
        &mut mf,
        login,
        session_state::now_secs(),
        &operation,
        |mf| refresh(mf, login),
        |mf| vault::save_plain(workspace, login, mf),
    );
    recover_login(result, || {
        super::reauth::automatic(workspace, login, &mut mf)?;
        vault::save_plain(workspace, login, &mf)?;
        match operation(&mf) {
            Err(error) if is_expired(&error) => {
                relogin_flag::mark(login);
                Err(AppError::NotReady("CONF_NEEDS_RELOGIN".into()))
            }
            result => result,
        }
    })
}

fn recover_login<T>(result: AppResult<T>, login: impl FnOnce() -> AppResult<T>) -> AppResult<T> {
    match result {
        Err(AppError::NotReady(code))
            if code == "CONF_NEEDS_RELOGIN" || code == "CONF_NO_SESSION" =>
        {
            login()
        }
        result => result,
    }
}

fn execute<T>(
    mf: &mut MaFile,
    login: &str,
    now: i64,
    mut operation: impl FnMut(&MaFile) -> AppResult<T>,
    mut renew: impl FnMut(&mut MaFile) -> AppResult<()>,
    mut save: impl FnMut(&MaFile) -> AppResult<()>,
) -> AppResult<T> {
    if mf.fully_enrolled == Some(false) {
        return Err(AppError::NotReady("CONF_NOT_ENROLLED".into()));
    }
    let mut changed = false;
    match session_state::classify(mf, login, now) {
        session_state::SessionState::NoSession => {
            return Err(AppError::NotReady("CONF_NO_SESSION".into()))
        }
        session_state::SessionState::NeedsRelogin => {
            return Err(AppError::NotReady("CONF_NEEDS_RELOGIN".into()))
        }
        session_state::SessionState::Refreshable => {
            renew(mf)?;
            changed = true;
        }
        session_state::SessionState::Ok => {}
    }
    // SessionID is a client-generated CSRF cookie; SDA exports may omit it.
    if let Some(session) = mf.session.as_mut() {
        if session.session_id.trim().is_empty() {
            use rand::RngCore;
            let mut bytes = [0u8; 12];
            rand::thread_rng().fill_bytes(&mut bytes);
            session.session_id = bytes.iter().map(|b| format!("{b:02x}")).collect();
            changed = true;
        }
    }
    if changed {
        save(mf)?;
    }
    match operation(mf) {
        Err(error) if is_expired(&error) => {
            renew(mf)?;
            save(mf)?;
            match operation(mf) {
                Err(error) if is_expired(&error) => {
                    relogin_flag::mark(login);
                    Err(AppError::NotReady("CONF_NEEDS_RELOGIN".into()))
                }
                result => result,
            }
        }
        result => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use std::cell::Cell;

    #[test]
    fn account_locks_serialize_same_identity_without_blocking_other_accounts() {
        let ws = std::env::temp_dir();
        let one = account_gate(&ws, "gate_one").unwrap();
        let same = account_gate(&ws, "GATE_ONE").unwrap();
        let other = account_gate(&ws, "gate_two").unwrap();
        let _held = one.lock().unwrap();
        assert!(same.try_lock().is_err());
        assert!(other.try_lock().is_ok());
        let other_workspace = account_gate(&ws.join("other"), "gate_one").unwrap();
        assert!(other_workspace.try_lock().is_ok());
    }

    #[test]
    fn password_login_only_runs_when_session_is_missing_or_rejected() {
        for code in ["CONF_NEEDS_RELOGIN", "CONF_NO_SESSION"] {
            assert_eq!(
                recover_login(Err(AppError::NotReady(code.into())), || Ok(42)).unwrap(),
                42
            );
        }
        for error in [
            AppError::NotReady("AUTH_LOCKED".into()),
            AppError::Other("HTTP 429".into()),
        ] {
            assert!(recover_login::<()>(Err(error), || panic!("must not send password")).is_err());
        }
        assert_eq!(
            recover_login(Ok(1), || panic!("session already works")).unwrap(),
            1
        );
    }

    fn fixture(exp: i64) -> MaFile {
        let token = |exp| {
            format!(
                "header.{}.signature",
                base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .encode(format!(r#"{{"exp":{exp}}}"#))
            )
        };
        let mut mf = MaFile::default();
        mf.session = Some(super::super::mafile::SessionData {
            steam_id: 1,
            session_id: String::new(),
            access_token: token(exp),
            refresh_token: token(10_000),
        });
        mf
    }

    #[test]
    fn manual_request_renews_expired_access_and_fills_missing_cookie() {
        let mut mf = fixture(100);
        let saves = Cell::new(0);
        let refreshes = Cell::new(0);
        execute(
            &mut mf,
            "manual_refresh_test",
            1000,
            |mf| {
                let s = mf.session.as_ref().unwrap();
                assert_eq!(s.access_token, "renewed");
                assert_eq!(s.session_id.len(), 24);
                Ok(())
            },
            |mf| {
                refreshes.set(refreshes.get() + 1);
                mf.session.as_mut().unwrap().access_token = "renewed".into();
                Ok(())
            },
            |_| {
                saves.set(saves.get() + 1);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(refreshes.get(), 1);
        assert_eq!(saves.get(), 1);
    }

    #[test]
    fn needauth_refreshes_and_retries_once() {
        let mut mf = fixture(2000);
        let calls = Cell::new(0);
        let refreshes = Cell::new(0);
        execute(
            &mut mf,
            "needauth_retry_test",
            1000,
            |_| {
                calls.set(calls.get() + 1);
                if calls.get() == 1 {
                    Err(AppError::NotReady("CONF_SESSION_EXPIRED".into()))
                } else {
                    Ok(())
                }
            },
            |_| {
                refreshes.set(refreshes.get() + 1);
                Ok(())
            },
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(calls.get(), 2);
        assert_eq!(refreshes.get(), 1);
        assert!(!relogin_flag::is_marked("needauth_retry_test"));
    }

    #[test]
    fn repeated_needauth_requires_login_without_an_infinite_retry() {
        let mut mf = fixture(2000);
        let calls = Cell::new(0);
        let error = execute::<()>(
            &mut mf,
            "repeated_needauth_test",
            1000,
            |_| {
                calls.set(calls.get() + 1);
                Err(AppError::NotReady("CONF_SESSION_EXPIRED".into()))
            },
            |_| Ok(()),
            |_| Ok(()),
        )
        .unwrap_err();
        assert!(error.to_string().contains("CONF_NEEDS_RELOGIN"));
        assert_eq!(calls.get(), 2);
        assert!(relogin_flag::is_marked("repeated_needauth_test"));
        relogin_flag::clear("repeated_needauth_test");
    }

    #[test]
    fn transient_refresh_failure_does_not_require_relogin() {
        let mut mf = fixture(100);
        let error = execute::<()>(
            &mut mf,
            "transient_refresh_test",
            1000,
            |_| panic!("must not request with expired token"),
            |_| Err(AppError::Other("Refresh HTTP 429".into())),
            |_| panic!("must not save failed refresh"),
        )
        .unwrap_err();
        assert!(error.to_string().contains("429"));
        assert!(!relogin_flag::is_marked("transient_refresh_test"));
        assert_eq!(
            session_state::classify(&mf, "transient_refresh_test", 1000),
            session_state::SessionState::Refreshable
        );
    }
}
