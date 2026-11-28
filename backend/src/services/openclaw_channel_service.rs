use hmac::{Hmac, Mac};
use mongodb::bson::doc;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::errors::{AppError, AppResult};
use crate::models::user_provider_token::{
    COLLECTION_NAME as USER_PROVIDER_TOKENS, UserProviderToken,
};

type HmacSha256 = Hmac<Sha256>;

/// Inbound message from OpenClaw channel webhook.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct OpenClawChannelMessage {
    /// OpenClaw channel identifier (e.g., "whatsapp", "telegram", "discord")
    pub channel: String,
    /// Channel-specific user identifier
    pub channel_user_id: String,
    /// The agent ID that handled this message in OpenClaw
    #[serde(default)]
    pub agent_id: Option<String>,
    /// Session key in OpenClaw
    #[serde(default)]
    pub session_key: Option<String>,
    /// The message content
    pub message: String,
    /// Message direction: "inbound" (user->agent) or "outbound" (agent->user)
    #[serde(default = "default_direction")]
    pub direction: String,
    /// Optional metadata from the channel
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

fn default_direction() -> String {
    "inbound".to_string()
}

/// Mapping record between OpenClaw channel users and NyxID users.
/// Each mapping has its own webhook secret so each user's OpenClaw
/// instance signs requests independently.
/// Stored in the `openclaw_channel_mappings` collection.
#[derive(Clone, Serialize, Deserialize)]
pub struct OpenClawChannelMapping {
    #[serde(rename = "_id")]
    pub id: String,
    /// The OpenClaw channel (e.g., "whatsapp")
    pub channel: String,
    /// Channel-specific user identifier
    pub channel_user_id: String,
    /// NyxID user ID
    pub nyxid_user_id: String,
    /// SHA-256 hash of the per-mapping webhook secret.
    /// The raw secret is returned once at creation time.
    pub webhook_secret_hash: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

pub const MAPPINGS_COLLECTION: &str = "openclaw_channel_mappings";

pub(crate) async fn ensure_mapping_index(db: &mongodb::Database) -> mongodb::error::Result<()> {
    // MongoDB 8 enforces uniqueness on new entries while allowing historical
    // duplicate tuples to keep their existing IDs and webhook secrets.
    db.run_command(doc! {
        "createIndexes": MAPPINGS_COLLECTION,
        "indexes": [{
            "name": "openclaw_channel_sender_claim",
            "key": { "channel": 1, "channel_user_id": 1 },
            "prepareUnique": true,
        }],
    })
    .await?;
    Ok(())
}

/// Generate a random 32-byte webhook secret, returned as a hex string.
pub fn generate_webhook_secret() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Hash a webhook secret with SHA-256 for storage.
pub fn hash_secret(secret: &str) -> String {
    use sha2::Digest;
    hex::encode(Sha256::digest(secret.as_bytes()))
}

/// Verify the HMAC-SHA256 signature on an OpenClaw webhook request.
pub fn verify_webhook_signature(secret: &str, body: &[u8], signature: &str) -> AppResult<()> {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|_| AppError::Internal("HMAC key error".to_string()))?;
    mac.update(body);

    let expected = hex::encode(mac.finalize().into_bytes());

    if !constant_time_eq(expected.as_bytes(), signature.as_bytes()) {
        return Err(AppError::Unauthorized(
            "Invalid webhook signature".to_string(),
        ));
    }

    Ok(())
}

/// Verify a webhook signature against a mapping's stored secret hash.
///
/// Since we only store the hash of the secret, we can't reconstruct the HMAC
/// key directly. Instead, the webhook must include the mapping ID so we can
/// look up which mapping to verify against. The actual HMAC verification
/// requires the raw secret, which the user configures in their OpenClaw
/// instance. We verify by:
/// 1. Looking up the mapping by channel + channel_user_id
/// 2. The caller (handler) compares the provided signature using the raw
///    secret from the X-OpenClaw-Webhook-Secret header (sent by OpenClaw).
///    We verify the secret matches the stored hash, then verify the HMAC.
pub async fn verify_webhook_for_mapping(
    db: &mongodb::Database,
    channel: &str,
    channel_user_id: &str,
    webhook_secret: &str,
    body: &[u8],
    signature: &str,
) -> AppResult<OpenClawChannelMapping> {
    let mapping = db
        .collection::<OpenClawChannelMapping>(MAPPINGS_COLLECTION)
        .find_one(doc! {
            "channel": channel,
            "channel_user_id": channel_user_id,
        })
        .await?
        .ok_or_else(|| {
            AppError::Unauthorized("No mapping found for this channel user".to_string())
        })?;

    // Verify the provided secret matches the stored hash
    let provided_hash = hash_secret(webhook_secret);
    if !constant_time_eq(
        provided_hash.as_bytes(),
        mapping.webhook_secret_hash.as_bytes(),
    ) {
        return Err(AppError::Unauthorized("Invalid webhook secret".to_string()));
    }

    // Verify HMAC signature on the body using the raw secret
    verify_webhook_signature(webhook_secret, body, signature)?;

    Ok(mapping)
}

/// Create or update a channel-to-user mapping.
/// Returns the raw webhook secret (only available at creation/rotation time).
pub async fn upsert_mapping(
    db: &mongodb::Database,
    channel: &str,
    channel_user_id: &str,
    nyxid_user_id: &str,
) -> AppResult<String> {
    let now = chrono::Utc::now();
    let col = db.collection::<OpenClawChannelMapping>(MAPPINGS_COLLECTION);
    let secret = generate_webhook_secret();
    let secret_hash = hash_secret(&secret);

    col.update_one(
        doc! { "channel": channel, "channel_user_id": channel_user_id, "nyxid_user_id": nyxid_user_id },
        doc! {
            "$set": {
                "webhook_secret_hash": &secret_hash,
                "updated_at": bson::DateTime::from_chrono(now),
            },
            "$setOnInsert": {
                "_id": uuid::Uuid::new_v4().to_string(),
                "created_at": bson::DateTime::from_chrono(now),
            },
        },
    ).upsert(true).await.map_err(|error| {
        if crate::services::service_pool_service::is_duplicate_key(&error) {
            AppError::Conflict("Channel user mapping is already assigned".into())
        } else {
            AppError::from(error)
        }
    })?;

    Ok(secret)
}

/// Get provider slugs that a user has active tokens for.
pub async fn get_user_provider_slugs(
    db: &mongodb::Database,
    user_id: &str,
) -> AppResult<Vec<String>> {
    use crate::models::provider_config::{COLLECTION_NAME as PROVIDER_CONFIGS, ProviderConfig};
    use futures::TryStreamExt;

    let tokens: Vec<UserProviderToken> = db
        .collection::<UserProviderToken>(USER_PROVIDER_TOKENS)
        .find(doc! { "user_id": user_id, "status": "active" })
        .await?
        .try_collect()
        .await?;

    let provider_ids: Vec<&str> = tokens
        .iter()
        .map(|t| t.provider_config_id.as_str())
        .collect();

    if provider_ids.is_empty() {
        return Ok(vec![]);
    }

    let providers: Vec<ProviderConfig> = db
        .collection::<ProviderConfig>(PROVIDER_CONFIGS)
        .find(doc! { "_id": { "$in": &provider_ids } })
        .await?
        .try_collect()
        .await?;

    Ok(providers.into_iter().map(|p| p.slug).collect())
}

/// Constant-time comparison for HMAC signatures.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

impl std::fmt::Debug for OpenClawChannelMapping {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenClawChannelMapping")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mapping_claim_preserves_owner_and_rotates_legacy_secret() {
        let db = crate::test_utils::connect_test_database("openclaw_owner")
            .await
            .unwrap();
        let col = db.collection::<bson::Document>(MAPPINGS_COLLECTION);
        let now = bson::DateTime::now();
        let id = uuid::Uuid::new_v4().to_string();
        col.insert_one(doc! { "_id": &id, "channel": "chat", "channel_user_id": "sender",
            "nyxid_user_id": "owner", "webhook_secret_hash": hash_secret("initial"), "created_at": now,
            "updated_at": now }).await.unwrap();
        ensure_mapping_index(&db).await.unwrap();
        let original = col.find_one(doc! { "_id": &id }).await.unwrap().unwrap();
        assert!(matches!(
            upsert_mapping(&db, "chat", "sender", "second").await,
            Err(AppError::Conflict(_))
        ));
        assert_eq!(
            col.find_one(doc! { "_id": &id }).await.unwrap().unwrap(),
            original
        );
        let rotated = upsert_mapping(&db, "chat", "sender", "owner")
            .await
            .unwrap();
        let row = col.find_one(doc! { "_id": id }).await.unwrap().unwrap();
        assert_eq!(row.get_str("nyxid_user_id").unwrap(), "owner");
        assert_eq!(
            row.get_str("webhook_secret_hash").unwrap(),
            hash_secret(&rotated)
        );
    }

    #[tokio::test]
    async fn concurrent_mapping_claims_select_one_owner() {
        let db = crate::test_utils::connect_test_database("openclaw_claim")
            .await
            .unwrap();
        ensure_mapping_index(&db).await.unwrap();
        let (a, b) = tokio::join!(
            upsert_mapping(&db, "chat", "sender", "first"),
            upsert_mapping(&db, "chat", "sender", "second"),
        );
        let (owner, secret) = match (a, b) {
            (Ok(secret), Err(AppError::Conflict(_))) => ("first", secret),
            (Err(AppError::Conflict(_)), Ok(secret)) => ("second", secret),
            _ => panic!("expected one successful claim and one conflict"),
        };
        let col = db.collection::<OpenClawChannelMapping>(MAPPINGS_COLLECTION);
        assert_eq!(col.count_documents(doc! {}).await.unwrap(), 1);
        let row = col.find_one(doc! {}).await.unwrap().unwrap();
        assert_eq!(row.nyxid_user_id, owner);
        assert_eq!(row.webhook_secret_hash, hash_secret(&secret));
    }

    #[test]
    fn verify_valid_signature() {
        let secret = "test-secret";
        let body = b"hello world";
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        let signature = hex::encode(mac.finalize().into_bytes());

        assert!(verify_webhook_signature(secret, body, &signature).is_ok());
    }

    #[test]
    fn reject_invalid_signature() {
        let secret = "test-secret";
        let body = b"hello world";

        assert!(verify_webhook_signature(secret, body, "bad-signature").is_err());
    }

    #[test]
    fn reject_tampered_body() {
        let secret = "test-secret";
        let body = b"hello world";
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        let signature = hex::encode(mac.finalize().into_bytes());

        assert!(verify_webhook_signature(secret, b"tampered", &signature).is_err());
    }

    #[test]
    fn generate_secret_is_64_hex_chars() {
        let secret = generate_webhook_secret();
        assert_eq!(secret.len(), 64);
        assert!(secret.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn hash_secret_is_deterministic() {
        let secret = "my-secret";
        assert_eq!(hash_secret(secret), hash_secret(secret));
    }

    #[test]
    fn hash_secret_differs_for_different_inputs() {
        assert_ne!(hash_secret("secret-a"), hash_secret("secret-b"));
    }
}
