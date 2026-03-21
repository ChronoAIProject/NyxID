//! Telegram Login Widget HMAC-SHA256 verification.
//!
//! Implements the server-side verification of the Telegram Login Widget
//! callback data, as described at <https://core.telegram.org/widgets/login>.

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::errors::{AppError, AppResult};

/// Maximum allowed age (in seconds) of a Telegram login callback.
const MAX_AUTH_AGE_SECS: i64 = 300; // 5 minutes

/// Data received from the Telegram Login Widget callback.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
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

/// Verify Telegram Login Widget data using HMAC-SHA256.
///
/// 1. Compute `secret_key = SHA256(bot_token)`.
/// 2. Build a data-check string by sorting all fields (except `hash`)
///    alphabetically and joining as `key=value\n`.
/// 3. Compute `HMAC-SHA256(secret_key, data_check_string)`.
/// 4. Compare with the received `hash` using constant-time comparison.
/// 5. Validate `auth_date` is within the allowed time window.
pub fn verify_telegram_login(bot_token: &str, data: &TelegramLoginData) -> AppResult<()> {
    // Validate auth_date freshness
    let now = chrono::Utc::now().timestamp();
    if now - data.auth_date > MAX_AUTH_AGE_SECS {
        return Err(AppError::Unauthorized(
            "Telegram login data has expired".to_string(),
        ));
    }
    if data.auth_date > now + 60 {
        return Err(AppError::Unauthorized(
            "Telegram login data has invalid auth_date".to_string(),
        ));
    }

    // Build the data-check string: sort fields alphabetically, join as key=value\n
    let mut fields: Vec<(String, String)> = Vec::new();
    fields.push(("auth_date".to_string(), data.auth_date.to_string()));
    fields.push(("first_name".to_string(), data.first_name.clone()));
    fields.push(("id".to_string(), data.id.to_string()));
    if let Some(ref last_name) = data.last_name {
        fields.push(("last_name".to_string(), last_name.clone()));
    }
    if let Some(ref photo_url) = data.photo_url {
        fields.push(("photo_url".to_string(), photo_url.clone()));
    }
    if let Some(ref username) = data.username {
        fields.push(("username".to_string(), username.clone()));
    }
    fields.sort_by(|a, b| a.0.cmp(&b.0));

    let data_check_string: String = fields
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("\n");

    // secret_key = SHA256(bot_token)
    let secret_key = Sha256::digest(bot_token.as_bytes());

    // HMAC-SHA256(secret_key, data_check_string)
    let mut mac =
        Hmac::<Sha256>::new_from_slice(&secret_key).expect("HMAC can accept any key size");
    mac.update(data_check_string.as_bytes());
    let computed = mac.finalize().into_bytes();

    // Decode the received hex hash
    let received = hex::decode(&data.hash).map_err(|_| {
        AppError::ValidationError("Invalid hash format in Telegram login data".to_string())
    })?;

    // Constant-time comparison
    if computed.as_slice().ct_eq(&received).into() {
        Ok(())
    } else {
        Err(AppError::Unauthorized(
            "Telegram login verification failed".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_valid_telegram_login() {
        // Use a known bot token and compute valid test data
        let bot_token = "123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11";

        let data = TelegramLoginData {
            id: 12345678,
            first_name: "John".to_string(),
            last_name: Some("Doe".to_string()),
            username: Some("johndoe".to_string()),
            photo_url: None,
            auth_date: chrono::Utc::now().timestamp(),
            hash: String::new(), // Will compute below
        };

        // Compute the expected hash
        let mut fields: Vec<(String, String)> = vec![
            ("auth_date".to_string(), data.auth_date.to_string()),
            ("first_name".to_string(), data.first_name.clone()),
            ("id".to_string(), data.id.to_string()),
            ("last_name".to_string(), "Doe".to_string()),
            ("username".to_string(), "johndoe".to_string()),
        ];
        fields.sort_by(|a, b| a.0.cmp(&b.0));
        let check_str: String = fields
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("\n");

        let secret = sha2::Sha256::digest(bot_token.as_bytes());
        let mut mac =
            Hmac::<Sha256>::new_from_slice(&secret).expect("HMAC can accept any key size");
        mac.update(check_str.as_bytes());
        let hash = hex::encode(mac.finalize().into_bytes());

        let valid_data = TelegramLoginData { hash, ..data };

        assert!(verify_telegram_login(bot_token, &valid_data).is_ok());
    }

    #[test]
    fn reject_invalid_hash() {
        let bot_token = "123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11";
        let data = TelegramLoginData {
            id: 12345678,
            first_name: "John".to_string(),
            last_name: None,
            username: None,
            photo_url: None,
            auth_date: chrono::Utc::now().timestamp(),
            hash: "0000000000000000000000000000000000000000000000000000000000000000"
                .to_string(),
        };

        let result = verify_telegram_login(bot_token, &data);
        assert!(result.is_err());
    }

    #[test]
    fn reject_expired_auth_date() {
        let bot_token = "123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11";
        let data = TelegramLoginData {
            id: 12345678,
            first_name: "John".to_string(),
            last_name: None,
            username: None,
            photo_url: None,
            auth_date: chrono::Utc::now().timestamp() - 600, // 10 minutes ago
            hash: "deadbeef".to_string(),
        };

        let result = verify_telegram_login(bot_token, &data);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("expired"),
            "Should mention expiry"
        );
    }
}
