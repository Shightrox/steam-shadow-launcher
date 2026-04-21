//! Shared HTTP client for both `download.rs` and SDA / Steam mobile auth.
//!
//! `reqwest::blocking::Client` is heavy (TLS + cookie jar) — building one per
//! request would burn allocations. Instead we keep a process-wide singleton
//! for "no cookies needed" calls (downloads, public APIs) and let SDA build
//! its own per-account cookie-bearing clients via [`new_session_client`].

use std::sync::OnceLock;
use std::time::Duration;

const USER_AGENT: &str =
    "Valve/Steam HTTP Client 1.0 (Steam Shadow Launcher; +https://github.com/Shightrox/steam-shadow-launcher)";

const MOBILE_UA: &str = "Mozilla/5.0 (Linux; U; Android 9; en-us; Valve Steam App Version/3) \
                         AppleWebKit/537.36 (KHTML, like Gecko) Mobile Safari/537.36";

static SHARED: OnceLock<reqwest::blocking::Client> = OnceLock::new();

/// Cookie-less, gzip-on, mid-timeout client for plain GET/POST.
/// Cheap to clone (it's already an `Arc` inside).
pub fn shared() -> reqwest::blocking::Client {
    SHARED
        .get_or_init(|| {
            reqwest::blocking::Client::builder()
                .user_agent(USER_AGENT)
                .gzip(true)
                .connect_timeout(Duration::from_secs(20))
                .timeout(Duration::from_secs(120))
                .build()
                .expect("build shared http client")
        })
        .clone()
}

/// Per-session client used by SDA. Each one holds its OWN cookie jar
/// (steamLoginSecure + sessionid + mobileClient cookies). Build once per
/// account, reuse for the lifetime of the session.
pub fn new_session_client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .user_agent(MOBILE_UA)
        .cookie_store(true)
        .gzip(true)
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(60))
        .build()
        .expect("build session http client")
}
