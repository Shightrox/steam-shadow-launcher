//! Adding a brand-new Steam Mobile Authenticator to an account.
//!
//! The flow (three phases, UI-driven):
//!
//! ### Phase A — Phone (optional, skipped if already attached)
//! 1. `IPhoneService/SetAccountPhoneNumber/v1` — user submits phone + country code.
//!    Steam sends an email to the account owner asking to approve.
//! 2. Poll `IPhoneService/IsAccountWaitingForEmailConfirmation/v1` until
//!    `awaiting_email_confirmation=false`.
//! 3. `IPhoneService/SendPhoneVerificationCode/v1` — triggers SMS.
//! 4. User submits SMS code → `IPhoneService/VerifyAccountPhoneWithCode/v1`.
//!
//! ### Phase B — AddAuthenticator
//! `ITwoFactorService/AddAuthenticator/v1` with `authenticator_type=1`.
//! Response contains `shared_secret`, `identity_secret`, `revocation_code`,
//! `uri`, `serial_number`, `secret_1`, `server_time`, `account_name`,
//! `token_gid`. We build a complete `MaFile` from this + the session tokens.
//!
//! **Status codes** (`response.status`):
//! - 1  = OK
//! - 2  = Need phone first (retry Phase A)
//! - 29 = Account is already using an authenticator (user must remove via
//!        `ITwoFactorService/RemoveAuthenticator/v1` + revocation code)
//! - 84 = Rate-limited
//!
//! ### Phase C — Finalize
//! `ITwoFactorService/FinalizeAddAuthenticator/v1` with:
//!   - steamid
//!   - authenticator_code = TOTP(shared_secret, server_time)
//!   - authenticator_time = server_time
//!   - activation_code    = SMS code the user just received
//!   - validate_sms_code  = true (on the first attempt)
//!
//! Steam may return status=89 (`TwoFactorActivationCodeMismatch` — retry SMS),
//! or `want_more=true` (server drift; retry with a newer server time).
//!
//! The caller (Tauri command layer) is responsible for persisting the maFile
//! **only after** a successful Finalize call — otherwise we'd leak secrets for
//! a half-baked authenticator.

use crate::error::{AppError, AppResult};
use crate::http;
use crate::sda::mafile::{MaFile, SessionData};
use crate::sda::totp;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

const API: &str = "https://api.steampowered.com";

// ── Per-login session registry ────────────────────────────────────────────
//
// The full AddAuthenticator result (including shared_secret, identity_secret,
// revocation_code) must NEVER leave the Rust process — if we round-tripped
// secrets to the TS layer we'd have no way of clearing them from JS heap.
// Instead we cache the whole `AddAuthResult` here, keyed by shadow-account
// login, and only expose opaque fields (phone_number_hint, revocation_code
// to display-once) via the Tauri command layer.

#[derive(Default)]
pub struct AddSession {
    pub access_token: String,
    pub refresh_token: String,
    pub steam_id: u64,
    pub session_id: String,
    /// Populated after `auth_add_create` succeeds.
    pub add: Option<AddAuthResult>,
    /// Device id we generated for AddAuthenticator and must reuse in the
    /// final maFile (to match the server's record).
    pub device_id: String,
    /// The `allowed_confirmations` Steam returned during the login that
    /// seeded this session. Authoritative signal for "is Email Guard on?"
    /// (QueryStatus only reports *mobile* authenticator state — it can't
    /// distinguish "Email Guard on" from "no Guard at all".)
    ///
    /// EAuthSessionGuardType values:
    ///   1 = None, 2 = EmailCode, 3 = DeviceCode,
    ///   4 = DeviceConfirmation, 5 = EmailConfirmation, 6 = MachineToken.
    pub login_confirmations: Vec<i64>,
    pub finalized: bool,
}

fn registry() -> &'static Mutex<HashMap<String, AddSession>> {
    static CELL: OnceLock<Mutex<HashMap<String, AddSession>>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Seed a fresh AddSession with tokens obtained through the normal login flow.
/// Overwrites any prior session for the same login.
pub fn put_session(
    login: &str,
    access_token: String,
    refresh_token: String,
    steam_id: u64,
    session_id: String,
    login_confirmations: Vec<i64>,
) {
    let sess = AddSession {
        access_token,
        refresh_token,
        steam_id,
        session_id,
        add: None,
        device_id: new_device_id(),
        login_confirmations,
        finalized: false,
    };
    registry().lock().unwrap().insert(login.to_string(), sess);
}

pub fn drop_session(login: &str) {
    registry().lock().unwrap().remove(login);
}

pub fn clear_sessions() {
    registry().lock().unwrap().clear();
}

fn with_session<F, T>(login: &str, f: F) -> AppResult<T>
where
    F: FnOnce(&mut AddSession) -> AppResult<T>,
{
    let mut map = registry().lock().unwrap();
    let sess = map
        .get_mut(login)
        .ok_or_else(|| AppError::NotReady("ADD_NO_SESSION".into()))?;
    f(sess)
}

fn access_token_of(login: &str) -> AppResult<String> {
    with_session(login, |s| Ok(s.access_token.clone()))
}

// ── Shared wire types ─────────────────────────────────────────────────────

fn authed_post(path: &str, access_token: &str) -> reqwest::blocking::RequestBuilder {
    let url = format!("{API}{path}?access_token={access_token}");
    http::shared()
        .post(url)
        .timeout(std::time::Duration::from_secs(20))
}

fn parse_response<T: for<'de> Deserialize<'de> + Default>(body: &str, label: &str) -> AppResult<T> {
    #[derive(Deserialize)]
    struct Wrap<R> {
        #[serde(default = "Option::default")]
        response: Option<R>,
    }
    let wrap: Wrap<T> = serde_json::from_str(body).map_err(|e| {
        AppError::Other(format!(
            "{label}: invalid response (line {}, column {})",
            e.line(),
            e.column()
        ))
    })?;
    Ok(wrap.response.unwrap_or_default())
}

// ── Phase A: phone ────────────────────────────────────────────────────────

#[derive(Debug, Default, Deserialize, Serialize, Clone)]
pub struct SetPhoneResult {
    #[serde(default)]
    pub confirmation_email_address: String,
    #[serde(default)]
    pub phone_number_formatted: String,
}

/// Attach a phone number. Triggers an email confirmation to the account owner.
pub fn set_account_phone_number(
    access_token: &str,
    phone_number: &str,
    phone_country_code: &str,
) -> AppResult<SetPhoneResult> {
    let form = [
        ("phone_number", phone_number.to_string()),
        ("phone_country_code", phone_country_code.to_string()),
    ];
    let resp = authed_post("/IPhoneService/SetAccountPhoneNumber/v1/", access_token)
        .form(&form)
        .send()
        .map_err(|e| AppError::Other(format!("SetPhone: {}", e.without_url())))?;
    let status = resp.status();
    let body = resp.text().unwrap_or_default();
    if !status.is_success() {
        return Err(AppError::Other(format!("SET_PHONE_FAIL: HTTP {status}")));
    }
    parse_response::<SetPhoneResult>(&body, "SetPhone")
}

#[derive(Debug, Default, Deserialize, Serialize, Clone)]
pub struct WaitingEmailResult {
    #[serde(default)]
    pub awaiting_email_confirmation: bool,
    #[serde(default)]
    pub seconds_to_wait: i64,
}

/// Poll whether we're still waiting on the email-confirmation step.
pub fn is_waiting_for_email_confirmation(access_token: &str) -> AppResult<WaitingEmailResult> {
    let resp = authed_post(
        "/IPhoneService/IsAccountWaitingForEmailConfirmation/v1/",
        access_token,
    )
    .header("Content-Length", "0")
    .send()
    .map_err(|e| AppError::Other(format!("WaitEmail: {}", e.without_url())))?;
    let body = resp.text().unwrap_or_default();
    parse_response(&body, "WaitEmail")
}

/// Send the phone-verification SMS. (Called after email has been confirmed.)
pub fn send_phone_verification_code(access_token: &str) -> AppResult<()> {
    let resp = authed_post("/IPhoneService/SendPhoneVerificationCode/v1/", access_token)
        .form(&[("language", "0")])
        .send()
        .map_err(|e| AppError::Other(format!("SendSMS: {}", e.without_url())))?;
    if !resp.status().is_success() {
        return Err(AppError::Other(format!(
            "SEND_SMS_FAIL: HTTP {}",
            resp.status()
        )));
    }
    Ok(())
}

/// Submit the SMS code the user received for the phone-verification step.
pub fn verify_phone_with_code(access_token: &str, code: &str) -> AppResult<()> {
    let resp = authed_post(
        "/IPhoneService/VerifyAccountPhoneWithCode/v1/",
        access_token,
    )
    .form(&[("code", code.trim())])
    .send()
    .map_err(|e| AppError::Other(format!("VerifyPhone: {}", e.without_url())))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(AppError::Other(format!("VERIFY_PHONE_FAIL: HTTP {status}")));
    }
    Ok(())
}

// ── Phase B: AddAuthenticator ─────────────────────────────────────────────

#[derive(Debug, Default, Deserialize, Serialize, Clone)]
pub struct AddAuthResult {
    /// 1 = OK, 2 = need phone, 29 = already has authenticator, 84 = rate-limit.
    #[serde(default)]
    pub status: i64,
    #[serde(default)]
    pub shared_secret: String,
    #[serde(default)]
    pub identity_secret: String,
    #[serde(default)]
    pub revocation_code: String,
    #[serde(default)]
    pub uri: String,
    #[serde(default)]
    pub serial_number: String,
    #[serde(default)]
    pub secret_1: String,
    #[serde(default)]
    pub server_time: String,
    #[serde(default)]
    pub account_name: String,
    #[serde(default)]
    pub token_gid: String,
    #[serde(default)]
    pub phone_number_hint: String,
}

/// Step B. Call AFTER phone is attached. Returns the freshly-minted secrets
/// (not yet activated — Finalize is still required).
#[allow(dead_code)] // public API; kept for symmetry with the SDA wire docs
pub fn add_authenticator(access_token: &str, steam_id: &str) -> AppResult<AddAuthResult> {
    add_authenticator_with(access_token, steam_id, &new_device_id())
}

fn add_authenticator_with(
    access_token: &str,
    steam_id: &str,
    device_id: &str,
) -> AppResult<AddAuthResult> {
    let form = [
        ("steamid", steam_id.to_string()),
        ("authenticator_type", "1".into()), // 1 = mobile
        ("device_identifier", device_id.to_string()),
        ("sms_phone_id", "1".into()),
    ];
    let resp = authed_post("/ITwoFactorService/AddAuthenticator/v1/", access_token)
        .form(&form)
        .send()
        .map_err(|e| AppError::Other(format!("AddAuth: {}", e.without_url())))?;
    let status = resp.status();
    let body = resp.text().unwrap_or_default();
    if !status.is_success() {
        return Err(AppError::Other(format!("ADD_AUTH_FAIL: HTTP {status}")));
    }
    let out: AddAuthResult = parse_response(&body, "AddAuth")?;
    match out.status {
        1 => Ok(out),
        2 => Err(AppError::NotReady("ADD_AUTH_NEED_PHONE".into())),
        29 => Err(AppError::Other("ADD_AUTH_ALREADY_HAS_AUTHENTICATOR".into())),
        84 => Err(AppError::Other("ADD_AUTH_RATE_LIMIT".into())),
        other => Err(AppError::Other(format!("ADD_AUTH_STATUS_{other}"))),
    }
}

/// Generate a plausible `android:<uuidv4>` device identifier.
fn new_device_id() -> String {
    use rand::RngCore;
    let mut buf = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut buf);
    // Version 4 UUID tags.
    buf[6] = (buf[6] & 0x0F) | 0x40;
    buf[8] = (buf[8] & 0x3F) | 0x80;
    let h = |r: &[u8]| r.iter().map(|b| format!("{:02x}", b)).collect::<String>();
    format!(
        "android:{}-{}-{}-{}-{}",
        h(&buf[0..4]),
        h(&buf[4..6]),
        h(&buf[6..8]),
        h(&buf[8..10]),
        h(&buf[10..16]),
    )
}

// ── Phase C: Finalize ─────────────────────────────────────────────────────

#[derive(Debug, Default, Deserialize, Clone)]
pub struct FinalizeResult {
    #[serde(default)]
    pub status: i64,
    #[allow(dead_code)] // reported by Steam; we log it but don't act on it
    #[serde(default)]
    pub server_time: String,
    #[serde(default)]
    pub want_more: bool,
    #[serde(default)]
    pub success: bool,
}

/// Finalize. Retry loop is the caller's responsibility — we report back
/// `want_more` and `status` so the UI can drive the next attempt.
///
/// `validate_sms` = false attempts a "no-phone" finalize: some accounts
/// (esp. those with only-email guard, no phone ever attached) accept an
/// empty `activation_code` this way. If Steam refuses (status 89), the
/// UI must fall back to the phone flow.
pub fn finalize_add(
    access_token: &str,
    steam_id: &str,
    shared_secret: &str,
    activation_code: &str,
    try_number: u32,
    validate_sms: bool,
) -> AppResult<FinalizeResult> {
    let server_time = totp::server_time();
    let code = totp::generate_code_at(shared_secret, server_time)?;
    let form = [
        ("steamid", steam_id.to_string()),
        ("authenticator_code", code),
        ("authenticator_time", server_time.to_string()),
        ("activation_code", activation_code.trim().to_string()),
        (
            "validate_sms_code",
            if validate_sms { "1".into() } else { "0".into() },
        ),
        ("try_number", try_number.to_string()),
    ];
    let resp = authed_post(
        "/ITwoFactorService/FinalizeAddAuthenticator/v1/",
        access_token,
    )
    .form(&form)
    .send()
    .map_err(|e| AppError::Other(format!("Finalize: {}", e.without_url())))?;
    let status = resp.status();
    let body = resp.text().unwrap_or_default();
    if !status.is_success() {
        return Err(AppError::Other(format!("FINALIZE_FAIL: HTTP {status}")));
    }
    let out: FinalizeResult = parse_response(&body, "Finalize")?;
    Ok(out)
}

/// Build an SDA-compatible `MaFile` from a finalized AddAuthenticator result
/// and the logged-in session tokens.
pub fn mafile_from_add(
    add: &AddAuthResult,
    steam_id: u64,
    access_token: String,
    refresh_token: String,
    session_id: String,
    device_id: String,
    fully_enrolled: bool,
) -> MaFile {
    MaFile {
        shared_secret: add.shared_secret.clone(),
        identity_secret: add.identity_secret.clone(),
        account_name: add.account_name.clone(),
        device_id,
        revocation_code: add.revocation_code.clone(),
        serial_number: add.serial_number.clone(),
        uri: add.uri.clone(),
        server_time: add.server_time.parse::<u64>().ok(),
        token_gid: add.token_gid.clone(),
        secret_1: add.secret_1.clone(),
        status: Some(add.status),
        phone_number_hint: add.phone_number_hint.clone(),
        confirm_type: None,
        fully_enrolled: Some(fully_enrolled),
        recovery_pending: true,
        session: Some(SessionData {
            steam_id,
            access_token,
            refresh_token,
            session_id,
        }),
    }
}

// ── High-level (login-keyed) wrappers used by Tauri commands ─────────────

/// Phone-number hint state for the wizard ("on which step are we?").
#[derive(Debug, Clone, Serialize)]
pub struct PhoneState {
    pub awaiting_email: bool,
    pub seconds_to_wait: i64,
}

// ── Diagnostic ───────────────────────────────────────────────────────────
//
// Called immediately after login succeeds. Combines `ITwoFactorService/QueryStatus`
// and `IPhoneService/AccountPhoneStatus` into a single struct so the UI can
// present a deterministic "what's blocking SDA attach" panel and pick the
// correct flow (no-phone email-Guard fast path, phone-required, blocker).

#[derive(Debug, Default, Deserialize)]
struct QueryStatusInner {
    /// 0 = no Guard, 1 = email Guard, 2 = mobile authenticator already on
    #[serde(default)]
    state: i64,
    #[serde(default)]
    authenticator_type: i64,
    #[serde(default)]
    steamguard_scheme: i64,
    /// Convenience bool — is mobile auth currently bound?
    #[serde(default)]
    authenticator_allowed: bool,
}

#[derive(Debug, Default, Deserialize)]
struct PhoneStatusInner {
    #[serde(default)]
    verified: bool,
    #[serde(default)]
    phone_number: String,
}

/// Public diagnostic snapshot for the UI.
#[derive(Debug, Clone, Serialize)]
pub struct AddDiagnostic {
    /// "none" | "email" | "mobile"
    pub guard: &'static str,
    /// True iff the account already has a working mobile authenticator.
    /// Triggers the "revoke first" blocker.
    pub already_has_mobile: bool,
    /// True iff the account has a verified phone (so the SMS path will work).
    pub phone_attached: bool,
    /// Last digits / hint string for display ("***1234"). Empty if no phone.
    pub phone_hint: String,
    /// Suggested wizard path: "phone-required" | "no-phone-fast" | "blocker-no-guard"
    /// | "blocker-already-mobile".
    pub suggested_path: &'static str,
}

pub fn diagnose(login: &str) -> AppResult<AddDiagnostic> {
    // 2-second throttle: the UI tends to call this on every phase change of
    // the wizard, which can fire 3-4 times per real user action. Cache the
    // last successful snapshot per login and return it verbatim if asked
    // again within the window.
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    static CACHE: OnceLock<Mutex<HashMap<String, (Instant, AddDiagnostic)>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    {
        let guard = cache.lock().unwrap();
        if let Some((t, d)) = guard.get(login) {
            if t.elapsed() < Duration::from_secs(2) {
                return Ok(d.clone());
            }
        }
    }
    let fresh = with_fresh_token(login, "auth_add_diagnose", || diagnose_once(login))?;
    cache
        .lock()
        .unwrap()
        .insert(login.to_string(), (Instant::now(), fresh.clone()));
    Ok(fresh)
}

fn refresh_session(login: &str) -> AppResult<()> {
    let (refresh, sid) = with_session(login, |s| {
        Ok((s.refresh_token.clone(), s.steam_id.to_string()))
    })?;
    if refresh.is_empty() {
        return Err(AppError::NotReady("ADD_NO_REFRESH_TOKEN".into()));
    }
    let new_token = crate::sda::login::refresh_access_token(&refresh, &sid)?;
    with_session(login, |s| {
        s.access_token = new_token;
        Ok(())
    })
}

/// Run `op` once. If it returns a 401-ish error, refresh the access_token via
/// the cached refresh_token and run it again. Used to wrap every Steam call
/// in this module that depends on the AddSession's `access_token` — Steam
/// rotates it whenever Guard settings change on the website, which is exactly
/// what the user is doing in parallel with the wizard.
fn with_fresh_token<F, T>(login: &str, label: &str, mut op: F) -> AppResult<T>
where
    F: FnMut() -> AppResult<T>,
{
    match op() {
        Err(AppError::Other(msg)) if msg.contains("HTTP 401") => {
            tracing::warn!("{label}: HTTP 401, refreshing access_token");
            refresh_session(login)?;
            op()
        }
        other => other,
    }
}

fn diagnose_once(login: &str) -> AppResult<AddDiagnostic> {
    let (tok, sid, login_confirms) = with_session(login, |s| {
        Ok((
            s.access_token.clone(),
            s.steam_id.to_string(),
            s.login_confirmations.clone(),
        ))
    })?;

    tracing::info!(
        "auth_add_diagnose: querying status for {login} (login_confirms={:?})",
        login_confirms
    );

    // ── QueryStatus. Requires `steamid` in the form body.
    let status_resp = authed_post("/ITwoFactorService/QueryStatus/v1/", &tok)
        .form(&[("steamid", sid.as_str())])
        .send()
        .map_err(|e| AppError::Other(format!("QueryStatus: {}", e.without_url())))?;
    let status_code = status_resp.status();
    let status_body = status_resp.text().unwrap_or_default();
    tracing::info!("auth_add_diagnose: QueryStatus HTTP {status_code}");
    if !status_code.is_success() {
        return Err(AppError::Other(format!("QueryStatus HTTP {status_code}")));
    }
    let qs: QueryStatusInner = parse_response(&status_body, "QueryStatus").unwrap_or_default();

    tracing::info!(
        "auth_add_diagnose: QueryStatus state={} auth_type={} scheme={} allowed={}",
        qs.state,
        qs.authenticator_type,
        qs.steamguard_scheme,
        qs.authenticator_allowed,
    );

    // ── AccountPhoneStatus (best-effort).
    let phone = match authed_post("/IPhoneService/AccountPhoneStatus/v1/", &tok)
        .form(&[("include_verified", "true")])
        .send()
    {
        Ok(r) => {
            let rc = r.status();
            let b = r.text().unwrap_or_default();
            tracing::info!("auth_add_diagnose: PhoneStatus HTTP {rc}");
            if rc.is_success() {
                parse_response::<PhoneStatusInner>(&b, "PhoneStatus").unwrap_or_default()
            } else {
                PhoneStatusInner::default()
            }
        }
        Err(e) => {
            tracing::warn!("auth_add_diagnose: PhoneStatus err: {}", e.without_url());
            PhoneStatusInner::default()
        }
    };

    // Mobile authenticator already bound? `state == 2` is the canonical signal,
    // but `authenticator_type != 0` is also a hard sign.
    let already_has_mobile = qs.state == 2 || qs.authenticator_type == 1;

    // QueryStatus only reports the *mobile* authenticator state — it returns
    // `state=0` for both "no Guard at all" AND "Email Guard on". To tell them
    // apart we look at the login-time `allowed_confirmations`: if Steam asked
    // for EmailCode (2) or EmailConfirmation (5) during this same login, Email
    // Guard is definitely active.
    const GUARD_EMAIL_CODE: i64 = 2;
    const GUARD_EMAIL_CONFIRM: i64 = 5;
    const GUARD_DEVICE_CODE: i64 = 3;
    const GUARD_DEVICE_CONFIRM: i64 = 4;
    let login_email_guard = login_confirms
        .iter()
        .any(|c| *c == GUARD_EMAIL_CODE || *c == GUARD_EMAIL_CONFIRM);
    let login_mobile_guard = login_confirms
        .iter()
        .any(|c| *c == GUARD_DEVICE_CODE || *c == GUARD_DEVICE_CONFIRM);

    // Guard label. QueryStatus is authoritative for "mobile", login hints are
    // authoritative for "email vs none".
    let guard = if already_has_mobile || login_mobile_guard {
        "mobile"
    } else if login_email_guard || qs.state == 1 {
        "email"
    } else {
        "none"
    };

    let phone_attached = phone.verified && !phone.phone_number.is_empty();
    let phone_hint = phone.phone_number;

    let suggested_path = if already_has_mobile {
        "blocker-already-mobile"
    } else if guard == "none" && !phone_attached {
        // Steam refuses AddAuthenticator without ANY guard at all and no phone.
        // Email Guard or a phone is required.
        "blocker-no-guard"
    } else if phone_attached {
        // Phone is verified — SMS flow will work straight away.
        "phone-required"
    } else {
        // Email Guard is on and no phone — try the no-SMS / email-Guard path.
        "no-phone-fast"
    };

    let _ = qs.steamguard_scheme; // silence unused warning
    let _ = qs.authenticator_allowed;

    Ok(AddDiagnostic {
        guard,
        already_has_mobile,
        phone_attached,
        phone_hint,
        suggested_path,
    })
}

pub fn add_set_phone(login: &str, number: &str, country_code: &str) -> AppResult<SetPhoneResult> {
    with_fresh_token(login, "auth_add_set_phone", || {
        let tok = access_token_of(login)?;
        set_account_phone_number(&tok, number, country_code)
    })
}

pub fn add_check_email(login: &str) -> AppResult<PhoneState> {
    with_fresh_token(login, "auth_add_check_email", || {
        let tok = access_token_of(login)?;
        let r = is_waiting_for_email_confirmation(&tok)?;
        Ok(PhoneState {
            awaiting_email: r.awaiting_email_confirmation,
            seconds_to_wait: r.seconds_to_wait,
        })
    })
}

pub fn add_send_sms(login: &str) -> AppResult<()> {
    with_fresh_token(login, "auth_add_send_sms", || {
        let tok = access_token_of(login)?;
        send_phone_verification_code(&tok)
    })
}

pub fn add_verify_phone(login: &str, code: &str) -> AppResult<()> {
    with_fresh_token(login, "auth_add_verify_phone", || {
        let tok = access_token_of(login)?;
        verify_phone_with_code(&tok, code)
    })
}

/// What we expose to JS after `AddAuthenticator/v1` succeeds. The actual
/// secrets stay locked inside the Rust registry; JS only learns the phone
/// hint (so the UI can echo "we sent SMS to ***1234").
#[derive(Debug, Clone, Serialize)]
pub struct AddCreatePublic {
    pub phone_number_hint: String,
    pub server_time: String,
}

pub fn add_create(login: &str) -> AppResult<AddCreatePublic> {
    if let Some(existing) = with_session(login, |s| Ok(s.add.clone()))? {
        return Ok(AddCreatePublic {
            phone_number_hint: existing.phone_number_hint,
            server_time: existing.server_time,
        });
    }
    with_fresh_token(login, "auth_add_create", || {
        let (tok, sid, device) = with_session(login, |s| {
            Ok((
                s.access_token.clone(),
                s.steam_id.to_string(),
                s.device_id.clone(),
            ))
        })?;
        let mut res = add_authenticator_with(&tok, &sid, &device)?;
        if res.account_name.is_empty() {
            res.account_name = login.to_string();
        }
        let public = AddCreatePublic {
            phone_number_hint: res.phone_number_hint.clone(),
            server_time: res.server_time.clone(),
        };
        with_session(login, |s| {
            s.add = Some(res);
            Ok(())
        })?;
        Ok(public)
    })
}

/// Persist a **partial** maFile with `fully_enrolled=false` immediately after
/// `add_create` succeeds. Prevents catastrophic data loss if the user closes
/// the wizard on the revocation screen — Steam has already bound the
/// authenticator to the account by that point, so losing the secrets means
/// losing access until the user uses the revocation code (which they only see
/// after this partial-write lands on disk via the revocation UI).
///
/// Safe to call: overwrites any prior maFile atomically.
pub fn add_persist_partial(login: &str, workspace: &std::path::Path) -> AppResult<()> {
    let (add, tok, refresh, sid, sess_id, device) = with_session(login, |s| {
        let add = s
            .add
            .as_ref()
            .ok_or_else(|| AppError::NotReady("ADD_NOT_CREATED".into()))?
            .clone();
        Ok((
            add,
            s.access_token.clone(),
            s.refresh_token.clone(),
            s.steam_id,
            s.session_id.clone(),
            s.device_id.clone(),
        ))
    })?;
    let mf = mafile_from_add(&add, sid, tok, refresh, sess_id, device, false);
    save_recovery(workspace, login, &mf)?;
    crate::sda::vault::save_plain(workspace, login, &mf)?;
    // NOTE: we deliberately do NOT call set_authenticator_meta here — the
    // account isn't fully bound yet, and we don't want the poller to pick it
    // up and start TOTP'ing with an un-finalized secret.
    Ok(())
}

/// What we expose to JS after `FinalizeAddAuthenticator/v1`. Crucially this
/// is where `revocation_code` makes its one and only trip up to the UI — the
/// user *must* write it down.
#[derive(Debug, Clone, Serialize)]
pub struct AddFinalizePublic {
    pub success: bool,
    pub want_more: bool,
    pub status: i64,
    /// Only filled when `success == true`.
    pub revocation_code: Option<String>,
}

pub fn add_finalize(
    login: &str,
    sms_code: &str,
    try_number: u32,
    validate_sms: bool,
) -> AppResult<AddFinalizePublic> {
    if with_session(login, |s| Ok(s.finalized))? {
        return with_session(login, |s| {
            Ok(AddFinalizePublic {
                success: true,
                want_more: false,
                status: 1,
                revocation_code: s.add.as_ref().map(|a| a.revocation_code.clone()),
            })
        });
    }
    if try_number == 0 || try_number > 5 {
        return Err(AppError::NotReady("ADD_RETRY_LIMIT".into()));
    }
    with_fresh_token(login, "auth_add_finalize", || {
        let (tok, sid, secret) = with_session(login, |s| {
            let add = s
                .add
                .as_ref()
                .ok_or_else(|| AppError::NotReady("ADD_NOT_CREATED".into()))?;
            Ok((
                s.access_token.clone(),
                s.steam_id.to_string(),
                add.shared_secret.clone(),
            ))
        })?;
        let r = finalize_add(&tok, &sid, &secret, sms_code, try_number, validate_sms)?;
        let revocation = if r.success {
            with_session(login, |s| {
                s.finalized = true;
                Ok(s.add.as_ref().map(|a| a.revocation_code.clone()))
            })?
        } else {
            None
        };
        Ok(AddFinalizePublic {
            success: r.success,
            want_more: r.want_more,
            status: r.status,
            revocation_code: revocation,
        })
    })
}

/// Persist immediately after activation, before exposing success to the UI.
/// A separate acknowledgement records that the recovery code was written down.
///
/// Returns the path of the saved maFile so the caller can log it.
pub fn add_persist(login: &str, workspace: &std::path::Path) -> AppResult<()> {
    if !with_session(login, |s| Ok(s.finalized))? {
        return Err(AppError::NotReady("CONF_NOT_ENROLLED".into()));
    }
    let (add, tok, refresh, sid, sess_id, device) = with_session(login, |s| {
        let add = s
            .add
            .as_ref()
            .ok_or_else(|| AppError::NotReady("ADD_NOT_CREATED".into()))?
            .clone();
        Ok((
            add,
            s.access_token.clone(),
            s.refresh_token.clone(),
            s.steam_id,
            s.session_id.clone(),
            s.device_id.clone(),
        ))
    })?;
    let mf = mafile_from_add(&add, sid, tok, refresh, sess_id, device, true);
    save_recovery(workspace, login, &mf)?;
    crate::sda::vault::save_plain(workspace, login, &mf)?;
    crate::workspace::set_authenticator_meta(
        workspace,
        login,
        true,
        Some(mf.account_name.clone()),
    )?;
    Ok(())
}

fn recovery_path(ws: &std::path::Path, login: &str) -> AppResult<std::path::PathBuf> {
    Ok(crate::workspace::auth_dir(ws, login)?.join("enrollment.dpapi"))
}
fn save_recovery(ws: &std::path::Path, login: &str, mf: &MaFile) -> AppResult<()> {
    crate::workspace::validate_auth_identity(ws, login, mf)?;
    let json = zeroize::Zeroizing::new(mf.to_json_pretty()?);
    crate::storage::atomic_write(
        &recovery_path(ws, login)?,
        &super::credentials::protect(json.as_bytes(), false)?,
    )
}

#[derive(Serialize)]
pub struct Resume {
    pub phase: &'static str,
    pub phone_hint: String,
    pub revocation_code: Option<String>,
}
pub fn resume(ws: &std::path::Path, login: &str) -> AppResult<Resume> {
    super::vault::ensure_writable(ws)?;
    let recovery = recovery_path(ws, login)?;
    let mf = if recovery.exists() {
        let bytes = super::credentials::protect(&std::fs::read(&recovery)?, true)?;
        Some(MaFile::from_json_bytes(&bytes)?)
    } else {
        super::vault::load_plain(ws, login)?
    };
    let Some(mf) = mf.filter(|mf| mf.fully_enrolled == Some(false) || mf.recovery_pending) else {
        return Ok(Resume {
            phase: "none",
            phone_hint: String::new(),
            revocation_code: None,
        });
    };
    crate::workspace::validate_auth_identity(ws, login, &mf)?;
    let session = mf
        .session
        .as_ref()
        .ok_or_else(|| AppError::NotReady("ADD_NO_SESSION".into()))?;
    let finalized = mf.fully_enrolled == Some(true);
    let add = AddAuthResult {
        status: mf.status.unwrap_or(1),
        shared_secret: mf.shared_secret.clone(),
        identity_secret: mf.identity_secret.clone(),
        revocation_code: mf.revocation_code.clone(),
        uri: mf.uri.clone(),
        serial_number: mf.serial_number.clone(),
        secret_1: mf.secret_1.clone(),
        server_time: mf.server_time.unwrap_or(0).to_string(),
        account_name: mf.account_name.clone(),
        token_gid: mf.token_gid.clone(),
        phone_number_hint: mf.phone_number_hint.clone(),
    };
    registry().lock().unwrap().insert(
        login.to_string(),
        AddSession {
            access_token: session.access_token.clone(),
            refresh_token: session.refresh_token.clone(),
            steam_id: session.steam_id,
            session_id: session.session_id.clone(),
            add: Some(add),
            device_id: mf.device_id.clone(),
            finalized,
            login_confirmations: Vec::new(),
        },
    );
    // Rehydrate the ordinary vault if the original write failed after the
    // durable emergency copy (e.g. the user locked the vault during the API call).
    super::vault::save_plain(ws, login, &mf)?;
    if finalized {
        crate::workspace::set_authenticator_meta(ws, login, true, Some(mf.account_name.clone()))?;
    }
    Ok(Resume {
        phase: if finalized { "revocation" } else { "finalize" },
        phone_hint: mf.phone_number_hint.clone(),
        revocation_code: finalized.then(|| mf.revocation_code.clone()),
    })
}
pub fn acknowledge(ws: &std::path::Path, login: &str) -> AppResult<()> {
    let mut mf = super::vault::load_plain(ws, login)?
        .ok_or_else(|| AppError::NotReady("ADD_NOT_CREATED".into()))?;
    if mf.fully_enrolled != Some(true) {
        return Err(AppError::NotReady("CONF_NOT_ENROLLED".into()));
    }
    mf.recovery_pending = false;
    super::vault::save_plain(ws, login, &mf)?;
    let path = recovery_path(ws, login)?;
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    drop_session(login);
    Ok(())
}
pub fn cancel(ws: &std::path::Path, login: &str) -> AppResult<()> {
    let issued = registry()
        .lock()
        .unwrap()
        .get(login)
        .is_some_and(|s| s.add.is_some());
    if issued && !recovery_path(ws, login)?.exists() {
        return Err(AppError::NotReady("ADD_UNSAVED_SECRETS".into()));
    }
    drop_session(login);
    Ok(())
}

#[cfg(test)]
mod recovery_tests {
    use super::*;
    #[test]
    fn issued_secrets_survive_restart_and_activation_is_saved_before_acknowledgement() {
        let root =
            std::env::temp_dir().join(format!("shadow-enrollment-{}", rand::random::<u64>()));
        let login = format!("enroll_{}", rand::random::<u32>());
        crate::workspace::ensure_account_dirs(&root, &login).unwrap();
        let mut mf = MaFile::default();
        mf.account_name = login.clone();
        mf.shared_secret = "MTIzNDU2Nzg5MDEyMzQ1Njc4OTA=".into();
        mf.identity_secret = mf.shared_secret.clone();
        mf.revocation_code = "R-TEST-RECOVERY".into();
        mf.fully_enrolled = Some(false);
        mf.recovery_pending = true;
        mf.session = Some(SessionData {
            steam_id: 123,
            access_token: "access".into(),
            refresh_token: "refresh".into(),
            session_id: "cookie".into(),
        });
        save_recovery(&root, &login, &mf).unwrap();
        let protected = std::fs::read(recovery_path(&root, &login).unwrap()).unwrap();
        assert!(!protected
            .windows(mf.shared_secret.len())
            .any(|w| w == mf.shared_secret.as_bytes()));
        assert_eq!(resume(&root, &login).unwrap().phase, "finalize");
        assert!(acknowledge(&root, &login).is_err());
        assert!(add_finalize(&login, "", 6, true).is_err());
        with_session(&login, |s| {
            s.finalized = true;
            Ok(())
        })
        .unwrap();
        add_persist(&login, &root).unwrap();
        drop_session(&login); // Simulate closing/restarting the app before UI acknowledgement.
        let resumed = resume(&root, &login).unwrap();
        assert_eq!(resumed.phase, "revocation");
        assert_eq!(resumed.revocation_code.as_deref(), Some("R-TEST-RECOVERY"));
        assert!(
            crate::workspace::read_meta(&crate::workspace::account_dir(&root, &login))
                .has_authenticator
        );
        acknowledge(&root, &login).unwrap();
        assert_eq!(resume(&root, &login).unwrap().phase, "none");
        assert!(!recovery_path(&root, &login).unwrap().exists());
        assert!(std::fs::canonicalize(&root)
            .unwrap()
            .starts_with(std::fs::canonicalize(std::env::temp_dir()).unwrap()));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn malformed_response_does_not_echo_secrets() {
        let error = parse_response::<serde_json::Value>("SECRET_TOKEN_MUST_NOT_APPEAR", "AddAuth")
            .unwrap_err()
            .to_string();
        assert!(!error.contains("SECRET_TOKEN_MUST_NOT_APPEAR"));
    }
}
