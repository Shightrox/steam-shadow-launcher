//! Steam Desktop Authenticator integration (P11).
//!
//! Submodules:
//! - [`totp`]: 5-char Steam Guard code generation + server time alignment.
//! - [`mafile`]: parse/serialize SDA `.maFile` JSON (plain only for M1).
//! - [`vault`]: load/save authenticator data under `<account>/auth/`.
//! - [`confirmations`]: mobileconf list + allow/reject.

pub mod add;
pub mod confirmations;
pub mod crypto;
pub mod login;
pub mod mafile;
pub mod poller;
pub mod totp;
pub mod vault;
