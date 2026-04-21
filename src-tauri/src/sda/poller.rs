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

use crate::sda::{confirmations, vault};
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
        let Ok(Some(mf)) = vault::load_plain(ws, &a.login) else {
            continue;
        };
        // Skip half-enrolled accounts — the secrets are real but the server
        // hasn't been told to trust them yet, so listing confirmations would
        // 403 forever and just spam backoff.
        if mf.fully_enrolled == Some(false) {
            continue;
        }
        // Skip if there's no session — we'd just get `CONF_NO_SESSION`.
        if mf.session.as_ref().map(|s| s.access_token.is_empty()).unwrap_or(true) {
            continue;
        }
        match confirmations::list(&mf) {
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
                    match confirmations::respond(&mf, &auto_ids, confirmations::Op::Allow) {
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
