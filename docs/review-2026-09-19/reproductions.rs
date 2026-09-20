use crate::{
    sda::{crypto, mafile::MaFile, vault},
    workspace,
};
use std::{fs, path::PathBuf};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let p =
            std::env::temp_dir().join(format!("shadow-review-fixture-{}", rand::random::<u64>()));
        fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        vault::set_master_password(None);
        let p = fs::canonicalize(&self.0).unwrap();
        let temp = fs::canonicalize(std::env::temp_dir()).unwrap();
        assert!(p.starts_with(&temp));
        assert!(p
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("shadow-review-fixture-"));
        fn unlink(p: &std::path::Path) {
            for e in fs::read_dir(p).unwrap() {
                let e = e.unwrap();
                if crate::junction::is_junction(&e.path()) {
                    crate::junction::remove(&e.path()).unwrap()
                } else if e.path().is_dir() {
                    unlink(&e.path());
                }
            }
        }
        unlink(&p);
        fs::remove_dir_all(p).unwrap();
    }
}
fn mafile() -> MaFile {
    let mut mf = MaFile::default();
    mf.account_name = "review_fake_account".into();
    mf.shared_secret = "MTIzNDU2Nzg5MDEyMzQ1Njc4OTA=".into();
    mf
}
#[test]
fn review_dot_login_escapes_accounts_directory() {
    let f = Fixture::new();
    let ws = f.0.join("workspace");
    assert!(workspace::validate_login("..").is_ok());
    let created = workspace::ensure_account_dirs(&ws, "..").unwrap();
    assert_eq!(
        fs::canonicalize(created).unwrap(),
        fs::canonicalize(&ws).unwrap()
    );
    assert!(ws.join("config").is_dir());
}
#[test]
fn review_duplicate_add_resets_authenticator_and_favorite() {
    let f = Fixture::new();
    let ws = f.0.join("workspace");
    let lib = f.0.join("library");
    fs::create_dir(&lib).unwrap();
    workspace::add_account(&ws, &lib, "review_fake_account", None).unwrap();
    workspace::set_authenticator_meta(
        &ws,
        "review_fake_account",
        true,
        Some("review_fake_account".into()),
    )
    .unwrap();
    workspace::set_favorite(&ws, "review_fake_account", true).unwrap();
    vault::save_plain(&ws, "review_fake_account", &mafile()).unwrap();
    let replaced = workspace::add_account(&ws, &lib, "review_fake_account", None).unwrap();
    assert!(!replaced.has_authenticator);
    assert!(!replaced.favorite);
    assert!(vault::has_any(&ws, "review_fake_account"));
}
#[test]
fn review_move_merges_and_overwrites_existing_destination_account() {
    let f = Fixture::new();
    let old = f.0.join("old");
    let new = f.0.join("new");
    let lib = f.0.join("library");
    fs::create_dir(&lib).unwrap();
    for ws in [&old, &new] {
        workspace::ensure_account_dirs(ws, "demo").unwrap();
        fs::create_dir_all(ws.join("accounts/demo/auth")).unwrap();
    }
    fs::write(
        old.join("accounts/demo/auth/maFile.json"),
        "OLD_WORKSPACE_SECRET",
    )
    .unwrap();
    fs::write(
        new.join("accounts/demo/auth/maFile.json"),
        "DESTINATION_SECRET",
    )
    .unwrap();
    workspace::change_workspace(&old, &new, workspace::ChangeStrategy::Move, &lib).unwrap();
    assert_eq!(
        fs::read_to_string(new.join("accounts/demo/auth/maFile.json")).unwrap(),
        "OLD_WORKSPACE_SECRET"
    );
    assert!(!old.join("accounts").exists());
}
#[test]
fn review_locked_vault_writes_new_account_in_plaintext() {
    let f = Fixture::new();
    let ws = f.0.join("workspace");
    let old = workspace::auth_dir(&ws, "old").unwrap();
    fs::write(
        old.join("maFile.enc"),
        crypto::vault_encrypt("test-master", mafile().to_json_pretty().unwrap().as_bytes())
            .unwrap(),
    )
    .unwrap();
    vault::set_master_password(None);
    vault::save_plain(&ws, "new", &mafile()).unwrap();
    assert!(ws.join("accounts/new/auth/maFile.json").is_file());
    assert!(!ws.join("accounts/new/auth/maFile.enc").exists());
}
#[test]
fn review_partial_rekey_leaves_first_account_unreadable_with_cached_key() {
    let f = Fixture::new();
    let ws = f.0.join("workspace");
    for name in ["aaa", "zzz"] {
        workspace::ensure_account_dirs(&ws, name).unwrap();
    }
    vault::set_master_password(Some("old-master".into()));
    vault::save_plain(&ws, "aaa", &mafile()).unwrap();
    let broken = workspace::auth_dir(&ws, "zzz").unwrap().join("maFile.enc");
    fs::write(broken, b"invalid-fixture").unwrap();
    assert!(vault::rekey_all(&ws, Some("new-master")).is_err());
    assert!(vault::load_plain(&ws, "aaa").is_err());
    let blob = fs::read(ws.join("accounts/aaa/auth/maFile.enc")).unwrap();
    assert!(crypto::vault_decrypt("new-master", &blob).is_ok());
    assert!(crypto::vault_decrypt("old-master", &blob).is_err());
}
#[test]
fn review_reqwest_display_contains_token_query() {
    let e = crate::http::shared()
        .post("http://127.0.0.1:1/?access_token=FAKE_REVIEW_TOKEN")
        .timeout(std::time::Duration::from_millis(200))
        .send()
        .unwrap_err();
    assert!(e.to_string().contains("FAKE_REVIEW_TOKEN"));
}

struct FakeAppData(Option<std::ffi::OsString>);
impl FakeAppData {
    fn new(path: &std::path::Path) -> Self {
        let old = std::env::var_os("APPDATA");
        std::env::set_var("APPDATA", path);
        Self(old)
    }
}
impl Drop for FakeAppData {
    fn drop(&mut self) {
        match &self.0 {
            Some(p) => std::env::set_var("APPDATA", p),
            None => std::env::remove_var("APPDATA"),
        }
    }
}
#[test]
fn review_failed_master_password_change_changes_cached_key() {
    let f = Fixture::new();
    let _env = FakeAppData::new(&f.0.join("roaming"));
    let ws = f.0.join("workspace");
    let settings = crate::settings::Settings {
        workspace: Some(ws.clone()),
        auth_master_password_enabled: true,
        ..crate::settings::Settings::default()
    };
    crate::settings::save(&settings).unwrap();
    vault::set_master_password(Some("correct-master".into()));
    vault::save_plain(&ws, "demo", &mafile()).unwrap();
    assert!(crate::commands::auth_set_master_password(
        Some("wrong-master".into()),
        Some("new-master".into())
    )
    .is_err());
    assert!(vault::has_master_password_unlocked());
    assert!(vault::load_plain(&ws, "demo").is_err());
    vault::save_plain(&ws, "fresh", &mafile()).unwrap();
    let blob = fs::read(ws.join("accounts/fresh/auth/maFile.enc")).unwrap();
    assert!(crypto::vault_decrypt("wrong-master", &blob).is_ok());
    assert!(crypto::vault_decrypt("correct-master", &blob).is_err());
}
#[test]
fn review_import_accepts_another_steam_account() {
    let f = Fixture::new();
    let _env = FakeAppData::new(&f.0.join("roaming"));
    let ws = f.0.join("workspace");
    let settings = crate::settings::Settings {
        workspace: Some(ws.clone()),
        ..crate::settings::Settings::default()
    };
    crate::settings::save(&settings).unwrap();
    workspace::ensure_account_dirs(&ws, "account_a").unwrap();
    fs::write(
        ws.join("accounts/account_a/shadow.json"),
        r#"{"steamId":"111"}"#,
    )
    .unwrap();
    let mut mf = mafile();
    mf.account_name = "account_b".into();
    mf.session = Some(crate::sda::mafile::SessionData {
        steam_id: 222,
        access_token: String::new(),
        refresh_token: String::new(),
        session_id: String::new(),
    });
    crate::commands::auth_import_mafile("account_a".into(), mf.to_json_pretty().unwrap(), None)
        .unwrap();
    assert_eq!(
        vault::load_plain(&ws, "account_a")
            .unwrap()
            .unwrap()
            .account_name,
        "account_b"
    );
    let status = crate::commands::auth_status().unwrap();
    assert_eq!(status[0].account_name.as_deref(), Some("account_a"));
}
