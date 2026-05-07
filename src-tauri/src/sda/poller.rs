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

use crate::sda::{confirmations, login as sda_login, relogin_flag, session_state, vault};
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
pub const EVENT_AUTO: &str = "auth://auto-confirmed";
pub const EVENT_SESSION_STATE: &str = "auth://session-state";

#[derive(Debug, Clone, Serialize)]
struct ConfChanged {
    login: String,
    count: usize,
    items: Vec<confirmations::Confirmation>,
}

#[derive(Debug, Clone, Serialize)]
struct AutoConfirmed {
    login: String,
    ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct SessionStateChanged {
    login: String,
    state: session_state::SessionState,
}

fn emit_state(app: &AppHandle, login: &str, state: session_state::SessionState) {
    let _ = app.emit(
        EVENT_SESSION_STATE,
        SessionStateChanged {
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
                run_once(&app, &s);
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

fn run_once(app: &AppHandle, s: &settings::Settings) {
    let Some(ws) = s.workspace.as_ref() else { return };
    let accounts = match workspace::list_accounts(ws) {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!("auth poller: list accounts: {e}");
            return;
        }
    };
    for a in accounts {
        if !a.has_authenticator {
            continue;
        }
        if backoff_should_skip(&a.login) {
            continue;
        }
        let Ok(Some(mut mf)) = vault::load_plain(ws, &a.login) else {
            continue;
        };
        // Skip half-enrolled accounts — the secrets are real but the server
        // hasn't been told to trust them yet, so listing confirmations would
        // 403 forever and just spam backoff.
        if mf.fully_enrolled == Some(false) {
            continue;
        }

        // Classify session and either silently refresh, give up, or proceed.
        let now = session_state::now_secs();
        let st = session_state::classify(&mf, &a.login, now);
        match st {
            session_state::SessionState::Refreshable => {
                let (refresh, sid) = mf
                    .session
                    .as_ref()
                    .map(|s| (s.refresh_token.clone(), s.steam_id.to_string()))
                    .unwrap_or_default();
                match sda_login::refresh_access_token(&refresh, &sid) {
                    Ok(new_tok) => {
                        if let Some(s) = mf.session.as_mut() {
                            s.access_token = new_tok;
                        }
                        if let Err(e) = vault::save_plain(ws, &a.login, &mf) {
                            tracing::warn!(
                                "auth poller: save refreshed mafile {}: {}",
                                a.login,
                                e
                            );
                        }
                        relogin_flag::clear(&a.login);
                        emit_state(app, &a.login, session_state::SessionState::Ok);
                        tracing::info!("auth poller: silent-refreshed access for {}", a.login);
                        // fall through to confirmations::list below
                    }
                    Err(e) => {
                        tracing::warn!("auth poller: refresh failed for {}: {}", a.login, e);
                        relogin_flag::mark(&a.login);
                        emit_state(app, &a.login, session_state::SessionState::NeedsRelogin);
                        continue;
                    }
                }
            }
            session_state::SessionState::NeedsRelogin => {
                emit_state(app, &a.login, session_state::SessionState::NeedsRelogin);
                continue;
            }
            session_state::SessionState::NoSession => {
                emit_state(app, &a.login, session_state::SessionState::NoSession);
                continue;
            }
            session_state::SessionState::Ok => {
                emit_state(app, &a.login, session_state::SessionState::Ok);
            }
        }

        match confirmations::list(&mf, &a.login) {
            Ok(items) => {
                backoff_record_ok(&a.login);
                let _ = app.emit(
                    EVENT_CONFIRMS,
                    ConfChanged {
                        login: a.login.clone(),
                        count: items.len(),
                        items: items.clone(),
                    },
                );
                // Handle auto-confirm for the enabled types.
                let auto_ids: Vec<String> = items
                    .iter()
                    .filter(|c| should_auto_confirm(&mf, c, s))
                    .map(|c| c.id.clone())
                    .collect();
                if !auto_ids.is_empty() {
                    match confirmations::respond(&mf, &a.login, &auto_ids, confirmations::Op::Allow) {
                        Ok(_) => {
                            tracing::info!(
                                "auth poller: auto-confirmed {} items for {}",
                                auto_ids.len(),
                                a.login
                            );
                            let _ = app.emit(
                                EVENT_AUTO,
                                AutoConfirmed {
                                    login: a.login.clone(),
                                    ids: auto_ids,
                                },
                            );
                        }
                        Err(e) => tracing::warn!(
                            "auth poller: auto-confirm failed for {}: {}",
                            a.login,
                            e
                        ),
                    }
                }
            }
            Err(e) => {
                backoff_record_err(&a.login);
                let msg = e.to_string();
                // "no session" / "needs relogin" aren't noisy warnings —
                // the UI already shows those.
                if !msg.contains("CONF_NEEDS_RELOGIN") && !msg.contains("CONF_NO_SESSION") {
                    tracing::debug!("auth poller: list {}: {}", a.login, msg);
                }
            }
        }
    }
}

fn should_auto_confirm(
    mf: &crate::sda::mafile::MaFile,
    c: &confirmations::Confirmation,
    s: &settings::Settings,
) -> bool {
    match c.kind {
        // Trade offer: ONLY auto-confirm if it's one we initiated (creator ==
        // our own steam_id). Incoming trades are never touched.
        1 | 2 if s.auth_auto_confirm_trades => {
            let our_sid = mf.session.as_ref().map(|s| s.steam_id).unwrap_or(0);
            our_sid != 0 && c.creator_id == our_sid.to_string()
        }
        // Market listing: confirm any — we only list items we own.
        3 if s.auth_auto_confirm_market => true,
        _ => false,
    }
}
