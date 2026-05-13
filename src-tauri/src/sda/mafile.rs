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

mod de_helpers {
    use serde::{Deserialize, Deserializer};

    /// Deserialize `null | missing | string` into `String`.
    pub fn string_or_null<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
        Ok(Option::<String>::deserialize(d)?.unwrap_or_default())
    }

    /// Deserialize an `Option<u64>` that may arrive as a number, string, or null.
    pub fn option_u64_or_string<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Val {
            Num(u64),
            Str(String),
            Null,
        }
        match Option::<Val>::deserialize(d)? {
            Some(Val::Num(n)) => Ok(Some(n)),
            Some(Val::Str(s)) if s.is_empty() => Ok(None),
            Some(Val::Str(s)) => s.parse::<u64>().map(Some).map_err(serde::de::Error::custom),
            Some(Val::Null) | None => Ok(None),
        }
    }

    /// Deserialize a `u64` that may arrive as either a JSON number or a string.
    pub fn u64_or_string<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum NumOrStr {
            Num(u64),
            Str(String),
        }
        match NumOrStr::deserialize(d)? {
            NumOrStr::Num(n) => Ok(n),
            NumOrStr::Str(s) => s.parse::<u64>().map_err(serde::de::Error::custom),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ZeroizeOnDrop)]
pub struct SessionData {
    #[serde(
        rename = "SteamID",
        default,
        deserialize_with = "de_helpers::u64_or_string"
    )]
    pub steam_id: u64,
    #[serde(
        rename = "AccessToken",
        default,
        deserialize_with = "de_helpers::string_or_null"
    )]
    pub access_token: String,
    #[serde(
        rename = "RefreshToken",
        default,
        deserialize_with = "de_helpers::string_or_null"
    )]
    pub refresh_token: String,
    #[serde(
        rename = "SessionID",
        default,
        deserialize_with = "de_helpers::string_or_null"
    )]
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
    #[serde(default, deserialize_with = "de_helpers::string_or_null")]
    pub device_id: String,
    /// `R12345` revocation. Required to revoke from Steam.
    #[serde(default, deserialize_with = "de_helpers::string_or_null")]
    pub revocation_code: String,

    // ── Optional, kept for round-trip ────────────────────────────────────
    #[serde(
        default,
        skip_serializing_if = "String::is_empty",
        deserialize_with = "de_helpers::string_or_null"
    )]
    pub serial_number: String,
    #[serde(
        default,
        skip_serializing_if = "String::is_empty",
        deserialize_with = "de_helpers::string_or_null"
    )]
    pub uri: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_helpers::option_u64_or_string"
    )]
    pub server_time: Option<u64>,
    #[serde(
        default,
        skip_serializing_if = "String::is_empty",
        deserialize_with = "de_helpers::string_or_null"
    )]
    pub token_gid: String,
    #[serde(
        default,
        skip_serializing_if = "String::is_empty",
        deserialize_with = "de_helpers::string_or_null"
    )]
    pub secret_1: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<i64>,
    #[serde(
        default,
        skip_serializing_if = "String::is_empty",
        deserialize_with = "de_helpers::string_or_null"
    )]
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

#[cfg(test)]
mod tests {
    use super::*;

    // Minimum-viable shared/identity secrets (valid base64).
    const SHARED: &str = "L/sdTRq9hLcjYnFqccZfU5jSya8=";
    const IDENT: &str = "IlXB0Wl4e75NPMGqiZ1n64SMe1E=";

    #[test]
    fn parses_session_id_null() {
        let json = format!(
            r#"{{"shared_secret":"{SHARED}","identity_secret":"{IDENT}","account_name":"foo","Session":{{"SteamID":1,"AccessToken":"a","RefreshToken":"r","SessionID":null}}}}"#
        );
        let mf = MaFile::from_json_str(&json).expect("must parse SessionID:null");
        let s = mf.session.clone().expect("session present");
        assert_eq!(s.session_id, "");
        assert_eq!(s.access_token, "a");
        assert_eq!(s.refresh_token, "r");
    }

    #[test]
    fn parses_access_token_null() {
        let json = format!(
            r#"{{"shared_secret":"{SHARED}","identity_secret":"{IDENT}","account_name":"foo","Session":{{"SteamID":1,"AccessToken":null,"RefreshToken":null,"SessionID":null}}}}"#
        );
        let mf = MaFile::from_json_str(&json).expect("must parse all-null Session strings");
        let s = mf.session.clone().expect("session present");
        assert_eq!(s.access_token, "");
        assert_eq!(s.refresh_token, "");
        assert_eq!(s.session_id, "");
    }

    #[test]
    fn parses_without_session_block() {
        let json = format!(
            r#"{{"shared_secret":"{SHARED}","identity_secret":"{IDENT}","account_name":"foo"}}"#
        );
        let mf = MaFile::from_json_str(&json).expect("must parse without Session");
        assert!(mf.session.is_none());
    }

    #[test]
    fn rejects_missing_shared_secret() {
        let json = r#"{"shared_secret":"","identity_secret":"","account_name":"foo"}"#;
        let err = MaFile::from_json_str(json).expect_err("must fail");
        let msg = format!("{err:?}");
        assert!(msg.contains("MAFILE_NO_SHARED_SECRET"), "got: {msg}");
    }

    #[test]
    fn parses_real_user_mafile_shape() {
        // Mirrors the user's _c1a maFile: SessionID is null, optional strings present.
        let json = format!(
            r#"{{"shared_secret":"{SHARED}","serial_number":"17558289479296558557","revocation_code":"R92495","uri":"otpauth://totp/Steam:c1a","server_time":1714526189,"account_name":"_c1a","token_gid":"377e45b120531a76","identity_secret":"{IDENT}","secret_1":"plshn1cLIKVGKC+ol7kQYym8Vew=","status":1,"device_id":"android:35dd53d8-c7e0-42fa-aa5e-fe131e52bff3","fully_enrolled":true,"Session":{{"SteamID":76561199681173042,"AccessToken":"a","RefreshToken":"r","SessionID":null}}}}"#
        );
        let mf = MaFile::from_json_str(&json).expect("user-shaped maFile must parse");
        assert_eq!(mf.account_name, "_c1a");
        assert_eq!(mf.session.as_ref().unwrap().session_id, "");
    }
}
