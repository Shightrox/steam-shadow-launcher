//! Credential login, optional password persistence and automatic Steam Guard.
use super::{
    credentials::{self, Credentials},
    login::{self, BeginOutcome, PollState},
    mafile::{MaFile, SessionData},
    relogin_flag, session_state, totp, vault,
};
use crate::error::{AppError, AppResult};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

struct PendingLogin {
    workspace: PathBuf,
    login: String,
    account_name: String,
    steam_id: u64,
    request_id: String,
    sealed_password: Option<Vec<u8>>,
    created: Instant,
}

fn pending() -> &'static Mutex<HashMap<String, PendingLogin>> {
    static PENDING: OnceLock<Mutex<HashMap<String, PendingLogin>>> = OnceLock::new();
    PENDING.get_or_init(Default::default)
}

fn prune(map: &mut HashMap<String, PendingLogin>) {
    map.retain(|_, p| p.created.elapsed() < Duration::from_secs(600));
}

pub fn validate_identity(mf: &MaFile, account: &str, steam_id: u64) -> AppResult<()> {
    if steam_id == 0
        || !mf.account_name.eq_ignore_ascii_case(account)
        || mf
            .session
            .as_ref()
            .is_some_and(|s| s.steam_id != 0 && s.steam_id != steam_id)
    {
        return Err(AppError::NotReady("AUTH_ACCOUNT_MISMATCH".into()));
    }
    Ok(())
}

pub fn begin_manual(
    workspace: &Path,
    shadow_login: &str,
    account_name: &str,
    password: Zeroizing<String>,
    remember: bool,
) -> AppResult<BeginOutcome> {
    crate::workspace::validate_login(shadow_login)?;
    if !shadow_login.eq_ignore_ascii_case(account_name) {
        return Err(AppError::NotReady("AUTH_ACCOUNT_MISMATCH".into()));
    }
    if !crate::workspace::account_dir(workspace, shadow_login).is_dir() {
        return Err(AppError::NotFound("Account does not exist".into()));
    }
    let mf = vault::load_plain(workspace, shadow_login)?;
    if mf
        .as_ref()
        .is_some_and(|mf| !mf.account_name.eq_ignore_ascii_case(account_name))
    {
        return Err(AppError::NotReady("AUTH_ACCOUNT_MISMATCH".into()));
    }
    let mut begin = login::begin(account_name, &password)?;
    let steam_id = begin
        .steam_id
        .parse::<u64>()
        .ok()
        .filter(|id| *id != 0)
        .ok_or_else(|| AppError::Other("Steam returned no account ID".into()))?;
    if let Some(mf) = mf.as_ref() {
        validate_identity(mf, account_name, steam_id)?;
    }
    let mut identity = MaFile::default();
    identity.account_name = account_name.to_string();
    identity.session = Some(SessionData {
        steam_id,
        access_token: String::new(),
        refresh_token: String::new(),
        session_id: String::new(),
    });
    crate::workspace::validate_auth_identity(workspace, shadow_login, &identity)?;
    let sealed = if remember {
        Some(credentials::seal(&Credentials::new(
            account_name.into(),
            password.to_string(),
            steam_id,
        ))?)
    } else {
        None
    };
    {
        let mut map = pending().lock().unwrap();
        prune(&mut map);
        // Keep attempts independent: a slow, cancelled Begin response must not
        // evict a newer attempt for the same account. The UI cancels its own ID.
        map.insert(
            begin.client_id.clone(),
            PendingLogin {
                workspace: workspace.into(),
                login: shadow_login.into(),
                account_name: account_name.into(),
                steam_id,
                request_id: begin.request_id.clone(),
                sealed_password: sealed,
                created: Instant::now(),
            },
        );
    }
    if let Some(mf) = mf.as_ref().filter(|mf| mf.fully_enrolled != Some(false)) {
        if begin.allowed_confirmations.iter().any(|c| c.kind == 3) {
            // On failure keep the ordinary code screen available. Do not save
            // the password until polling proves the entire login succeeded.
            match totp::generate_code_now(&mf.shared_secret).and_then(|(code, _)| {
                login::submit_code(&begin.client_id, &begin.steam_id, &code, 3)
            }) {
                Ok(()) => begin.guard_submitted = true,
                Err(_) => begin.auto_guard_failed = true,
            }
        }
    }
    Ok(begin)
}

pub fn validate_pending(
    workspace: &Path,
    shadow_login: &str,
    client_id: &str,
    request_id: &str,
) -> AppResult<()> {
    let mut map = pending().lock().unwrap();
    prune(&mut map);
    if map.get(client_id).is_some_and(|p| {
        p.workspace == workspace && p.login == shadow_login && p.request_id == request_id
    }) {
        Ok(())
    } else {
        Err(AppError::NotReady("AUTH_LOGIN_CANCELLED".into()))
    }
}

pub fn complete_manual(
    workspace: &Path,
    shadow_login: &str,
    client_id: &str,
    state: &PollState,
    allowed: Vec<i64>,
) -> AppResult<()> {
    let PollState::Done {
        access_token,
        refresh_token,
        account_name,
        steam_id,
        ..
    } = state
    else {
        return Ok(());
    };
    let gate = super::session::account_gate(workspace, shadow_login)?;
    let _guard = gate.lock().unwrap();
    // Serialize completion with cancellation/Forget. A cancelled attempt cannot
    // restore a password the user just removed.
    let mut map = pending().lock().unwrap();
    prune(&mut map);
    let attempt = map
        .remove(client_id)
        .ok_or_else(|| AppError::NotReady("AUTH_LOGIN_CANCELLED".into()))?;
    let sid = steam_id.parse::<u64>().unwrap_or(0);
    if attempt.workspace != workspace
        || attempt.login != shadow_login
        || sid != attempt.steam_id
        || (!account_name.is_empty() && !account_name.eq_ignore_ascii_case(&attempt.account_name))
    {
        return Err(AppError::NotReady("AUTH_ACCOUNT_MISMATCH".into()));
    }
    let session_id = new_session_id();
    if let Some(mut mf) = vault::load_plain(workspace, shadow_login)? {
        validate_identity(&mf, &attempt.account_name, sid)?;
        mf.session = Some(SessionData {
            steam_id: sid,
            access_token: access_token.clone(),
            refresh_token: refresh_token.clone(),
            session_id: session_id.clone(),
        });
        vault::save_plain(workspace, shadow_login, &mf)?;
        crate::workspace::set_authenticator_meta(
            workspace,
            shadow_login,
            mf.fully_enrolled != Some(false),
            Some(mf.account_name.clone()),
        )?;
    }
    if let Some(sealed) = attempt.sealed_password.as_ref() {
        credentials::save_sealed(workspace, shadow_login, sealed)?;
    } else {
        credentials::forget(workspace, shadow_login)?;
    }
    super::add::put_session(
        shadow_login,
        access_token.clone(),
        refresh_token.clone(),
        sid,
        session_id,
        allowed,
    );
    relogin_flag::clear(shadow_login);
    Ok(())
}

pub fn cancel(client_id: &str) {
    pending().lock().unwrap().remove(client_id);
}

pub fn forget(workspace: &Path, shadow_login: &str) -> AppResult<()> {
    let mut map = pending().lock().unwrap();
    map.retain(|_, p| p.workspace != workspace || p.login != shadow_login);
    credentials::forget(workspace, shadow_login)
}

pub fn new_session_id() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn automatic(workspace: &Path, shadow_login: &str, mf: &mut MaFile) -> AppResult<()> {
    let credentials =
        credentials::claim_attempt(workspace, shadow_login, session_state::now_secs())?;
    let result = authenticate(&credentials, mf, &mut SteamApi);
    match result {
        Ok(session) => {
            if !credentials::is_current(workspace, shadow_login, &credentials)? {
                return Err(AppError::NotReady("AUTH_LOGIN_CANCELLED".into()));
            }
            mf.session = Some(session);
            relogin_flag::clear(shadow_login);
            Ok(())
        }
        Err(error) => {
            if matches!(&error, AppError::NotReady(_)) {
                credentials::disable_attempt(workspace, shadow_login, &credentials)?;
            }
            relogin_flag::mark(shadow_login);
            Err(error)
        }
    }
}

trait AuthApi {
    fn begin(&mut self, account: &str, password: &str) -> AppResult<BeginOutcome>;
    fn guard(&mut self, begin: &BeginOutcome, shared_secret: &str) -> AppResult<()>;
    fn poll(&mut self, begin: &BeginOutcome) -> AppResult<PollState>;
    fn wait(&mut self, seconds: f64);
}
struct SteamApi;
impl AuthApi for SteamApi {
    fn begin(&mut self, account: &str, password: &str) -> AppResult<BeginOutcome> {
        login::begin(account, password)
    }
    fn guard(&mut self, begin: &BeginOutcome, secret: &str) -> AppResult<()> {
        let (code, _) = totp::generate_code_now(secret)?;
        login::submit_code(&begin.client_id, &begin.steam_id, &code, 3)
    }
    fn poll(&mut self, begin: &BeginOutcome) -> AppResult<PollState> {
        login::poll(&begin.client_id, &begin.request_id)
    }
    fn wait(&mut self, seconds: f64) {
        std::thread::sleep(Duration::from_secs_f64(seconds.clamp(2.0, 5.0)));
    }
}

fn authenticate(
    credentials: &Credentials,
    mf: &MaFile,
    api: &mut impl AuthApi,
) -> AppResult<SessionData> {
    validate_identity(mf, &credentials.account_name, credentials.steam_id)?;
    let begin = api.begin(&credentials.account_name, &credentials.password)?;
    if begin.steam_id != credentials.steam_id.to_string() {
        return Err(AppError::NotReady("AUTH_ACCOUNT_MISMATCH".into()));
    }
    if begin.allowed_confirmations.iter().any(|c| c.kind == 3) {
        api.guard(&begin, &mf.shared_secret)?;
    } else if !begin.allowed_confirmations.iter().any(|c| c.kind == 1) {
        return Err(AppError::NotReady("AUTH_AUTO_LOGIN_NEEDS_INPUT".into()));
    }
    let started = Instant::now();
    for _ in 0..15 {
        match api.poll(&begin)? {
            PollState::Done {
                access_token,
                refresh_token,
                account_name,
                steam_id,
                ..
            } => {
                if steam_id != credentials.steam_id.to_string()
                    || (!account_name.is_empty()
                        && !account_name.eq_ignore_ascii_case(&credentials.account_name))
                {
                    return Err(AppError::NotReady("AUTH_ACCOUNT_MISMATCH".into()));
                }
                return Ok(SessionData {
                    steam_id: credentials.steam_id,
                    access_token,
                    refresh_token,
                    session_id: new_session_id(),
                });
            }
            PollState::NeedsCode | PollState::Failed { .. } => {
                return Err(AppError::NotReady("AUTH_AUTO_LOGIN_NEEDS_INPUT".into()))
            }
            PollState::Pending => {}
        }
        if started.elapsed() >= Duration::from_secs(45) {
            break;
        }
        api.wait(begin.interval);
    }
    Err(AppError::Other("AUTH_AUTO_LOGIN_TIMEOUT".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    struct FakeApi {
        guard_type: i64,
        begin_id: String,
        guard_calls: usize,
        poll_calls: usize,
        finish: bool,
    }
    impl AuthApi for FakeApi {
        fn begin(&mut self, _: &str, _: &str) -> AppResult<BeginOutcome> {
            Ok(BeginOutcome {
                client_id: "client".into(),
                request_id: "request".into(),
                steam_id: self.begin_id.clone(),
                weak_token: String::new(),
                allowed_confirmations: vec![login::AllowedConfirmation {
                    kind: self.guard_type,
                    associated_message: String::new(),
                }],
                interval: 2.0,
                extended_domain: None,
                guard_submitted: false,
                auto_guard_failed: false,
            })
        }
        fn guard(&mut self, _: &BeginOutcome, _: &str) -> AppResult<()> {
            self.guard_calls += 1;
            Ok(())
        }
        fn poll(&mut self, _: &BeginOutcome) -> AppResult<PollState> {
            self.poll_calls += 1;
            if !self.finish {
                return Ok(PollState::Pending);
            }
            Ok(PollState::Done {
                access_token: "access".into(),
                refresh_token: "refresh".into(),
                account_name: "test".into(),
                steam_id: "1".into(),
                new_guard_data: None,
            })
        }
        fn wait(&mut self, _: f64) {}
    }
    fn fixture() -> (Credentials, MaFile, FakeApi) {
        let mut mf = MaFile::default();
        mf.account_name = "test".into();
        (
            Credentials::new("test".into(), "password".into(), 1),
            mf,
            FakeApi {
                guard_type: 3,
                begin_id: "1".into(),
                guard_calls: 0,
                poll_calls: 0,
                finish: true,
            },
        )
    }
    #[test]
    fn saved_password_and_guard_obtain_new_session() {
        let (credentials, mf, mut api) = fixture();
        let session = authenticate(&credentials, &mf, &mut api).unwrap();
        assert_eq!(session.access_token, "access");
        assert_eq!(session.session_id.len(), 24);
        assert_eq!(api.guard_calls, 1);
        assert_eq!(api.poll_calls, 1);
    }
    #[test]
    fn wrong_account_or_email_challenge_cannot_be_auto_approved() {
        let (credentials, mf, mut api) = fixture();
        api.begin_id = "2".into();
        assert!(authenticate(&credentials, &mf, &mut api).is_err());
        assert_eq!(api.guard_calls, 0);
        api.begin_id = "1".into();
        api.guard_type = 2;
        assert!(authenticate(&credentials, &mf, &mut api).is_err());
        assert_eq!(api.poll_calls, 0);
    }
    #[test]
    fn pending_login_has_a_bounded_number_of_attempts() {
        let (credentials, mf, mut api) = fixture();
        api.finish = false;
        assert!(authenticate(&credentials, &mf, &mut api).is_err());
        assert_eq!(api.poll_calls, 15);
    }

    fn pending_fixture(ws: &Path, client: &str, remember: bool) {
        let sealed = remember.then(|| {
            credentials::seal(&Credentials::new("test".into(), "test-password".into(), 1)).unwrap()
        });
        pending().lock().unwrap().insert(
            client.into(),
            PendingLogin {
                workspace: ws.into(),
                login: "test".into(),
                account_name: "test".into(),
                steam_id: 1,
                request_id: "request".into(),
                sealed_password: sealed,
                created: Instant::now(),
            },
        );
    }
    fn done() -> PollState {
        PollState::Done {
            access_token: "access".into(),
            refresh_token: "refresh".into(),
            account_name: "test".into(),
            steam_id: "1".into(),
            new_guard_data: None,
        }
    }
    fn clean_test_workspace(ws: PathBuf) {
        if ws.join("vault-policy.json").exists() {
            std::fs::remove_file(ws.join("vault-policy.json")).unwrap();
        }
        for p in [
            ws.join("accounts/test/auth"),
            ws.join("accounts/test"),
            ws.join("accounts"),
            ws,
        ] {
            if p.exists() {
                std::fs::remove_dir(p).unwrap();
            }
        }
    }
    #[test]
    fn password_saved_only_after_matching_success_and_optout_forgets_it() {
        let ws = std::env::temp_dir().join(format!("shadow-manual-test-{}", rand::random::<u64>()));
        pending_fixture(&ws, "save_success_test", true);
        complete_manual(
            &ws,
            "test",
            "save_success_test",
            &PollState::Pending,
            vec![],
        )
        .unwrap();
        assert!(!credentials::has_saved(&ws, "test"));
        complete_manual(&ws, "test", "save_success_test", &done(), vec![3]).unwrap();
        assert!(credentials::has_saved(&ws, "test"));
        pending_fixture(&ws, "forget_success_test", false);
        complete_manual(&ws, "test", "forget_success_test", &done(), vec![3]).unwrap();
        assert!(!credentials::has_saved(&ws, "test"));
        clean_test_workspace(ws);
    }
    #[test]
    fn cancelled_or_mismatched_login_cannot_persist_password() {
        let ws = std::env::temp_dir().join(format!("shadow-cancel-test-{}", rand::random::<u64>()));
        pending_fixture(&ws, "cancel_success_test", true);
        assert!(validate_pending(&ws, "another", "cancel_success_test", "request").is_err());
        forget(&ws, "test").unwrap();
        assert!(complete_manual(&ws, "test", "cancel_success_test", &done(), vec![]).is_err());
        pending_fixture(&ws, "mismatch_success_test", true);
        let mut wrong = done();
        if let PollState::Done { steam_id, .. } = &mut wrong {
            *steam_id = "2".into();
        }
        assert!(complete_manual(&ws, "test", "mismatch_success_test", &wrong, vec![]).is_err());
        assert!(!credentials::has_saved(&ws, "test"));
        clean_test_workspace(ws);
    }
}

pub fn clear_pending() {
    pending().lock().unwrap().clear();
}
