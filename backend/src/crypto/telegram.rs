use chrono::{DateTime, Duration, TimeZone, Utc};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TelegramLoginData {
    pub id: i64,
    pub first_name: String,
    #[serde(default)]
    pub last_name: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub photo_url: Option<String>,
    pub auth_date: i64,
    pub hash: String,
}

pub fn verify_telegram_login(bot_token: &str, data: &TelegramLoginData) -> bool {
    verify_telegram_login_at(bot_token, data, Utc::now())
}

fn verify_telegram_login_at(bot_token: &str, data: &TelegramLoginData, now: DateTime<Utc>) -> bool {
    let Some(auth_date) = Utc.timestamp_opt(data.auth_date, 0).single() else {
        return false;
    };

    let age = now.signed_duration_since(auth_date);
    if age < Duration::zero() || age > Duration::minutes(5) {
        return false;
    }

    let provided_hash = match hex::decode(&data.hash) {
        Ok(hash) => hash,
        Err(_) => return false,
    };

    let secret_key = Sha256::digest(bot_token.as_bytes());
    let mut mac = match HmacSha256::new_from_slice(secret_key.as_slice()) {
        Ok(mac) => mac,
        Err(_) => return false,
    };
    mac.update(build_data_check_string(data).as_bytes());

    mac.verify_slice(&provided_hash).is_ok()
}

fn build_data_check_string(data: &TelegramLoginData) -> String {
    let mut fields = vec![
        ("auth_date", data.auth_date.to_string()),
        ("first_name", data.first_name.clone()),
        ("id", data.id.to_string()),
    ];

    if let Some(value) = non_empty(&data.last_name) {
        fields.push(("last_name", value.to_string()));
    }
    if let Some(value) = non_empty(&data.photo_url) {
        fields.push(("photo_url", value.to_string()));
    }
    if let Some(value) = non_empty(&data.username) {
        fields.push(("username", value.to_string()));
    }

    fields.sort_by(|left, right| left.0.cmp(right.0));

    fields
        .into_iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn non_empty(value: &Option<String>) -> Option<&str> {
    value.as_deref().filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signed_login_data(bot_token: &str) -> TelegramLoginData {
        let mut data = TelegramLoginData {
            id: 123_456_789,
            first_name: "Nyx".to_string(),
            last_name: Some("Bot".to_string()),
            username: Some("nyxid_bot".to_string()),
            photo_url: Some("https://example.com/avatar.png".to_string()),
            auth_date: Utc::now().timestamp(),
            hash: String::new(),
        };

        let secret_key = Sha256::digest(bot_token.as_bytes());
        let mut mac = HmacSha256::new_from_slice(secret_key.as_slice()).expect("valid hmac key");
        mac.update(build_data_check_string(&data).as_bytes());
        data.hash = hex::encode(mac.finalize().into_bytes());
        data
    }

    #[test]
    fn verifies_valid_login_payload() {
        let bot_token = "123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11";
        let data = signed_login_data(bot_token);

        assert!(verify_telegram_login(bot_token, &data));
    }

    #[test]
    fn rejects_stale_login_payload() {
        let bot_token = "123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11";
        let mut data = signed_login_data(bot_token);
        data.auth_date = (Utc::now() - Duration::minutes(6)).timestamp();

        let secret_key = Sha256::digest(bot_token.as_bytes());
        let mut mac = HmacSha256::new_from_slice(secret_key.as_slice()).expect("valid hmac key");
        mac.update(build_data_check_string(&data).as_bytes());
        data.hash = hex::encode(mac.finalize().into_bytes());

        assert!(!verify_telegram_login(bot_token, &data));
    }

    #[test]
    fn rejects_tampered_login_payload() {
        let bot_token = "123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11";
        let mut data = signed_login_data(bot_token);
        data.username = Some("intruder".to_string());

        assert!(!verify_telegram_login(bot_token, &data));
    }

    #[test]
    fn data_check_string_sorts_optional_fields() {
        let data = TelegramLoginData {
            id: 42,
            first_name: "Nyx".to_string(),
            last_name: Some("ID".to_string()),
            username: Some("nyx".to_string()),
            photo_url: None,
            auth_date: 1_700_000_000,
            hash: "deadbeef".to_string(),
        };

        assert_eq!(
            build_data_check_string(&data),
            "auth_date=1700000000\nfirst_name=Nyx\nid=42\nlast_name=ID\nusername=nyx"
        );
    }
}
