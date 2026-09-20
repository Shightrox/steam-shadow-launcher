//! Background confirmation poller.
//!
//! One long-running thread per process. It wakes up every
//! `settings.auth_poller_interval` seconds, loads the current settings, and
//! for every account with a working maFile session:
//!   1. Calls `confirmations::list`.
//!   2. Emits `auth://confirmations-changed { login, count, items }` to the UI.
//!   3. If auto-confirm is enabled for the matching confirmation type,
//!      immediately calls `respond(Allow)` and emits a toast event.
//!
//! Errors are logged and swallowed — a temporary network hiccup shouldn't
//! kill the loop.

use crate::sda::{confirmations, session, session_state, vault};
use crate::{settings, workspace};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

static RUNNING: AtomicBool = AtomicBool::new(false);

/// Per-login exponential backoff state. After repeated errors we skip the
/// account for a while so one broken session doesn't hog every tick.
#[derive(Default, Clone, Copy)]
struct Backoff {
    failures: u32,
    /// Absolute instant at which we're allowed to try again.
    next_attempt: Option<Instant>,
}

fn backoff_map() -> &'static Mutex<HashMap<String, Backoff>> {
    static CELL: OnceLock<Mutex<HashMap<String, Backoff>>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Linear doubling, clamped at 5 minutes: 15 → 30 → 60 → 120 → 240 → 300…
fn backoff_delay(failures: u32) -> Duration {
    let base = 15u64.saturating_mul(1u64 << failures.min(5));
    Duration::from_secs(base.min(300))
}

fn backoff_should_skip(login: &str) -> bool {
    let map = backoff_map().lock().unwrap();
    map.get(login)
        .and_then(|b| b.next_attempt)
        .map(|t| Instant::now() < t)
        .unwrap_or(false)
}

fn backoff_record_ok(login: &str) {
    let mut map = backoff_map().lock().unwrap();
    map.remove(login);
}

fn backoff_record_err(login: &str) {
    let mut map = backoff_map().lock().unwrap();
    let entry = map.entry(login.to_string()).or_default();
    entry.failures = entry.failures.saturating_add(1);
    entry.next_attempt = Some(Instant::now() + backoff_delay(entry.failures));
}

fn tick_signal() -> &'static Mutex<Option<std::sync::mpsc::Sender<()>>> {
    static CELL: OnceLock<Mutex<Option<std::sync::mpsc::Sender<()>>>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(None))
}

pub const EVENT_CONFIRMS: &str = "auth://confirmations-changed";
pub const EVENT_ERROR: &str = "auth://confirmations-error";
pub const EVENT_AUTO: &str = "auth://auto-confirmed";
pub const EVENT_SESSION_STATE: &str = "auth://session-state";

#[derive(Debug, Clone, Serialize)]
struct ConfChanged {
    workspace: String,
    login: String,
    count: usize,
    items: Vec<confirmations::Confirmation>,
}

#[derive(Debug, Clone, Serialize)]
struct AutoConfirmed {
    workspace: String,
    login: String,
    ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct SessionStateChanged {
    workspace: String,
    login: String,
    state: session_state::SessionState,
}

fn emit_state(
    app: &AppHandle,
    ws: &std::path::Path,
    login: &str,
    state: session_state::SessionState,
) {
    let _ = app.emit(
        EVENT_SESSION_STATE,
        SessionStateChanged {
            workspace: ws.display().to_string(),
            login: login.to_string(),
            state,
        },
    );
}

/// Start the poller. Idempotent — if already running, does nothing.
pub fn start(app: AppHandle) {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    *tick_signal().lock().unwrap() = Some(tx);
    std::thread::spawn(move || {
        tracing::info!("auth poller: started");
        loop {
            let s = match settings::load() {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("auth poller: load settings: {e}");
                    std::thread::sleep(Duration::from_secs(30));
                    continue;
                }
            };
            let interval = s.auth_poller_interval.max(15) as u64;
            if s.auth_poller_enabled {
                run_once(&app, &s, false);
            }
            // Wait with wakeup: if someone calls `poke` we rerun sooner.
            let _ = rx.recv_timeout(Duration::from_secs(interval));
        }
    });
}

/// Request an immediate rerun (e.g. from `auth_poller_configure`).
pub fn poke() {
    if let Some(tx) = tick_signal().lock().unwrap().as_ref() {
        let _ = tx.send(());
    }
}

#[derive(Debug, Clone, Serialize)]
struct ConfirmationError {
    workspace: String,
    login: String,
    message: String,
}

fn report_error(
    app: &AppHandle,
    ws: &std::path::Path,
    login: &str,
    error: &crate::error::AppError,
) {
    backoff_record_err(&format!("{}:{login}", ws.display()));
    tracing::warn!("auth poller: {login}: {error}");
    if let Ok(Some(mf)) = vault::load_plain(ws, login) {
        emit_state(
            app,
            ws,
            login,
            session_state::classify(&mf, login, session_state::now_secs()),
        );
    }
    let _ = app.emit(
        EVENT_ERROR,
        ConfirmationError {
            workspace: ws.display().to_string(),
            login: login.into(),
            message: error.to_string(),
        },
    );
}

fn run_once(app: &AppHandle, s: &settings::Settings, manual: bool) {
    let _workspace = crate::lifecycle::read();
    if settings::load().ok().and_then(|s| s.workspace) != s.workspace {
        return;
    }
    let Some(ws) = s.workspace.as_ref() else {
        return;
    };
    if let Err(e) = vault::initialize(ws, s.auth_master_password_enabled) {
        tracing::warn!("Vault unavailable: {e}");
        return;
    }
    let accounts = match workspace::list_accounts(ws) {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!("auth poller: list accounts: {e}");
            return;
        }
    };
    for a in accounts {
        if !a.has_authenticator
            || (!manual && backoff_should_skip(&format!("{}:{}", ws.display(), a.login)))
        {
            continue;
        }
        let result = session::with_ready(ws, &a.login, |mf| confirmations::list(mf, &a.login));
        let items = match result {
            Ok(items) => items,
            Err(error) => {
                report_error(app, ws, &a.login, &error);
                continue;
            }
        };
        emit_state(app, ws, &a.login, session_state::SessionState::Ok);
        let _ = app.emit(
            EVENT_CONFIRMS,
            ConfChanged {
                workspace: ws.display().to_string(),
                login: a.login.clone(),
                count: items.len(),
                items: items.clone(),
            },
        );
        if items.is_empty() || (!s.auth_auto_confirm_trades && !s.auth_auto_confirm_market) {
            backoff_record_ok(&format!("{}:{}", ws.display(), a.login));
            continue;
        }
        let result = session::with_ready(ws, &a.login, |mf| {
            let mut auto_ids = Vec::new();
            for item in &items {
                if should_auto_confirm(mf, item, s)? {
                    auto_ids.push(item.id.clone());
                }
            }
            confirmations::respond(mf, &a.login, &auto_ids, confirmations::Op::Allow)
        });
        match result {
            Ok(results) => {
                let ids = successful_ids(&results);
                if !ids.is_empty() {
                    let remaining: Vec<_> = items
                        .into_iter()
                        .filter(|item| !ids.contains(&item.id))
                        .collect();
                    let _ = app.emit(
                        EVENT_CONFIRMS,
                        ConfChanged {
                            workspace: ws.display().to_string(),
                            login: a.login.clone(),
                            count: remaining.len(),
                            items: remaining,
                        },
                    );
                    let _ = app.emit(
                        EVENT_AUTO,
                        AutoConfirmed {
                            workspace: ws.display().to_string(),
                            login: a.login.clone(),
                            ids,
                        },
                    );
                }
                if let Some(failed) = results.iter().find(|r| !r.success) {
                    report_error(
                        app,
                        ws,
                        &a.login,
                        &crate::error::AppError::Other(failed.message.clone()),
                    );
                } else {
                    backoff_record_ok(&format!("{}:{}", ws.display(), a.login));
                }
            }
            Err(error) => report_error(app, ws, &a.login, &error),
        }
    }
}

pub fn check_now(app: &AppHandle) -> crate::error::AppResult<()> {
    let mut s = settings::load()?;
    // A manual check fetches the list; it never grants automatic approval.
    s.auth_auto_confirm_trades = false;
    s.auth_auto_confirm_market = false;
    run_once(app, &s, true);
    Ok(())
}

fn successful_ids(results: &[confirmations::RespondResult]) -> Vec<String> {
    results
        .iter()
        .filter(|r| r.success)
        .map(|r| r.id.clone())
        .collect()
}

fn should_auto_confirm(
    mf: &crate::sda::mafile::MaFile,
    c: &confirmations::Confirmation,
    s: &settings::Settings,
) -> crate::error::AppResult<bool> {
    match c.kind {
        2 if s.auth_auto_confirm_trades => confirmations::is_outgoing_trade(mf, c),
        3 if s.auth_auto_confirm_market => Ok(true),
        _ => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_successful_confirmations_generate_success_events() {
        let rows = vec![
            confirmations::RespondResult {
                id: "1".into(),
                success: false,
                message: "Denied".into(),
            },
            confirmations::RespondResult {
                id: "2".into(),
                success: true,
                message: String::new(),
            },
        ];
        assert_eq!(successful_ids(&rows), vec!["2"]);
        assert!(successful_ids(&rows[..1]).is_empty());
    }

    #[test]
    fn test_and_security_confirmations_are_never_automatic() {
        let mut settings = settings::Settings::default();
        settings.auth_auto_confirm_trades = true;
        settings.auth_auto_confirm_market = true;
        for kind in [1, 4, 5, 6, 9, 11, 99] {
            let row: confirmations::Confirmation = serde_json::from_value(serde_json::json!({
                "id":"1", "nonce":"2", "type":kind,
            }))
            .unwrap();
            assert!(!should_auto_confirm(&Default::default(), &row, &settings).unwrap());
        }
    }
}
