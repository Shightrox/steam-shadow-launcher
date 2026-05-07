//! Per-account Steam-session classification.
//!
//! Used by the UI to render a "session stale / re-login required" badge
//! near the SDA code, and by the poller to decide whether it can silently
//! refresh the access_token or must give up and wait for the user to
//! re-enter the password.

use crate::sda::{mafile::MaFile, relogin_flag};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64URL, Engine as _};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    /// Tokens are present and not about to expire — confirmations should work.
    Ok,
    /// access_token is dead/dying but refresh_token is still valid:
    /// one click (no password) refreshes it.
    Refreshable,
    /// refresh_token is dead too, OR Steam has invalidated the session
    /// (logout-everywhere / password change). Needs full re-login with password.
    NeedsRelogin,
    /// maFile has no Session block at all (e.g. dragged in pre-login).
    /// UI treats this like NeedsRelogin but distinguishes it for diagnostics.
    NoSession,
}

/// Decode a JWT payload's `exp` claim without verifying the signature
/// (Steam re-checks signatures on every call; we only need the expiry).
pub fn jwt_exp(token: &str) -> Option<i64> {
    let mid = token.split('.').nth(1)?;
    // JWT uses base64url without padding.
    let mut s = mid.replace('-', "+").replace('_', "/");
    while s.len() % 4 != 0 {
        s.push('=');
    }
    // We re-encode for URL_SAFE_NO_PAD to keep base64 happy with both shapes.
    let bytes = if mid.contains('+') || mid.contains('/') {
        base64::engine::general_purpose::STANDARD.decode(s.as_bytes()).ok()?
    } else {
        // Strip the padding we just added back off for the URL-safe decoder.
        B64URL.decode(mid.as_bytes()).ok()?
    };
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    v.get("exp").and_then(|x| x.as_i64())
}

/// Classify the current session state of a maFile.
pub fn classify(mf: &MaFile, login: &str, now: i64) -> SessionState {
    let Some(sess) = mf.session.as_ref() else {
        return SessionState::NoSession;
    };
    if sess.access_token.trim().is_empty() && sess.refresh_token.trim().is_empty() {
        return SessionState::NoSession;
    }
    if relogin_flag::is_marked(login) {
        return SessionState::NeedsRelogin;
    }
    // refresh dead/missing → only full login can save us.
    let refresh_exp = jwt_exp(&sess.refresh_token);
    match refresh_exp {
        Some(exp) if exp > now + 60 => {}
        _ => return SessionState::NeedsRelogin,
    }
    // access dead/dying → silent refresh will save us.
    let access_exp = jwt_exp(&sess.access_token);
    match access_exp {
        Some(exp) if exp > now + 30 => SessionState::Ok,
        _ => SessionState::Refreshable,
    }
}

/// `now()` helper isolated for testability.
pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sda::mafile::SessionData;

    fn jwt_with_exp(exp: i64) -> String {
        // Header is irrelevant — classify only inspects the payload.
        let header = B64URL.encode(br#"{"alg":"none","typ":"JWT"}"#);
        let payload = B64URL.encode(format!(r#"{{"exp":{exp}}}"#).as_bytes());
        format!("{header}.{payload}.sig")
    }

    fn mf_with_tokens(access: &str, refresh: &str) -> MaFile {
        let mut mf = MaFile::default();
        mf.shared_secret = "L/sdTRq9hLcjYnFqccZfU5jSya8=".into();
        mf.identity_secret = "IlXB0Wl4e75NPMGqiZ1n64SMe1E=".into();
        mf.account_name = "tester".into();
        mf.session = Some(SessionData {
            steam_id: 1,
            access_token: access.to_string(),
            refresh_token: refresh.to_string(),
            session_id: "sid".into(),
        });
        mf
    }

    #[test]
    fn ok_when_both_tokens_fresh() {
        let now = 1_700_000_000;
        let mf = mf_with_tokens(&jwt_with_exp(now + 3600), &jwt_with_exp(now + 30 * 86400));
        relogin_flag::clear("u_ok");
        assert_eq!(classify(&mf, "u_ok", now), SessionState::Ok);
    }

    #[test]
    fn refreshable_when_access_expired() {
        let now = 1_700_000_000;
        let mf = mf_with_tokens(&jwt_with_exp(now - 60), &jwt_with_exp(now + 30 * 86400));
        relogin_flag::clear("u_refresh");
        assert_eq!(classify(&mf, "u_refresh", now), SessionState::Refreshable);
    }

    #[test]
    fn needs_relogin_when_refresh_expired() {
        let now = 1_700_000_000;
        let mf = mf_with_tokens(&jwt_with_exp(now + 3600), &jwt_with_exp(now - 1));
        relogin_flag::clear("u_re");
        assert_eq!(classify(&mf, "u_re", now), SessionState::NeedsRelogin);
    }

    #[test]
    fn no_session_when_block_missing() {
        let mut mf = MaFile::default();
        mf.shared_secret = "L/sdTRq9hLcjYnFqccZfU5jSya8=".into();
        mf.identity_secret = "IlXB0Wl4e75NPMGqiZ1n64SMe1E=".into();
        mf.account_name = "x".into();
        relogin_flag::clear("u_ns");
        assert_eq!(classify(&mf, "u_ns", 1_700_000_000), SessionState::NoSession);
    }

    #[test]
    fn flag_forces_needs_relogin_even_with_valid_jwts() {
        let now = 1_700_000_000;
        let mf = mf_with_tokens(&jwt_with_exp(now + 3600), &jwt_with_exp(now + 30 * 86400));
        relogin_flag::mark("u_flag");
        assert_eq!(classify(&mf, "u_flag", now), SessionState::NeedsRelogin);
        relogin_flag::clear("u_flag");
    }

    #[test]
    fn unparseable_jwt_means_needs_relogin() {
        let mf = mf_with_tokens("not-a-jwt", "also-garbage");
        relogin_flag::clear("u_bad");
        assert_eq!(
            classify(&mf, "u_bad", 1_700_000_000),
            SessionState::NeedsRelogin
        );
    }
}
