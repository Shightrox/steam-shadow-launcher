use crate::error::{AppError, AppResult};
use crate::junction::{self, JunctionHealth};
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
pub struct AccountHealth {
    pub junction: JunctionHealth,
    #[serde(rename = "configDirExists")]
    pub config_dir_exists: bool,
    #[serde(rename = "hasLoginusersVdf")]
    pub has_loginusers_vdf: bool,
    pub ready: bool,
}

pub fn is_account_ready(account_dir: &Path, main_steamapps: &Path) -> AppResult<AccountHealth> {
    let link = account_dir.join("steamapps");
    let j = junction::verify(&link, main_steamapps)?;
    let config_dir = account_dir.join("config");
    let config_dir_exists = config_dir.exists();
    let has_loginusers = config_dir.join("loginusers.vdf").exists();
    let ready = matches!(j, JunctionHealth::Healthy) && config_dir_exists;
    Ok(AccountHealth {
        junction: j,
        config_dir_exists,
        has_loginusers_vdf: has_loginusers,
        ready,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct DiscoveredAccount {
    #[serde(rename = "accountName")]
    pub account_name: String,
    #[serde(rename = "personaName")]
    pub persona_name: Option<String>,
    #[serde(rename = "mostRecent")]
    pub most_recent: bool,
}

/// Parse the main Steam install's `config/loginusers.vdf` to discover
/// previously logged-in accounts (so we can offer one-click import).
pub fn parse_loginusers(steam_install_dir: &Path) -> AppResult<Vec<DiscoveredAccount>> {
    let path = steam_install_dir.join("config").join("loginusers.vdf");
    if !path.exists() {
        return Ok(vec![]);
    }
    let bytes = std::fs::read(&path)
        .map_err(|e| AppError::Io(format!("read loginusers.vdf: {e}")))?;
    let txt = String::from_utf8_lossy(&bytes);
    Ok(parse_loginusers_text(&txt))
}

/// Minimal hand-rolled VDF subset parser tuned for loginusers.vdf shape:
/// "users" {
///   "76561198..." {
///     "AccountName"  "foo"
///     "PersonaName"  "Foo"
///     "MostRecent"   "1"
///   }
/// }
fn parse_loginusers_text(txt: &str) -> Vec<DiscoveredAccount> {
    let mut out = Vec::new();
    let mut current: Option<(Option<String>, Option<String>, bool)> = None;
    let mut depth: i32 = 0;
    let mut at_user_block = false;

    for raw_line in txt.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        if line == "{" {
            depth += 1;
            if depth == 2 {
                at_user_block = true;
                current = Some((None, None, false));
            }
            continue;
        }
        if line == "}" {
            if depth == 2 {
                if let Some((name, persona, recent)) = current.take() {
                    if let Some(n) = name {
                        out.push(DiscoveredAccount {
                            account_name: n,
                            persona_name: persona,
                            most_recent: recent,
                        });
                    }
                }
                at_user_block = false;
            }
            depth -= 1;
            continue;
        }
        if !at_user_block {
            continue;
        }
        // Key/value: "Key"  "Value"
        let parts: Vec<&str> = line.split('"').collect();
        // Expected shape: ["", key, "<sep>", value, ""] or similar
        if parts.len() >= 5 {
            let key = parts[1];
            let val = parts[3];
            if let Some(c) = current.as_mut() {
                match key {
                    "AccountName" => c.0 = Some(val.to_string()),
                    "PersonaName" => c.1 = Some(val.to_string()),
                    "MostRecent" => c.2 = val == "1",
                    _ => {}
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_loginusers() {
        let sample = r#"
"users"
{
    "76561198000000001"
    {
        "AccountName"   "alice"
        "PersonaName"   "Alice"
        "RememberPassword"  "1"
        "MostRecent"        "1"
    }
    "76561198000000002"
    {
        "AccountName"   "bob"
        "PersonaName"   "Bob"
        "MostRecent"        "0"
    }
}
"#;
        let accounts = parse_loginusers_text(sample);
        assert_eq!(accounts.len(), 2);
        assert_eq!(accounts[0].account_name, "alice");
        assert_eq!(accounts[0].persona_name.as_deref(), Some("Alice"));
        assert!(accounts[0].most_recent);
        assert_eq!(accounts[1].account_name, "bob");
        assert!(!accounts[1].most_recent);
    }
}
