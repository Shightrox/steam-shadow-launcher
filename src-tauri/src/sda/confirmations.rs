//! Steam mobile confirmations (trades / market / login / etc).
//!
//! Endpoints (as of 2024-2025):
//! - `GET  https://steamcommunity.com/mobileconf/getlist?<query>` → JSON list.
//! - `POST https://steamcommunity.com/mobileconf/multiajaxop` → bulk allow/reject.
//!
//! Query params required on every call:
//!   p   = device_id         (maFile.device_id, "android:<guid>")
//!   a   = steam_id (u64)    (Session.steam_id)
//!   k   = base64 HMAC-SHA1(identity_secret, be8(t) || tag)  (URL-encoded)
//!   t   = server_time       (unix seconds, server-aligned)
//!   m   = "react"           (UI flavour Steam expects)
//!   tag = "list" | "conf" | "details" | "accept" | "reject"
//!
//! Cookies required for session authentication:
//!   sessionid           = Session.session_id
//!   steamLoginSecure    = "<steam_id>||<access_token>"
//!   mobileClient        = "android"
//!   mobileClientVersion = "777777 3.6.4"
//!   Steam_Language      = "english"

use crate::error::{AppError, AppResult};
use crate::http;
use crate::sda::mafile::MaFile;
use crate::sda::totp;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};

const BASE: &str = "https://steamcommunity.com/mobileconf";

/// One entry from `/mobileconf/getlist`.
///
/// Field names match Valve's JSON shape (camelCase via serde rename where needed).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Confirmation {
    /// Confirmation ID (string of digits).
    pub id: String,
    /// Opaque nonce for this confirmation, used as `ck=` on accept/reject.
    pub nonce: String,
    /// Either the trade-offer id, market-listing id, or 0 for login etc.
    #[serde(default)]
    pub creator_id: String,
    /// Short headline (e.g. "Trade with Alice").
    #[serde(default)]
    pub headline: String,
    /// Multi-line details.
    #[serde(default)]
    pub summary: Vec<String>,
    /// `1`=trade, `2`=market, `3`=phone, `5`=account-recovery etc.
    /// We pass through the raw int so the UI can localise.
    #[serde(rename = "type")]
    pub kind: i64,
    #[serde(default)]
    pub type_name: String,
    #[serde(default)]
    pub accept: String,
    #[serde(default)]
    pub cancel: String,
    #[serde(default)]
    pub icon: String,
}

/// Wire shape of the `/mobileconf/getlist` response.
#[derive(Debug, Deserialize)]
struct ListResponse {
    success: bool,
    #[serde(default)]
    needauth: bool,
    #[serde(default)]
    message: String,
    #[serde(default)]
    conf: Vec<Confirmation>,
}

/// One row in a multi-ajax response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RespondResult {
    pub id: String,
    pub success: bool,
    #[serde(default)]
    pub message: String,
}

#[derive(Debug, Deserialize)]
struct RespondWire {
    success: bool,
    #[serde(default)]
    message: String,
}

/// `allow` accepts the confirmation, `cancel` rejects it.
#[derive(Debug, Clone, Copy)]
pub enum Op {
    Allow,
    Reject,
}

impl Op {
    fn as_str(self) -> &'static str {
        match self {
            Op::Allow => "allow",
            Op::Reject => "cancel",
        }
    }
    fn tag(self) -> &'static str {
        match self {
            Op::Allow => "accept",
            Op::Reject => "reject",
        }
    }
}

/// Build a session-cookie-bearing HTTP client.
pub fn session_client(mafile: &MaFile) -> AppResult<Client> {
    let session = mafile
        .session
        .as_ref()
        .ok_or_else(|| AppError::NotReady("CONF_NO_SESSION".into()))?;
    if session.access_token.trim().is_empty() {
        return Err(AppError::NotReady("CONF_NO_ACCESS_TOKEN".into()));
    }
    if session.session_id.trim().is_empty() {
        return Err(AppError::NotReady("CONF_NO_SESSION_ID".into()));
    }
    if session.steam_id == 0 {
        return Err(AppError::NotReady("CONF_NO_STEAM_ID".into()));
    }
    // `reqwest`'s built-in cookie jar is opaque once the client is built;
    // easier for our purposes is to pass cookies via the Cookie header on
    // each request via helper `cookie_header`, since the endpoints are all
    // same-origin (steamcommunity.com).
    Ok(http::new_session_client())
}

/// Assemble the `Cookie:` header value for mobileconf requests.
fn cookie_header(mafile: &MaFile) -> String {
    let s = mafile.session.as_ref().expect("checked by session_client");
    format!(
        "sessionid={sid}; steamLoginSecure={steamid}%7C%7C{tok}; mobileClient=android; \
         mobileClientVersion=777777%203.6.4; Steam_Language=english",
        sid = s.session_id,
        steamid = s.steam_id,
        tok = s.access_token,
    )
}

/// URL-encode `s` using a minimal subset sufficient for SDA query params
/// (HMAC base64 may contain `/`, `+`, `=`, all of which need escaping).
fn pct_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char);
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// Build the shared `?p=&a=&k=&t=&m=&tag=` query string for a given tag.
fn query_params(mafile: &MaFile, tag: &str) -> AppResult<String> {
    let session = mafile
        .session
        .as_ref()
        .ok_or_else(|| AppError::NotReady("CONF_NO_SESSION".into()))?;
    let t = totp::server_time();
    let k = totp::confirmation_key(&mafile.identity_secret, t, tag)?;
    Ok(format!(
        "p={p}&a={a}&k={k}&t={t}&m=react&tag={tag}",
        p = pct_encode(&mafile.device_id),
        a = session.steam_id,
        k = pct_encode(&k),
        t = t,
        tag = tag,
    ))
}

/// Fetch the current confirmation list.
pub fn list(mafile: &MaFile) -> AppResult<Vec<Confirmation>> {
    let client = session_client(mafile)?;
    let qs = query_params(mafile, "list")?;
    let url = format!("{BASE}/getlist?{qs}");
    let resp = client
        .get(&url)
        .header("Cookie", cookie_header(mafile))
        .header("X-Requested-With", "com.valvesoftware.android.steam.community")
        .header("Referer", "https://steamcommunity.com/mobileconf/conf")
        .send()
        .map_err(|e| AppError::Other(format!("getlist: {e}")))?;
    let status = resp.status();
    let body = resp
        .text()
        .map_err(|e| AppError::Other(format!("getlist read: {e}")))?;
    if !status.is_success() {
        return Err(AppError::Other(format!("getlist HTTP {status}: {body}")));
    }
    let parsed: ListResponse = serde_json::from_str(&body)
        .map_err(|e| AppError::Other(format!("getlist JSON: {e}: {body}")))?;
    if parsed.needauth {
        return Err(AppError::NotReady("CONF_NEEDS_RELOGIN".into()));
    }
    if !parsed.success {
        // Valve returns success=false for "no confirmations" sometimes with
        // a non-empty conf list anyway — we're lenient, but surface the msg.
        if parsed.conf.is_empty() {
            return Err(AppError::Other(format!(
                "CONF_FAIL: {}",
                if parsed.message.is_empty() { "unknown" } else { &parsed.message }
            )));
        }
    }
    Ok(parsed.conf)
}

/// Respond to multiple confirmations in a single call.
pub fn respond(
    mafile: &MaFile,
    ids: &[String],
    op: Op,
) -> AppResult<Vec<RespondResult>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    // We need one `nonce` per id — caller doesn't have it so we fetch-once.
    // This costs an extra roundtrip but keeps the command ergonomic (the UI
    // has already displayed nonces, but bundling them in the respond command
    // would leak implementation detail).
    let list_items = list(mafile)?;
    let mut pairs: Vec<(String, String)> = Vec::with_capacity(ids.len());
    for id in ids {
        let Some(row) = list_items.iter().find(|c| &c.id == id) else {
            return Err(AppError::NotFound(format!("CONF_NOT_FOUND: {id}")));
        };
        pairs.push((row.id.clone(), row.nonce.clone()));
    }

    let client = session_client(mafile)?;
    let qs = query_params(mafile, op.tag())?;
    let mut form = format!("op={}&{}", op.as_str(), qs);
    for (id, nonce) in &pairs {
        form.push_str(&format!("&cid%5B%5D={}&ck%5B%5D={}", pct_encode(id), pct_encode(nonce)));
    }

    let url = format!("{BASE}/multiajaxop");
    let resp = client
        .post(&url)
        .header("Cookie", cookie_header(mafile))
        .header("X-Requested-With", "com.valvesoftware.android.steam.community")
        .header("Referer", "https://steamcommunity.com/mobileconf/conf")
        .header("Content-Type", "application/x-www-form-urlencoded; charset=UTF-8")
        .body(form)
        .send()
        .map_err(|e| AppError::Other(format!("multiajaxop: {e}")))?;
    let status = resp.status();
    let body = resp
        .text()
        .map_err(|e| AppError::Other(format!("multiajaxop read: {e}")))?;
    if !status.is_success() {
        return Err(AppError::Other(format!(
            "multiajaxop HTTP {status}: {body}"
        )));
    }

    // The response shape is `{ success: bool, message?: string }` that applies
    // to the entire batch. Single-op `/mobileconf/ajaxop` returns per-item
    // but multiajaxop is all-or-nothing from Valve's perspective.
    let parsed: RespondWire = serde_json::from_str(&body)
        .map_err(|e| AppError::Other(format!("multiajaxop JSON: {e}: {body}")))?;
    let ok = parsed.success;
    let msg = parsed.message;
    Ok(pairs
        .into_iter()
        .map(|(id, _)| RespondResult {
            id,
            success: ok,
            message: msg.clone(),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_plus_and_slash() {
        let out = pct_encode("a+b/c=");
        assert_eq!(out, "a%2Bb%2Fc%3D");
    }

    #[test]
    fn op_tags() {
        assert_eq!(Op::Allow.as_str(), "allow");
        assert_eq!(Op::Allow.tag(), "accept");
        assert_eq!(Op::Reject.as_str(), "cancel");
        assert_eq!(Op::Reject.tag(), "reject");
    }
}
