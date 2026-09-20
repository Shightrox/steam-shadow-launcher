//! Steam mobile confirmations (trades / market / login / etc).
//!
//! Endpoints (as of 2024-2025):
//! - `GET  https://steamcommunity.com/mobileconf/getlist?<query>` → JSON list.
//! - `POST https://steamcommunity.com/mobileconf/multiajaxop` → bulk allow/reject.
//!
//! Query params required on every call:
//!   p   = device_id         (maFile.device_id, "android:<guid>")
//!   a   = steam_id (u64)    (Session.steam_id)
//!   k   = base64 HMAC-SHA1(identity_secret, be8(t) || tag)  (URL-encoded)
//!   t   = server_time       (unix seconds, server-aligned)
//!   m   = "react"           (UI flavour Steam expects)
//!   tag = "list" | "conf" | "details" | "accept" | "reject"
//!
//! Cookies required for session authentication:
//!   sessionid           = Session.session_id
//!   steamLoginSecure    = "<steam_id>||<access_token>"
//!   mobileClient        = "android"
//!   mobileClientVersion = "777777 3.6.4"
//!   Steam_Language      = "english"

use crate::error::{AppError, AppResult};
use crate::http;
use crate::sda::mafile::MaFile;
use crate::sda::totp;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};

const BASE: &str = "https://steamcommunity.com/mobileconf";

/// One entry from `/mobileconf/getlist`.
///
/// Field names match Valve's JSON shape (camelCase via serde rename where needed).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Confirmation {
    /// Confirmation ID (string of digits).
    pub id: String,
    /// Opaque nonce for this confirmation, used as `ck=` on accept/reject.
    pub nonce: String,
    /// Either the trade-offer id, market-listing id, or 0 for login etc.
    #[serde(default, deserialize_with = "id_string")]
    pub creator_id: String,
    /// Short headline (e.g. "Trade with Alice").
    #[serde(default)]
    pub headline: String,
    /// Multi-line details.
    #[serde(default)]
    pub summary: Vec<String>,
    /// `1`=test, `2`=trade, `3`=market, `5`=phone, `6`=account recovery.
    /// We pass through the raw int so the UI can localise.
    #[serde(rename = "type")]
    pub kind: i64,
    #[serde(default)]
    pub type_name: String,
    #[serde(default)]
    pub accept: String,
    #[serde(default)]
    pub cancel: String,
    #[serde(default)]
    pub icon: String,
}

/// Wire shape of the `/mobileconf/getlist` response.
#[derive(Debug, Deserialize)]
struct ListResponse {
    #[serde(default)]
    success: bool,
    #[serde(default)]
    needauth: bool,
    #[serde(default)]
    message: String,
    #[serde(default)]
    detail: String,
    #[serde(default)]
    conf: Vec<Confirmation>,
}

/// One row in a multi-ajax response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RespondResult {
    pub id: String,
    pub success: bool,
    #[serde(default)]
    pub message: String,
}

/// Display data only. Approval still re-fetches the pending confirmation/nonce.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TradeDetails {
    pub partner_steam_id: String,
    pub giving: Vec<TradeItem>,
    pub receiving: Vec<TradeItem>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TradeItem {
    pub asset_id: String,
    pub app_id: u32,
    pub name: String,
    pub amount: String,
    pub icon: String,
}

#[derive(Deserialize)]
struct OfferAsset {
    appid: u32,
    #[serde(deserialize_with = "id_string")]
    assetid: String,
    #[serde(deserialize_with = "id_string")]
    classid: String,
    #[serde(default, deserialize_with = "id_string")]
    instanceid: String,
    #[serde(deserialize_with = "id_string")]
    amount: String,
}

#[derive(Deserialize)]
struct AssetDescription {
    appid: u32,
    #[serde(deserialize_with = "id_string")]
    classid: String,
    #[serde(default, deserialize_with = "id_string")]
    instanceid: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    market_hash_name: String,
    #[serde(default)]
    icon_url: String,
}

#[derive(Deserialize)]
struct DisplayOffer {
    #[serde(deserialize_with = "id_string")]
    tradeofferid: String,
    accountid_other: u32,
    trade_offer_state: u32,
    #[serde(default)]
    items_to_give: Vec<OfferAsset>,
    #[serde(default)]
    items_to_receive: Vec<OfferAsset>,
}

fn decode_trade_details(body: &str, expected_id: &str) -> AppResult<TradeDetails> {
    #[derive(Deserialize)]
    struct Wire {
        response: OfferResponse,
    }
    #[derive(Deserialize)]
    struct OfferResponse {
        offer: DisplayOffer,
        #[serde(default)]
        descriptions: Vec<AssetDescription>,
    }
    let wire: Wire = serde_json::from_str(body)
        .map_err(|_| AppError::Other("CONF_DETAILS_UNAVAILABLE".into()))?;
    let response = wire.response;
    let offer = response.offer;
    if offer.tradeofferid != expected_id || offer.trade_offer_state != 9 {
        return Err(AppError::Other("CONF_DETAILS_STALE".into()));
    }
    if (offer.items_to_give.is_empty() && offer.items_to_receive.is_empty())
        || offer
            .items_to_give
            .iter()
            .chain(&offer.items_to_receive)
            .any(|a| a.amount.parse::<u64>().map_or(true, |n| n == 0))
    {
        return Err(AppError::Other("CONF_DETAILS_UNAVAILABLE".into()));
    }
    let items = |assets: Vec<OfferAsset>| {
        assets
            .into_iter()
            .map(|a| {
                // Names/icons are keyed by ALL three values, never just classid.
                let d = response.descriptions.iter().find(|d| {
                    d.appid == a.appid && d.classid == a.classid && d.instanceid == a.instanceid
                });
                let name = d
                    .map(|d| {
                        if d.name.is_empty() {
                            &d.market_hash_name
                        } else {
                            &d.name
                        }
                    })
                    .cloned()
                    .unwrap_or_default();
                let icon = d
                    .filter(|d| !d.icon_url.is_empty())
                    .map(|d| {
                        format!(
                            "https://community.akamai.steamstatic.com/economy/image/{}/128fx128f",
                            d.icon_url
                        )
                    })
                    .unwrap_or_default();
                TradeItem {
                    asset_id: a.assetid,
                    app_id: a.appid,
                    name,
                    amount: a.amount,
                    icon,
                }
            })
            .collect()
    };
    Ok(TradeDetails {
        partner_steam_id: (76561197960265728u64 + u64::from(offer.accountid_other)).to_string(),
        giving: items(offer.items_to_give),
        receiving: items(offer.items_to_receive),
    })
}

/// Resolve a pending confirmation on this account before looking up its offer.
/// No HTML from Steam is rendered in the privileged application webview.
pub fn trade_details(mafile: &MaFile, login: &str, id: &str) -> AppResult<TradeDetails> {
    let pending = list(mafile, login)?;
    let confirmation = pending
        .iter()
        .find(|c| c.id == id && c.kind == 2)
        .ok_or_else(|| AppError::NotFound("CONF_NOT_FOUND".into()))?;
    let session = mafile
        .session
        .as_ref()
        .ok_or_else(|| AppError::NotReady("CONF_NO_SESSION".into()))?;
    let resp = http::shared()
        .get("https://api.steampowered.com/IEconService/GetTradeOffer/v1/")
        .query(&[
            ("access_token", session.access_token.as_str()),
            ("tradeofferid", confirmation.creator_id.as_str()),
            ("get_descriptions", "true"),
            ("language", "english"),
        ])
        .send()
        .map_err(|e| AppError::Other(format!("GetTradeOffer: {}", e.without_url())))?;
    // Economy permission failures must not invalidate mobileconf's session.
    if !resp.status().is_success() {
        return Err(AppError::Other(format!(
            "CONF_DETAILS_UNAVAILABLE: HTTP {}",
            resp.status()
        )));
    }
    decode_trade_details(
        &response_body(resp, "GetTradeOffer")?,
        &confirmation.creator_id,
    )
}

#[derive(Debug, Deserialize)]
struct RespondWire {
    #[serde(default)]
    success: bool,
    #[serde(default)]
    needauth: bool,
    #[serde(default)]
    message: String,
    #[serde(default)]
    detail: String,
}

fn id_string<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Id {
        Text(String),
        Number(u64),
    }
    Ok(match Id::deserialize(d)? {
        Id::Text(s) => s,
        Id::Number(n) => n.to_string(),
    })
}

fn failure_message(message: String, detail: String) -> String {
    if !message.is_empty() {
        message
    } else if !detail.is_empty() {
        detail
    } else {
        "Steam rejected the confirmation request".into()
    }
}

fn decode_list(body: &str) -> AppResult<Vec<Confirmation>> {
    let parsed: ListResponse =
        serde_json::from_str(body).map_err(|e| AppError::Other(format!("getlist JSON: {e}")))?;
    if parsed.needauth {
        return Err(AppError::NotReady("CONF_SESSION_EXPIRED".into()));
    }
    if !parsed.success {
        return Err(AppError::Other(format!(
            "CONF_FAIL: {}",
            failure_message(parsed.message, parsed.detail)
        )));
    }
    Ok(parsed.conf)
}

fn decode_response(body: &str, ids: impl Iterator<Item = String>) -> AppResult<Vec<RespondResult>> {
    let parsed: RespondWire = serde_json::from_str(body)
        .map_err(|e| AppError::Other(format!("multiajaxop JSON: {e}")))?;
    if parsed.needauth {
        return Err(AppError::NotReady("CONF_SESSION_EXPIRED".into()));
    }
    let message = if parsed.success {
        String::new()
    } else {
        failure_message(parsed.message, parsed.detail)
    };
    Ok(ids
        .map(|id| RespondResult {
            id,
            success: parsed.success,
            message: message.clone(),
        })
        .collect())
}

fn response_body(resp: reqwest::blocking::Response, operation: &str) -> AppResult<String> {
    let status = resp.status();
    if matches!(status.as_u16(), 401 | 403) || resp.url().path().starts_with("/login") {
        return Err(AppError::NotReady("CONF_SESSION_EXPIRED".into()));
    }
    if !status.is_success() {
        return Err(AppError::Other(format!("{operation} HTTP {status}")));
    }
    resp.text()
        .map_err(|e| AppError::Other(format!("{operation} read: {}", e.without_url())))
}

/// `allow` accepts the confirmation, `cancel` rejects it.
#[derive(Debug, Clone, Copy)]
pub enum Op {
    Allow,
    Reject,
}

impl Op {
    fn as_str(self) -> &'static str {
        match self {
            Op::Allow => "allow",
            Op::Reject => "cancel",
        }
    }
    fn tag(self) -> &'static str {
        match self {
            Op::Allow => "accept",
            Op::Reject => "reject",
        }
    }
}

/// Build a session-cookie-bearing HTTP client.
pub fn session_client(mafile: &MaFile) -> AppResult<Client> {
    if mafile.identity_secret.trim().is_empty() {
        return Err(AppError::NotReady("CONF_NO_IDENTITY_SECRET".into()));
    }
    if mafile.device_id.trim().is_empty() {
        return Err(AppError::NotReady("CONF_NO_DEVICE_ID".into()));
    }
    let session = mafile
        .session
        .as_ref()
        .ok_or_else(|| AppError::NotReady("CONF_NO_SESSION".into()))?;
    if session.access_token.trim().is_empty() {
        return Err(AppError::NotReady("CONF_NO_ACCESS_TOKEN".into()));
    }
    if session.session_id.trim().is_empty() {
        return Err(AppError::NotReady("CONF_NO_SESSION_ID".into()));
    }
    if session.steam_id == 0 {
        return Err(AppError::NotReady("CONF_NO_STEAM_ID".into()));
    }
    // `reqwest`'s built-in cookie jar is opaque once the client is built;
    // easier for our purposes is to pass cookies via the Cookie header on
    // each request via helper `cookie_header`, since the endpoints are all
    // same-origin (steamcommunity.com).
    Ok(http::new_session_client())
}

/// Assemble the `Cookie:` header value for mobileconf requests.
fn cookie_header(mafile: &MaFile) -> String {
    let s = mafile.session.as_ref().expect("checked by session_client");
    format!(
        "sessionid={sid}; steamLoginSecure={steamid}%7C%7C{tok}; mobileClient=android; \
         mobileClientVersion=777777%203.6.4; Steam_Language=english",
        sid = s.session_id,
        steamid = s.steam_id,
        tok = s.access_token,
    )
}

/// URL-encode `s` using a minimal subset sufficient for SDA query params
/// (HMAC base64 may contain `/`, `+`, `=`, all of which need escaping).
fn pct_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char);
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// Build the shared `?p=&a=&k=&t=&m=&tag=` query string for a given tag.
fn query_params(mafile: &MaFile, tag: &str) -> AppResult<String> {
    let session = mafile
        .session
        .as_ref()
        .ok_or_else(|| AppError::NotReady("CONF_NO_SESSION".into()))?;
    // Steam rejects replayed HMACs. A manual reload, nonce lookup or auth retry
    // can reuse the same second, so allocate a distinct time per account/tag.
    static TIMES: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<(u64, String), i64>>,
    > = std::sync::OnceLock::new();
    let now = totp::server_time();
    let t = {
        let mut times = TIMES.get_or_init(Default::default).lock().unwrap();
        let previous = times
            .entry((session.steam_id, tag.to_string()))
            .or_insert(0);
        let time = now.max(*previous + 1);
        if time > now + 30 {
            return Err(AppError::Other(
                "CONF_RATE_LIMIT: wait before retrying confirmations".into(),
            ));
        }
        *previous = time;
        time
    };
    let k = totp::confirmation_key(&mafile.identity_secret, t, tag)?;
    Ok(format!(
        "p={p}&a={a}&k={k}&t={t}&m=react&tag={tag}",
        p = pct_encode(&mafile.device_id),
        a = session.steam_id,
        k = pct_encode(&k),
        t = t,
        tag = tag,
    ))
}

/// Fetch the current confirmation list.
pub fn list(mafile: &MaFile, _login: &str) -> AppResult<Vec<Confirmation>> {
    let client = session_client(mafile)?;
    let qs = query_params(mafile, "list")?;
    let url = format!("{BASE}/getlist?{qs}");
    let resp = client
        .get(&url)
        .header("Cookie", cookie_header(mafile))
        .header(
            "X-Requested-With",
            "com.valvesoftware.android.steam.community",
        )
        .header("Referer", "https://steamcommunity.com/mobileconf/conf")
        .send()
        .map_err(|e| AppError::Other(format!("getlist: {}", e.without_url())))?;
    decode_list(&response_body(resp, "getlist")?)
}

/// Respond to multiple confirmations in a single call.
pub fn respond(
    mafile: &MaFile,
    login: &str,
    ids: &[String],
    op: Op,
) -> AppResult<Vec<RespondResult>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    // We need one `nonce` per id — caller doesn't have it so we fetch-once.
    // This costs an extra roundtrip but keeps the command ergonomic (the UI
    // has already displayed nonces, but bundling them in the respond command
    // would leak implementation detail).
    let list_items = list(mafile, login)?;
    let mut pairs: Vec<(String, String)> = Vec::with_capacity(ids.len());
    for id in ids {
        let Some(row) = list_items.iter().find(|c| &c.id == id) else {
            return Err(AppError::NotFound(format!("CONF_NOT_FOUND: {id}")));
        };
        pairs.push((row.id.clone(), row.nonce.clone()));
    }

    let client = session_client(mafile)?;
    let qs = query_params(mafile, op.tag())?;
    let mut form = format!("op={}&{}", op.as_str(), qs);
    for (id, nonce) in &pairs {
        form.push_str(&format!(
            "&cid%5B%5D={}&ck%5B%5D={}",
            pct_encode(id),
            pct_encode(nonce)
        ));
    }

    let url = format!("{BASE}/multiajaxop");
    let resp = client
        .post(&url)
        .header("Cookie", cookie_header(mafile))
        .header(
            "X-Requested-With",
            "com.valvesoftware.android.steam.community",
        )
        .header("Referer", "https://steamcommunity.com/mobileconf/conf")
        .header(
            "Content-Type",
            "application/x-www-form-urlencoded; charset=UTF-8",
        )
        .body(form)
        .send()
        .map_err(|e| AppError::Other(format!("multiajaxop: {}", e.without_url())))?;
    decode_response(
        &response_body(resp, "multiajaxop")?,
        pairs.into_iter().map(|(id, _)| id),
    )
}

/// creator_id is a tradeofferid, NOT the sender's SteamID. Ask Steam for the
/// offer direction before allowing unattended confirmation of outgoing trades.
pub fn is_outgoing_trade(mafile: &MaFile, confirmation: &Confirmation) -> AppResult<bool> {
    if confirmation.kind != 2 {
        return Ok(false);
    }
    let session = mafile
        .session
        .as_ref()
        .ok_or_else(|| AppError::NotReady("CONF_NO_SESSION".into()))?;
    let resp = http::shared()
        .get("https://api.steampowered.com/IEconService/GetTradeOffer/v1/")
        .query(&[
            ("access_token", session.access_token.as_str()),
            ("tradeofferid", confirmation.creator_id.as_str()),
        ])
        .send()
        .map_err(|e| AppError::Other(format!("GetTradeOffer: {}", e.without_url())))?;
    // Economy API permissions can differ from mobileconf. A denied lookup must
    // skip automation without invalidating a working confirmation session.
    if !resp.status().is_success() {
        return Err(AppError::Other(format!(
            "GetTradeOffer HTTP {}",
            resp.status()
        )));
    }
    let body = response_body(resp, "GetTradeOffer")?;
    let value: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| AppError::Other(format!("GetTradeOffer JSON: {e}")))?;
    outgoing_offer(&value, &confirmation.creator_id)
}

fn outgoing_offer(value: &serde_json::Value, id: &str) -> AppResult<bool> {
    let offer = value.pointer("/response/offer").ok_or_else(|| {
        AppError::Other("GetTradeOffer: Steam did not return offer details".into())
    })?;
    Ok(
        offer.get("tradeofferid").and_then(|v| v.as_str()) == Some(id)
            && offer.get("is_our_offer").and_then(|v| v.as_bool()) == Some(true)
            && offer.get("trade_offer_state").and_then(|v| v.as_u64()) == Some(9),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_plus_and_slash() {
        let out = pct_encode("a+b/c=");
        assert_eq!(out, "a%2Bb%2Fc%3D");
    }

    #[test]
    fn op_tags() {
        assert_eq!(Op::Allow.as_str(), "allow");
        assert_eq!(Op::Allow.tag(), "accept");
        assert_eq!(Op::Reject.as_str(), "cancel");
        assert_eq!(Op::Reject.tag(), "reject");
    }

    #[test]
    fn reads_numeric_creator_without_losing_precision() {
        let rows = decode_list(r#"{"success":true,"conf":[{"id":"1","nonce":"2","creator_id":123456789012345678,"type":2}]}"#).unwrap();
        assert_eq!(rows[0].creator_id, "123456789012345678");
    }

    #[test]
    fn auth_errors_and_denials_are_not_empty_lists_or_successes() {
        assert!(super::super::session::is_expired(
            &decode_list(r#"{"needauth":true}"#).unwrap_err()
        ));
        assert!(super::super::session::is_expired(
            &decode_response(r#"{"needauth":true}"#, ["1".into()].into_iter()).unwrap_err()
        ));
        assert!(decode_list(r#"{"success":false,"detail":"Try again"}"#)
            .unwrap_err()
            .to_string()
            .contains("Try again"));
        assert!(decode_list(r#"{"success":true,"conf":[]}"#)
            .unwrap()
            .is_empty());
        let rows = decode_response(
            r#"{"success":false,"detail":"Denied"}"#,
            ["1".into()].into_iter(),
        )
        .unwrap();
        assert!(!rows[0].success);
        assert_eq!(rows[0].message, "Denied");
    }

    #[test]
    fn automatic_trades_require_verified_outgoing_offer() {
        let mut value = serde_json::json!({"response":{"offer":{"tradeofferid":"123", "is_our_offer":true, "trade_offer_state":9}}});
        assert!(outgoing_offer(&value, "123").unwrap());
        assert!(!outgoing_offer(&value, "456").unwrap());
        value["response"]["offer"]["is_our_offer"] = false.into();
        assert!(!outgoing_offer(&value, "123").unwrap());
        assert!(outgoing_offer(&serde_json::json!({"response":{}}), "123").is_err());
    }

    fn display_fixture() -> serde_json::Value {
        serde_json::json!({"response": {
            "offer": {"tradeofferid":"123456789012345678", "accountid_other":42, "trade_offer_state":9,
                "items_to_give":[{"appid":730,"assetid":"987654321012345678","classid":"10","instanceid":"0","amount":"2"}],
                "items_to_receive":[{"appid":570,"assetid":"2","classid":"10","instanceid":"1","amount":"1"}]},
            "descriptions":[
                {"appid":570,"classid":"10","instanceid":"0","name":"Wrong app/instance","icon_url":"wrong"},
                {"appid":730,"classid":"10","instanceid":"0","name":"Demo case","icon_url":"case-hash"},
                {"appid":570,"classid":"10","instanceid":"1","market_hash_name":"Demo item","icon_url":"item-hash"}
            ]
        }})
    }

    #[test]
    fn details_keep_both_sides_quantities_ids_and_match_complete_description_key() {
        let result =
            decode_trade_details(&display_fixture().to_string(), "123456789012345678").unwrap();
        assert_eq!(result.partner_steam_id, "76561197960265770");
        assert_eq!(result.giving[0].asset_id, "987654321012345678");
        assert_eq!(result.giving[0].amount, "2");
        assert_eq!(result.giving[0].name, "Demo case");
        assert!(result.giving[0].icon.ends_with("/case-hash/128fx128f"));
        assert_eq!(result.receiving[0].name, "Demo item");
        assert!(result.receiving[0].icon.ends_with("/item-hash/128fx128f"));
    }

    #[test]
    fn missing_descriptions_preserve_assets_and_one_empty_side_is_valid() {
        let mut value = display_fixture();
        value["response"]["descriptions"] = serde_json::json!([]);
        value["response"]["offer"]["items_to_receive"] = serde_json::json!([]);
        let result = decode_trade_details(&value.to_string(), "123456789012345678").unwrap();
        assert_eq!(result.giving.len(), 1);
        assert!(result.giving[0].icon.is_empty());
        assert!(result.giving[0].name.is_empty());
        assert!(result.receiving.is_empty());
    }

    #[test]
    fn details_reject_wrong_offer_inactive_and_incomplete_responses() {
        let mut value = display_fixture();
        assert!(decode_trade_details(&value.to_string(), "different").is_err());
        value["response"]["offer"]["trade_offer_state"] = 3.into();
        assert!(decode_trade_details(&value.to_string(), "123456789012345678").is_err());
        value["response"]["offer"]["trade_offer_state"] = 9.into();
        value["response"]["offer"]["items_to_give"] = serde_json::json!([]);
        value["response"]["offer"]["items_to_receive"] = serde_json::json!([]);
        assert!(decode_trade_details(&value.to_string(), "123456789012345678").is_err());
        assert!(decode_trade_details(r#"{"response":{}}"#, "123").is_err());
    }

    #[test]
    fn confirmation_list_preserves_icon_and_missing_icon_is_supported() {
        let rows = decode_list(r#"{"success":true,"conf":[{"id":"1","nonce":"2","type":3,"icon":"https://community.steamstatic.com/economy/image/demo"},{"id":"3","nonce":"4","type":2}]}"#).unwrap();
        assert_eq!(
            rows[0].icon,
            "https://community.steamstatic.com/economy/image/demo"
        );
        assert!(rows[1].icon.is_empty());
    }
}
