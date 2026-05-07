//! In-memory "this account needs full re-login" flag.
//!
//! Set when Steam returns `needauth` on a confirmation list (the saved
//! refresh_token has been invalidated by Steam — password change, sign-out
//! everywhere, etc.) or when refreshing the access_token fails. Cleared on
//! a successful full login or successful refresh.
//!
//! Stored only in process memory: after a restart the very next call to
//! Steam will re-derive the state, so we don't need workspace persistence.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

fn set() -> &'static Mutex<HashSet<String>> {
    static C: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashSet::new()))
}

pub fn mark(login: &str) {
    set().lock().unwrap().insert(login.to_string());
}

pub fn clear(login: &str) {
    set().lock().unwrap().remove(login);
}

pub fn is_marked(login: &str) -> bool {
    set().lock().unwrap().contains(login)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mark_clear_roundtrip() {
        let login = "relogin_flag_test_user";
        clear(login);
        assert!(!is_marked(login));
        mark(login);
        assert!(is_marked(login));
        clear(login);
        assert!(!is_marked(login));
    }
}
