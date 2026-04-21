//! Steam Desktop Authenticator `.maFile` model + parser.
//!
//! Clean-room JSON shape based on the public spec (see SDA's `SteamGuardAccount.cs`
//! and `SessionData.cs`). For M1 we support **plain** maFiles only; SDA-encrypted
//! and steamguard-cli manifests come in M4.
//!
//! ## Fields
//!
//! Required to function:
//! - `shared_secret` (base64) — TOTP key.
//! - `identity_secret` (base64) — confirmation HMAC key.
//! - `account_name`, `device_id`.
//!
//! Required to revoke:
//! - `revocation_code` (`R12345`).
//!
//! Optional (we keep them around for round-trip export):
//! - `serial_number`, `uri`, `server_time`, `token_gid`, `secret_1`, `status`,
//!   `phone_number_hint`, `confirm_type`, `fully_enrolled`, `Session`.

use crate::error::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use std::path::Path;
use zeroize::ZeroizeOnDrop;

#[derive(Debug, Clone, Serialize, Deserialize, ZeroizeOnDrop)]
pub struct SessionData {
    #[serde(rename = "SteamID", default)]
    pub steam_id: u64,
    #[serde(rename = "AccessToken", default)]
    pub access_token: String,
    #[serde(rename = "RefreshToken", default)]
    pub refresh_token: String,
    #[serde(rename = "SessionID", default)]
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, ZeroizeOnDrop)]
pub struct MaFile {
    /// TOTP key (base64). REQUIRED.
    pub shared_secret: String,
    /// Confirmation HMAC key (base64). REQUIRED.
    pub identity_secret: String,
    /// Steam login. REQUIRED.
    pub account_name: String,
    /// `android:<guid>` style id used in confirmation `p=` query.
    #[serde(default)]
    pub device_id: String,
    /// `R12345` revocation. Required to revoke from Steam.
    #[serde(default)]
    pub revocation_code: String,

    // ── Optional, kept for round-trip ────────────────────────────────────
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub serial_number: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_time: Option<u64>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub token_gid: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub secret_1: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<i64>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub phone_number_hint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm_type: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fully_enrolled: Option<bool>,

    /// Cookies and access/refresh tokens. May be missing if user dragged in
    /// a maFile that was generated without ever logging in (rare).
    #[serde(rename = "Session", default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionData>,
}

impl MaFile {
    /// Validate that the bare-minimum fields needed for code generation are
    /// present. Returns a human-readable error code suitable for i18n lookup.
    pub fn validate_for_codes(&self) -> AppResult<()> {
        if self.shared_secret.trim().is_empty() {
            return Err(AppError::Other("MAFILE_NO_SHARED_SECRET".into()));
        }
        if self.account_name.trim().is_empty() {
            return Err(AppError::Other("MAFILE_NO_ACCOUNT_NAME".into()));
        }
        // Sanity-check base64 right now so we fail at import, not at first use.
        use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
        B64.decode(self.shared_secret.trim())
            .map_err(|_| AppError::Other("MAFILE_BAD_SHARED_SECRET".into()))?;
        if !self.identity_secret.trim().is_empty() {
            B64.decode(self.identity_secret.trim())
                .map_err(|_| AppError::Other("MAFILE_BAD_IDENTITY_SECRET".into()))?;
        }
        Ok(())
    }

    /// Parse from raw JSON bytes.
    pub fn from_json_bytes(bytes: &[u8]) -> AppResult<Self> {
        let s = std::str::from_utf8(bytes)
            .map_err(|_| AppError::Other("MAFILE_NOT_UTF8".into()))?;
        Self::from_json_str(s)
    }

    /// Parse from a JSON string. Tolerates leading BOM and stray whitespace.
    pub fn from_json_str(s: &str) -> AppResult<Self> {
        let trimmed = s.trim_start_matches('\u{feff}').trim();
        let mut mf: MaFile = serde_json::from_str(trimmed)
            .map_err(|e| AppError::Other(format!("MAFILE_PARSE: {e}")))?;
        // SDA writes account_name lowercased on import but UI may have uppercase.
        // We don't normalize destructively — but we DO trim whitespace to avoid
        // surprises in the UI ("user " != "user").
        mf.account_name = mf.account_name.trim().to_string();
        mf.shared_secret = mf.shared_secret.trim().to_string();
        mf.identity_secret = mf.identity_secret.trim().to_string();
        mf.validate_for_codes()?;
        Ok(mf)
    }

    /// Read a `.maFile` from disk.
    pub fn read_file(path: &Path) -> AppResult<Self> {
        let bytes = std::fs::read(path)?;
        Self::from_json_bytes(&bytes)
    }

    /// Serialize back to SDA-compatible pretty JSON.
    pub fn to_json_pretty(&self) -> AppResult<String> {
        serde_json::to_string_pretty(self)
            .map_err(|e| AppError::Other(format!("MAFILE_SERIALIZE: {e}")))
    }
}
