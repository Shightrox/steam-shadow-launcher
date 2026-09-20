//! Workspace-scoped keys and recoverable all-account encryption changes.
use super::{crypto, mafile::MaFile};
use crate::{
    error::{AppError, AppResult},
    storage, workspace,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};
use zeroize::Zeroizing;

static IO: Mutex<()> = Mutex::new(());
type Keys = HashMap<PathBuf, Zeroizing<String>>;
fn keys() -> &'static Mutex<Keys> {
    static CELL: OnceLock<Mutex<Keys>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(HashMap::new()))
}
const POLICY: &str = "vault-policy.json";
const TRANSACTION: &str = ".vault-transaction";
const CHECK: &[u8] = b"Steam Shadow vault key v1";
#[derive(Default, Serialize, Deserialize)]
struct Policy {
    encrypted: bool,
    verifier: Option<Vec<u8>>,
}

pub fn mafile_plain_path(ws: &Path, login: &str) -> AppResult<PathBuf> {
    Ok(workspace::auth_dir(ws, login)?.join("maFile.json"))
}
pub fn mafile_enc_path(ws: &Path, login: &str) -> AppResult<PathBuf> {
    Ok(workspace::auth_dir(ws, login)?.join("maFile.enc"))
}
pub fn has_any(ws: &Path, login: &str) -> bool {
    workspace::checked_account_dir(ws, login).is_ok_and(|dir| {
        dir.join("auth/maFile.json").is_file() || dir.join("auth/maFile.enc").is_file()
    })
}
fn logins(ws: &Path) -> AppResult<Vec<String>> {
    let mut result = Vec::new();
    let dir = workspace::accounts_dir(ws);
    if dir.exists() {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let login = entry.file_name().to_string_lossy().to_string();
            workspace::checked_account_dir(ws, &login)?;
            result.push(login);
        }
    }
    result.sort();
    Ok(result)
}
fn policy(ws: &Path, legacy_enabled: bool) -> AppResult<Policy> {
    if ws.join(POLICY).exists() {
        return Ok(serde_json::from_slice(&fs::read(ws.join(POLICY))?)?);
    }
    let encrypted = legacy_enabled
        || logins(ws)?.iter().any(|login| {
            workspace::account_dir(ws, login)
                .join("auth/maFile.enc")
                .is_file()
        });
    let p = Policy {
        encrypted,
        verifier: None,
    };
    storage::atomic_write(&ws.join(POLICY), &serde_json::to_vec(&p)?)?;
    Ok(p)
}
pub fn initialize(ws: &Path, legacy_enabled: bool) -> AppResult<bool> {
    let _io = IO.lock().unwrap();
    recover(ws)?;
    Ok(policy(ws, legacy_enabled)?.encrypted)
}
pub fn has_master_password_unlocked(ws: &Path) -> bool {
    storage::path_key(ws).is_ok_and(|key| keys().lock().unwrap().contains_key(&key))
}
pub fn lock() {
    let _io = IO.lock().unwrap();
    keys().lock().unwrap().clear();
}
fn password(ws: &Path) -> AppResult<Option<Zeroizing<String>>> {
    Ok(keys().lock().unwrap().get(&storage::path_key(ws)?).cloned())
}
fn validate_password(ws: &Path, pw: &str, p: &Policy) -> AppResult<()> {
    if let Some(blob) = &p.verifier {
        if crypto::vault_decrypt(pw, blob)? != CHECK {
            return Err(AppError::Other("VAULT_BAD_PASSWORD".into()));
        }
    }
    for login in logins(ws)? {
        let enc = mafile_enc_path(ws, &login)?;
        if enc.exists() {
            let plain = Zeroizing::new(crypto::vault_decrypt(pw, &fs::read(enc)?)?);
            MaFile::from_json_bytes(&plain)?;
        }
    }
    Ok(())
}
pub fn unlock(ws: &Path, pw: String) -> AppResult<()> {
    let pw = Zeroizing::new(pw);
    let _io = IO.lock().unwrap();
    recover(ws)?;
    let mut p = policy(ws, false)?;
    if !p.encrypted {
        return Err(AppError::NotReady("AUTH_ENCRYPTION_DISABLED".into()));
    }
    validate_password(ws, &pw, &p)?;
    let mut repairs = Vec::new();
    for login in logins(ws)? {
        if mafile_plain_path(ws, &login)?.exists() {
            // Repair plaintext left by old versions while encryption was enabled.
            let mf = read_with(ws, &login, Some(&pw))?.unwrap();
            let json = Zeroizing::new(mf.to_json_pretty()?);
            repairs.push((
                format!("accounts/{login}/auth/maFile.enc"),
                Some(Zeroizing::new(crypto::vault_encrypt(&pw, json.as_bytes())?)),
            ));
            repairs.push((format!("accounts/{login}/auth/maFile.json"), None));
        }
    }
    if p.verifier.is_none() {
        p.verifier = Some(crypto::vault_encrypt(&pw, CHECK)?);
        repairs.push((POLICY.into(), Some(Zeroizing::new(serde_json::to_vec(&p)?))));
    }
    if !repairs.is_empty() {
        transact(ws, repairs, None)?;
    }
    keys().lock().unwrap().insert(storage::path_key(ws)?, pw);
    Ok(())
}
fn read_with(ws: &Path, login: &str, pw: Option<&str>) -> AppResult<Option<MaFile>> {
    let enc = mafile_enc_path(ws, login)?;
    if enc.exists() {
        let pw = pw.ok_or_else(|| AppError::NotReady("AUTH_LOCKED".into()))?;
        let bytes = Zeroizing::new(crypto::vault_decrypt(pw, &fs::read(enc)?)?);
        return MaFile::from_json_bytes(&bytes).map(Some);
    }
    let plain = mafile_plain_path(ws, login)?;
    if plain.exists() {
        return MaFile::read_file(&plain).map(Some);
    }
    Ok(None)
}
pub fn load_plain(ws: &Path, login: &str) -> AppResult<Option<MaFile>> {
    let _io = IO.lock().unwrap();
    recover(ws)?;
    let p = policy(ws, false)?;
    let pw = password(ws)?;
    if p.encrypted && pw.is_none() {
        return Err(AppError::NotReady("AUTH_LOCKED".into()));
    }
    read_with(ws, login, pw.as_ref().map(|p| p.as_str()))
}
pub fn ensure_writable(ws: &Path) -> AppResult<()> {
    let _io = IO.lock().unwrap();
    recover(ws)?;
    if policy(ws, false)?.encrypted && password(ws)?.is_none() {
        return Err(AppError::NotReady("AUTH_LOCKED".into()));
    }
    Ok(())
}
pub fn save_plain(ws: &Path, login: &str, mf: &MaFile) -> AppResult<()> {
    let _io = IO.lock().unwrap();
    recover(ws)?;
    let p = policy(ws, false)?;
    let json = Zeroizing::new(mf.to_json_pretty()?);
    if p.encrypted {
        let pw = password(ws)?.ok_or_else(|| AppError::NotReady("AUTH_LOCKED".into()))?;
        storage::atomic_write(
            &mafile_enc_path(ws, login)?,
            &crypto::vault_encrypt(&pw, json.as_bytes())?,
        )?;
        remove_if_exists(&mafile_plain_path(ws, login)?)?;
    } else {
        if mafile_enc_path(ws, login)?.exists() {
            return Err(AppError::NotReady("AUTH_LOCKED".into()));
        }
        storage::atomic_write(&mafile_plain_path(ws, login)?, json.as_bytes())?;
    }
    Ok(())
}
pub fn remove(ws: &Path, login: &str) -> AppResult<()> {
    let _io = IO.lock().unwrap();
    recover(ws)?;
    remove_if_exists(&mafile_plain_path(ws, login)?)?;
    remove_if_exists(&mafile_enc_path(ws, login)?)
}
fn remove_if_exists(path: &Path) -> AppResult<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

#[derive(Serialize, Deserialize)]
struct Change {
    relative: String,
    existed: bool,
    write: bool,
}
fn target(ws: &Path, relative: &str) -> AppResult<PathBuf> {
    if relative == POLICY {
        return Ok(ws.join(POLICY));
    }
    let parts: Vec<_> = relative.split('/').collect();
    if parts.len() != 4
        || parts[0] != "accounts"
        || parts[2] != "auth"
        || !matches!(parts[3], "maFile.json" | "maFile.enc")
    {
        return Err(AppError::Config("VAULT_INVALID_JOURNAL".into()));
    }
    Ok(workspace::auth_dir(ws, parts[1])?.join(parts[3]))
}
fn cleanup(dir: &Path) -> AppResult<()> {
    // Once rollback/commit is complete, remove the manifest first. A crash
    // during cleanup must never replay a journal whose backups were deleted.
    remove_if_exists(&dir.join("journal.json"))?;
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() || storage::is_reparse(&entry.path()) {
            return Err(AppError::Config("VAULT_INVALID_JOURNAL".into()));
        }
        fs::remove_file(entry.path())?;
    }
    fs::remove_dir(dir)?;
    Ok(())
}
// Staged old AND new bytes are DPAPI-protected, including plaintext imports.
// Incomplete changes roll back before any reader can observe a mixed-key vault.
fn recover(ws: &Path) -> AppResult<()> {
    let dir = ws.join(TRANSACTION);
    if !dir.exists() {
        return Ok(());
    }
    if storage::is_reparse(&dir) {
        return Err(AppError::Config("VAULT_INVALID_JOURNAL".into()));
    }
    if dir.join("journal.json").exists() && !dir.join("committed").exists() {
        let changes: Vec<Change> = serde_json::from_slice(&fs::read(dir.join("journal.json"))?)?;
        for (i, change) in changes.iter().enumerate() {
            let path = target(ws, &change.relative)?;
            if change.existed {
                let original =
                    super::credentials::protect(&fs::read(dir.join(format!("{i}.old")))?, true)?;
                storage::atomic_write(&path, &original)?;
            } else {
                remove_if_exists(&path)?;
            }
        }
    }
    cleanup(&dir)
}
type Replacement = (String, Option<Zeroizing<Vec<u8>>>);
fn transact(ws: &Path, replacements: Vec<Replacement>, fail_after: Option<usize>) -> AppResult<()> {
    let dir = ws.join(TRANSACTION);
    fs::create_dir(&dir)?;
    let result = (|| -> AppResult<()> {
        let mut changes = Vec::new();
        for (i, (relative, bytes)) in replacements.iter().enumerate() {
            let path = target(ws, relative)?;
            let existed = path.exists();
            if existed {
                let original = Zeroizing::new(fs::read(path)?);
                storage::atomic_write(
                    &dir.join(format!("{i}.old")),
                    &super::credentials::protect(&original, false)?,
                )?;
            }
            if let Some(bytes) = bytes {
                storage::atomic_write(
                    &dir.join(format!("{i}.new")),
                    &super::credentials::protect(bytes, false)?,
                )?;
            }
            changes.push(Change {
                relative: relative.clone(),
                existed,
                write: bytes.is_some(),
            });
        }
        storage::atomic_write(&dir.join("journal.json"), &serde_json::to_vec(&changes)?)?;
        for (i, change) in changes.iter().enumerate() {
            if fail_after == Some(i) {
                return Err(AppError::Io("injected transaction failure".into()));
            }
            let path = target(ws, &change.relative)?;
            if change.write {
                let bytes =
                    super::credentials::protect(&fs::read(dir.join(format!("{i}.new")))?, true)?;
                storage::atomic_write(&path, &bytes)?;
            } else {
                remove_if_exists(&path)?;
            }
        }
        storage::atomic_write(&dir.join("committed"), b"1")?;
        Ok(())
    })();
    if let Err(error) = result {
        recover(ws)?;
        return Err(error);
    }
    if let Err(error) = cleanup(&dir) {
        tracing::warn!("Vault transaction cleanup deferred: {error}");
    }
    Ok(())
}
pub fn rekey_all(
    ws: &Path,
    old_password: Option<&str>,
    new_password: Option<&str>,
) -> AppResult<()> {
    let _io = IO.lock().unwrap();
    recover(ws)?;
    let p = policy(ws, false)?;
    if p.encrypted {
        let old =
            old_password.ok_or_else(|| AppError::NotReady("AUTH_NEEDS_OLD_PASSWORD".into()))?;
        validate_password(ws, old, &p)?;
    }
    if new_password.is_some_and(str::is_empty) {
        return Err(AppError::Config("VAULT_EMPTY_PASSWORD".into()));
    }
    let mut changes = Vec::new();
    for login in logins(ws)? {
        if let Some(mf) = read_with(ws, &login, old_password)? {
            let json = Zeroizing::new(mf.to_json_pretty()?.into_bytes());
            let (ext, stale, bytes) = match new_password {
                Some(pw) => (
                    "enc",
                    "json",
                    Zeroizing::new(crypto::vault_encrypt(pw, &json)?),
                ),
                None => ("json", "enc", json),
            };
            changes.push((format!("accounts/{login}/auth/maFile.{ext}"), Some(bytes)));
            changes.push((format!("accounts/{login}/auth/maFile.{stale}"), None));
        }
    }
    let next = Policy {
        encrypted: new_password.is_some(),
        verifier: new_password
            .map(|pw| crypto::vault_encrypt(pw, CHECK))
            .transpose()?,
    };
    changes.push((
        POLICY.into(),
        Some(Zeroizing::new(serde_json::to_vec(&next)?)),
    ));
    transact(ws, changes, None)?;
    let id = storage::path_key(ws)?;
    let mut keys = keys().lock().unwrap();
    keys.remove(&id);
    if let Some(pw) = new_password {
        keys.insert(id, Zeroizing::new(pw.to_string()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let ws = std::env::temp_dir().join(format!("shadow-vault-{}", rand::random::<u64>()));
            fs::create_dir(&ws).unwrap();
            Self(ws)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let canonical = fs::canonicalize(&self.0).unwrap();
            assert!(canonical.starts_with(fs::canonicalize(std::env::temp_dir()).unwrap()));
            keys()
                .lock()
                .unwrap()
                .remove(&storage::path_key(&self.0).unwrap());
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    #[test]
    fn locked_vault_rejects_new_plaintext_and_keys_are_workspace_scoped() {
        let f = Fixture::new();
        let other = Fixture::new();
        rekey_all(&f.0, None, Some("master")).unwrap();
        keys()
            .lock()
            .unwrap()
            .remove(&storage::path_key(&f.0).unwrap());
        assert!(save_plain(&f.0, "new", &MaFile::default())
            .unwrap_err()
            .to_string()
            .contains("AUTH_LOCKED"));
        assert!(!mafile_plain_path(&f.0, "new").unwrap().exists());
        unlock(&f.0, "master".into()).unwrap();
        assert!(!has_master_password_unlocked(&other.0));
        assert!(unlock(&f.0, "wrong".into()).is_err());
        assert!(has_master_password_unlocked(&f.0));
    }
    #[test]
    fn failed_change_does_not_poison_key_or_change_any_file() {
        let f = Fixture::new();
        let mut mf = MaFile::default();
        mf.shared_secret = "MTIzNDU2Nzg5MDEyMzQ1Njc4OTA=".into();
        mf.account_name = "aaa".into();
        save_plain(&f.0, "aaa", &mf).unwrap();
        rekey_all(&f.0, None, Some("old")).unwrap();
        assert!(rekey_all(&f.0, Some("wrong"), Some("new")).is_err());
        assert!(load_plain(&f.0, "aaa").unwrap().is_some());
        let original = fs::read(mafile_enc_path(&f.0, "aaa").unwrap()).unwrap();
        fs::write(mafile_enc_path(&f.0, "zzz").unwrap(), b"broken").unwrap();
        assert!(rekey_all(&f.0, Some("old"), Some("new")).is_err());
        assert_eq!(
            fs::read(mafile_enc_path(&f.0, "aaa").unwrap()).unwrap(),
            original
        );
    }
    #[test]
    fn interrupted_commit_rolls_back_before_reading() {
        let f = Fixture::new();
        let _io = IO.lock().unwrap();
        let file = mafile_plain_path(&f.0, "demo").unwrap();
        fs::write(&file, b"original").unwrap();
        let result = transact(
            &f.0,
            vec![
                (
                    "accounts/demo/auth/maFile.json".into(),
                    Some(Zeroizing::new(b"changed".to_vec())),
                ),
                (POLICY.into(), Some(Zeroizing::new(b"{}".to_vec()))),
            ],
            Some(1),
        );
        assert!(result.is_err());
        assert_eq!(fs::read(file).unwrap(), b"original");
        assert!(!f.0.join(TRANSACTION).exists());
    }
    #[test]
    fn restart_recovers_an_uncommitted_journal() {
        let f = Fixture::new();
        let file = mafile_plain_path(&f.0, "demo").unwrap();
        let dir = f.0.join(TRANSACTION);
        fs::create_dir(&dir).unwrap();
        fs::write(&file, b"partially-replaced").unwrap();
        let protected = super::super::credentials::protect(b"original-secret", false).unwrap();
        assert!(!protected.windows(15).any(|w| w == b"original-secret"));
        fs::write(dir.join("0.old"), &protected).unwrap();
        let change = Change {
            relative: "accounts/demo/auth/maFile.json".into(),
            existed: true,
            write: true,
        };
        fs::write(
            dir.join("journal.json"),
            serde_json::to_vec(&vec![change]).unwrap(),
        )
        .unwrap();
        initialize(&f.0, false).unwrap();
        assert_eq!(fs::read(file).unwrap(), b"original-secret");
        assert!(!dir.exists());
    }
    #[test]
    fn unlock_repairs_plaintext_left_by_old_versions() {
        let f = Fixture::new();
        rekey_all(&f.0, None, Some("pw")).unwrap();
        let mut mf = MaFile::default();
        mf.account_name = "demo".into();
        mf.shared_secret = "MTIzNDU2Nzg5MDEyMzQ1Njc4OTA=".into();
        let plain = mafile_plain_path(&f.0, "demo").unwrap();
        fs::write(&plain, mf.to_json_pretty().unwrap()).unwrap();
        unlock(&f.0, "pw".into()).unwrap();
        assert!(!plain.exists());
        assert_eq!(
            load_plain(&f.0, "demo").unwrap().unwrap().shared_secret,
            mf.shared_secret
        );
    }
}
