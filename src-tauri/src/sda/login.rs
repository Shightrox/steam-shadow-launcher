//! Steam mobile authentication flow.
//!
//! Four steps (all via `https://api.steampowered.com`):
//!
//! 1. `IAuthenticationService/GetPasswordRSAPublicKey/v1`  — fetch public key.
//! 2. `IAuthenticationService/BeginAuthSessionViaCredentials/v1` — submit
//!    RSA-encrypted password, receive `client_id`, `request_id`, `steam_id`
//!    and a list of `allowed_confirmations` (device code / email / none).
//! 3. `IAuthenticationService/UpdateAuthSessionWithSteamGuardCode/v1` — if a
//!    guard code is needed, submit it.
//! 4. `IAuthenticationService/PollAuthSessionStatus/v1` — poll every `interval`
//!    seconds. Eventually returns `access_token`, `refresh_token`, plus a
//!    new `account_name` confirmation.
//!
//! Refresh: `IAuthenticationService/GenerateAccessTokenForApp/v1` takes the
//! `refresh_token` and yields a fresh `access_token`. Refresh-token TTL is
//! ~200 days; access-token TTL is ~24h.

use crate::error::{AppError, AppResult};
use crate::http;
use crate::sda::crypto;
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use serde::{Deserialize, Serialize};

const API: &str = "https://api.steampowered.com";

// ── Helpers ───────────────────────────────────────────────────────────────

/// `GetPasswordRSAPublicKey` wire response.
#[derive(Debug, Deserialize)]
struct RsaKeyResp {
    #[serde(default)]
    response: RsaKeyInner,
}
#[derive(Debug, Default, Deserialize)]
struct RsaKeyInner {
    #[serde(default)]
    publickey_mod: String,
    #[serde(default)]
    publickey_exp: String,
    #[serde(default)]
    timestamp: String,
}

/// One allowed confirmation method, as returned by BeginAuthSession.
///
/// Steam enum `k_EAuthSessionGuardType_*`:
///   1 = Unknown, 2 = None, 3 = EmailCode, 4 = DeviceCode,
///   5 = DeviceConfirmation, 6 = EmailConfirmation, 7 = MachineToken, 9 = LegacyMachineAuth.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllowedConfirmation {
    #[serde(default, rename = "confirmation_type")]
    pub kind: i64,
    #[serde(default)]
    pub associated_message: String,
}

/// Output of [`begin`] — what the UI needs to continue.
#[derive(Debug, Clone, Serialize)]
pub struct BeginOutcome {
    #[serde(rename = "clientId")]
    pub client_id: String,
    #[serde(rename = "requestId")]
    pub request_id: String,
    #[serde(rename = "steamId")]
    pub steam_id: String,
    #[serde(rename = "weakToken")]
    pub weak_token: String,
    /// Whichever guard-confirmation types Steam will accept for this session.
    #[serde(rename = "allowedConfirmations")]
    pub allowed_confirmations: Vec<AllowedConfirmation>,
    /// Seconds between recommended poll attempts.
    pub interval: f64,
    /// Extended domain (if email 2FA is in play).
    #[serde(rename = "extendedDomain", skip_serializing_if = "Option::is_none")]
    pub extended_domain: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BeginResp {
    #[serde(default)]
    response: BeginInner,
}
#[derive(Debug, Default, Deserialize)]
struct BeginInner {
    #[serde(default)]
    client_id: String,
    #[serde(default)]
    request_id: String, // base64-encoded
    #[serde(default)]
    steamid: String,
    #[serde(default)]
    weak_token: String,
    #[serde(default)]
    allowed_confirmations: Vec<AllowedConfirmation>,
    #[serde(default)]
    interval: f64,
    #[serde(default)]
    extended_error_message: String,
}

/// Output of [`poll`].
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state")]
pub enum PollState {
    /// Still waiting for the code / app confirmation.
    Pending,
    /// The user submitted a wrong / expired guard code.
    NeedsCode,
    /// Auth session expired or was rejected.
    Failed { reason: String },
    /// Tokens issued.
    Done {
        #[serde(rename = "accessToken")]
        access_token: String,
        #[serde(rename = "refreshToken")]
        refresh_token: String,
        #[serde(rename = "accountName")]
        account_name: String,
        #[serde(rename = "steamId")]
        steam_id: String,
        #[serde(rename = "newGuardData", skip_serializing_if = "Option::is_none")]
        new_guard_data: Option<String>,
    },
}

#[derive(Debug, Deserialize)]
struct PollResp {
    #[serde(default)]
    response: PollInner,
}
#[derive(Debug, Default, Deserialize)]
struct PollInner {
    #[allow(dead_code)] // wire field; we don't currently rotate client ids
    #[serde(default)]
    new_client_id: String,
    #[serde(default)]
    new_challenge_url: String,
    #[serde(default)]
    refresh_token: String,
    #[serde(default)]
    access_token: String,
    #[allow(dead_code)] // wire field; tells us "user clicked something"
    #[serde(default)]
    had_remote_interaction: bool,
    #[serde(default)]
    account_name: String,
    #[serde(default)]
    new_guard_data: String,
    #[allow(dead_code)] // wire field; agreement URL on first-login terms accept
    #[serde(default)]
    agreement_session_url: String,
}

// ── Step 1: RSA key ───────────────────────────────────────────────────────

fn fetch_rsa_key(account_name: &str) -> AppResult<RsaKeyInner> {
    let url = format!("{API}/IAuthenticationService/GetPasswordRSAPublicKey/v1/");
    let resp = http::shared()
        .get(&url)
        .query(&[("account_name", account_name)])
        .send()
        .map_err(|e| AppError::Other(format!("RSAKey: {e}")))?;
    if !resp.status().is_success() {
        return Err(AppError::Other(format!(
            "RSAKey HTTP {}",
            resp.status()
        )));
    }
    let body: RsaKeyResp = resp
        .json()
        .map_err(|e| AppError::Other(format!("RSAKey JSON: {e}")))?;
    if body.response.publickey_mod.is_empty() {
        return Err(AppError::NotFound("RSA_KEY_UNKNOWN_USER".into()));
    }
    Ok(body.response)
}

// ── Step 2: Begin ─────────────────────────────────────────────────────────

/// Begin a mobile-app auth session. Blocks on network.
pub fn begin(account_name: &str, password: &str) -> AppResult<BeginOutcome> {
    let key = fetch_rsa_key(account_name)?;
    let encrypted = crypto::encrypt_password(password, &key.publickey_mod, &key.publickey_exp)?;
    let device_friendly_name = format!(
        "{} (Shadow)",
        std::env::var("COMPUTERNAME").unwrap_or_else(|_| "PC".into())
    );

    // Send request as `input_json` — Steam's IService gateway supports that
    // for every method and always returns JSON in response. This avoids the
    // protobuf-mirror problem where sending protobuf bytes made Valve reply
    // in protobuf as well (we'd have to parse it by hand).
    let ts: u64 = key.timestamp.parse().unwrap_or(0);
    let input = serde_json::json!({
        "device_friendly_name": device_friendly_name,
        "account_name": account_name,
        "encrypted_password": encrypted,
        "encryption_timestamp": ts,
        "remember_login": true,
        "platform_type": 3,     // k_EAuthTokenPlatformType_MobileApp
        "persistence": 1,       // k_ESessionPersistence_Persistent
        "website_id": "Mobile",
        "device_details": {
            "device_friendly_name": device_friendly_name,
            "platform_type": 3,
        },
        "language": 0,
    });
    let form = [("input_json", input.to_string())];

    let url = format!("{API}/IAuthenticationService/BeginAuthSessionViaCredentials/v1/");
    let resp = http::shared()
        .post(&url)
        .header("Accept", "application/json")
        .form(&form)
        .send()
        .map_err(|e| AppError::Other(format!("BeginAuth: {e}")))?;
    let status = resp.status();
    let body = resp
        .text()
        .map_err(|e| AppError::Other(format!("BeginAuth read: {e}")))?;
    if !status.is_success() {
        return Err(AppError::Other(format!(
            "BEGIN_AUTH_FAIL: HTTP {status}: {body}"
        )));
    }
    let parsed: BeginResp = serde_json::from_str(&body)
        .map_err(|e| AppError::Other(format!("BeginAuth JSON: {e}: {body}")))?;
    let inner = parsed.response;
    tracing::info!(
        "auth_login_begin: client_id={} steam_id={} interval={} confirms={:?}",
        inner.client_id, inner.steamid, inner.interval,
        inner.allowed_confirmations.iter().map(|c| c.kind).collect::<Vec<_>>()
    );
    if inner.client_id.is_empty() {
        return Err(AppError::Other(format!(
            "BEGIN_AUTH_NO_CLIENT_ID: {}",
            inner.extended_error_message
        )));
    }
    Ok(BeginOutcome {
        client_id: inner.client_id,
        request_id: inner.request_id,
        steam_id: inner.steamid,
        weak_token: inner.weak_token,
        allowed_confirmations: inner.allowed_confirmations,
        interval: if inner.interval > 0.0 { inner.interval } else { 5.0 },
        extended_domain: None,
    })
}

// (protobuf helpers removed — we now use input_json)

// ── Step 3: Submit guard code ─────────────────────────────────────────────

/// `code_type` values (EAuthSessionGuardType): 3 = EmailCode, 4 = DeviceCode,
/// 6 = EmailConfirmation, etc.
pub fn submit_code(client_id: &str, steam_id: &str, code: &str, code_type: i64) -> AppResult<()> {
    let url = format!("{API}/IAuthenticationService/UpdateAuthSessionWithSteamGuardCode/v1/");
    let form = [
        ("client_id", client_id.to_string()),
        ("steamid", steam_id.to_string()),
        ("code", code.to_string()),
        ("code_type", code_type.to_string()),
    ];
    let resp = http::shared()
        .post(&url)
        .form(&form)
        .send()
        .map_err(|e| AppError::Other(format!("SubmitCode: {e}")))?;
    let status = resp.status();
    // Steam stuffs the real result code into `x-eresult` header. HTTP can be
    // 200 even when the guard code was rejected; we have to consult the
    // header to detect that.
    let eresult = resp
        .headers()
        .get("x-eresult")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let emsg = resp
        .headers()
        .get("x-error_message")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let body = resp
        .text()
        .map_err(|e| AppError::Other(format!("SubmitCode read: {e}")))?;
    tracing::info!(
        "submit_code: HTTP {status} x-eresult='{eresult}' x-err='{emsg}' body_len={} code_type={code_type}",
        body.len()
    );
    if !status.is_success() {
        return Err(AppError::Other(format!(
            "SUBMIT_CODE_FAIL: HTTP {status} x-eresult={eresult} x-err={emsg}: {body}"
        )));
    }
    // eresult: 1 = OK, 29 = TwoFactorCodeMismatch, 88 = RateLimit
    if !eresult.is_empty() && eresult != "1" {
        return Err(AppError::Other(format!(
            "SUBMIT_CODE_FAIL: x-eresult={eresult} x-err={emsg}"
        )));
    }
    Ok(())
}

// ── Step 4: Poll ──────────────────────────────────────────────────────────

pub fn poll(client_id: &str, request_id: &str) -> AppResult<PollState> {
    let url = format!("{API}/IAuthenticationService/PollAuthSessionStatus/v1/");
    let form = [
        ("client_id", client_id.to_string()),
        ("request_id", request_id.to_string()),
    ];
    let resp = http::shared()
        .post(&url)
        .form(&form)
        .send()
        .map_err(|e| AppError::Other(format!("Poll: {e}")))?;
    let status = resp.status();
    let eresult = resp
        .headers()
        .get("x-eresult")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let body = resp
        .text()
        .map_err(|e| AppError::Other(format!("Poll read: {e}")))?;
    tracing::info!(
        "poll: HTTP {status} x-eresult='{eresult}' body_len={}",
        body.len()
    );
    if !status.is_success() {
        // eresult 3 = Expired/Denied (session dead).
        if body.contains("InvalidGrant") || body.contains("AuthSessionExpired") {
            return Ok(PollState::Failed {
                reason: "SESSION_EXPIRED".into(),
            });
        }
        return Err(AppError::Other(format!("POLL_FAIL: HTTP {status} x-eresult={eresult}: {body}")));
    }
    let parsed: PollResp = serde_json::from_str(&body)
        .map_err(|e| AppError::Other(format!("Poll JSON: {e}: {body}")))?;
    let r = parsed.response;
    if !r.access_token.is_empty() && !r.refresh_token.is_empty() {
        // Derive a simple steam_id out of the access-token JWT sub claim.
        let steam_id = steam_id_from_jwt(&r.access_token).unwrap_or_default();
        return Ok(PollState::Done {
            access_token: r.access_token,
            refresh_token: r.refresh_token,
            account_name: r.account_name,
            steam_id,
            new_guard_data: if r.new_guard_data.is_empty() {
                None
            } else {
                Some(r.new_guard_data)
            },
        });
    }
    if !r.new_challenge_url.is_empty() {
        // A new challenge means the user must submit a different guard method.
        return Ok(PollState::NeedsCode);
    }
    Ok(PollState::Pending)
}

/// Minimal JWT `sub` extraction (no signature verification — we only trust
/// this token for display purposes, real verification is done by Steam
/// servers on every API call).
fn steam_id_from_jwt(tok: &str) -> Option<String> {
    let mid = tok.split('.').nth(1)?;
    // JWT payload is base64url without padding.
    let pad = (4 - mid.len() % 4) % 4;
    let mut padded = mid.replace('-', "+").replace('_', "/");
    padded.push_str(&"=".repeat(pad));
    let decoded = B64.decode(padded.as_bytes()).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&decoded).ok()?;
    v.get("sub").and_then(|x| x.as_str()).map(|s| s.to_string())
}

// ── Refresh ───────────────────────────────────────────────────────────────

pub fn refresh_access_token(
    refresh_token: &str,
    steam_id: &str,
) -> AppResult<String> {
    let url = format!("{API}/IAuthenticationService/GenerateAccessTokenForApp/v1/");
    let form = [
        ("refresh_token", refresh_token.to_string()),
        ("steamid", steam_id.to_string()),
    ];
    let resp = http::shared()
        .post(&url)
        .form(&form)
        .send()
        .map_err(|e| AppError::Other(format!("Refresh: {e}")))?;
    if !resp.status().is_success() {
        return Err(AppError::Other(format!(
            "REFRESH_FAIL: HTTP {}",
            resp.status()
        )));
    }
    let v: serde_json::Value = resp
        .json()
        .map_err(|e| AppError::Other(format!("Refresh JSON: {e}")))?;
    let tok = v
        .get("response")
        .and_then(|x| x.get("access_token"))
        .and_then(|x| x.as_str())
        .ok_or_else(|| AppError::Other("REFRESH_NO_TOKEN".into()))?;
    Ok(tok.to_string())
}
