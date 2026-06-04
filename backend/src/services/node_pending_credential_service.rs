use std::fmt;

use chrono::{DateTime, Duration, Utc};
use futures::TryStreamExt;
use mongodb::bson::{self, Bson, doc};
use mongodb::options::ReturnDocument;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::errors::{AppError, AppResult};
use crate::models::node_pending_credential::{
    COLLECTION_NAME as NODE_PENDING_CREDENTIALS, CryptoBundle, InjectionMethod,
    NodePendingCredential, RemoteCryptoState,
};
use crate::services::{node_service, url_validation};

pub const MAX_CIPHERTEXT_SIZE: usize = 16 * 1024;
pub const OFFLINE_CIPHERTEXT_QUEUE_TTL_SECS: i64 = 15 * 60;
pub const MAX_OFFLINE_CIPHERTEXT_QUEUE_PER_NODE: u64 = 5;

pub struct CreatePendingCredentialInput {
    pub service_slug: String,
    pub injection_method: InjectionMethod,
    pub field_name: String,
    pub target_url: Option<String>,
    pub label: Option<String>,
    pub ttl_secs: i64,
    pub remote_crypto: bool,
}

#[derive(Clone, PartialEq, Eq)]
pub struct StorePendingCiphertextInput {
    pub admin_pubkey: Zeroizing<String>,
    pub nonce: Zeroizing<String>,
    pub ciphertext: Zeroizing<Vec<u8>>,
}

impl StorePendingCiphertextInput {
    pub fn new(admin_pubkey: String, nonce: String, ciphertext: Vec<u8>) -> Self {
        Self {
            admin_pubkey: Zeroizing::new(admin_pubkey),
            nonce: Zeroizing::new(nonce),
            ciphertext: Zeroizing::new(ciphertext),
        }
    }
}

impl fmt::Debug for StorePendingCiphertextInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StorePendingCiphertextInput")
            .field("admin_pubkey", &"[REDACTED]")
            .field("nonce", &"[REDACTED]")
            .field(
                "ciphertext",
                &format!("[REDACTED; {} bytes]", self.ciphertext.len()),
            )
            .finish()
    }
}

#[derive(Clone)]
pub enum StorePendingCiphertextOutcome {
    StoredForOnlineNode(NodePendingCredential),
    QueuedOffline(NodePendingCredential),
    QueueFull,
}

impl fmt::Debug for StorePendingCiphertextOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StoredForOnlineNode(pending) => f
                .debug_struct("StoredForOnlineNode")
                .field("pending_id", &pending.id)
                .field("remote_state", &pending.remote_state)
                .finish(),
            Self::QueuedOffline(pending) => f
                .debug_struct("QueuedOffline")
                .field("pending_id", &pending.id)
                .field("remote_state", &pending.remote_state)
                .finish(),
            Self::QueueFull => f.write_str("QueueFull"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingCredentialDecryptOutcome {
    Ok,
    Error,
}

pub async fn create_pending_credential(
    db: &mongodb::Database,
    actor_user_id: &str,
    node_id: &str,
    input: CreatePendingCredentialInput,
) -> AppResult<NodePendingCredential> {
    validate_service_slug(&input.service_slug)?;
    validate_field_name(&input.field_name, &input.injection_method)?;
    let target_url = clean_optional_string(input.target_url);
    if let Some(url) = target_url.as_deref() {
        url_validation::validate_advisory_http_url(url, "target_url", url_validation::MAX_URL_LEN)?;
    }
    let label = clean_optional_string(input.label);
    if let Some(label) = label.as_deref()
        && label.len() > 128
    {
        return Err(AppError::ValidationError(
            "label must be 128 characters or fewer".to_string(),
        ));
    }

    let node = node_service::ensure_node_writable_by_actor(db, actor_user_id, node_id).await?;
    let existing = db
        .collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .find_one(doc! {
            "node_id": node_id,
            "service_slug": &input.service_slug,
            "is_active": true,
        })
        .await?;
    if let Some(existing) = existing {
        return Err(AppError::Conflict(format!(
            "A pending credential already exists for service '{}' on this node (id: {})",
            input.service_slug, existing.id
        )));
    }

    let now = Utc::now();
    let expires_at = now + Duration::seconds(input.ttl_secs.max(1));
    let crypto = input.remote_crypto.then(|| CryptoBundle {
        version: "v1".to_string(),
        node_pubkey: String::new(),
        admin_pubkey: None,
        nonce: None,
        ciphertext: None,
    });
    let pending = NodePendingCredential {
        id: Uuid::new_v4().to_string(),
        node_id: node_id.to_string(),
        service_slug: input.service_slug,
        injection_method: input.injection_method,
        field_name: input.field_name,
        target_url,
        label,
        created_by_user_id: actor_user_id.to_string(),
        owner_user_id: node.user_id,
        created_at: now,
        expires_at,
        consumed_at: None,
        declined_at: None,
        crypto,
        remote_state: None,
        ciphertext_queued_at: None,
        ciphertext_expires_at: None,
        is_active: true,
    };

    db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .insert_one(&pending)
        .await?;

    Ok(pending)
}

pub async fn list_pending_credentials_for_admin(
    db: &mongodb::Database,
    actor_user_id: &str,
    node_id: &str,
    include_history: bool,
) -> AppResult<Vec<NodePendingCredential>> {
    node_service::ensure_node_writable_by_actor(db, actor_user_id, node_id).await?;

    let mut filter = doc! { "node_id": node_id };
    if !include_history {
        filter.insert("is_active", true);
        filter.insert(
            "expires_at",
            doc! { "$gt": bson::DateTime::from_chrono(Utc::now()) },
        );
    }

    db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .find(filter)
        .sort(doc! { "created_at": -1 })
        .await?
        .try_collect()
        .await
        .map_err(AppError::from)
}

pub async fn list_pending_credentials_for_node(
    db: &mongodb::Database,
    node_id: &str,
) -> AppResult<Vec<NodePendingCredential>> {
    db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .find(doc! {
            "node_id": node_id,
            "is_active": true,
            "expires_at": { "$gt": bson::DateTime::from_chrono(Utc::now()) },
        })
        .sort(doc! { "created_at": -1 })
        .await?
        .try_collect()
        .await
        .map_err(AppError::from)
}

pub async fn record_pending_credential_pubkey(
    db: &mongodb::Database,
    node_id: &str,
    pending_id: &str,
    version: &str,
    node_pubkey: &str,
) -> AppResult<NodePendingCredential> {
    if version != "v1" {
        return Err(AppError::PendingCredentialVersionUnsupported(
            version.to_string(),
        ));
    }
    let now = Utc::now();
    let updated = db
        .collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .find_one_and_update(
            doc! {
                "_id": pending_id,
                "node_id": node_id,
                "is_active": true,
                "expires_at": { "$gt": bson::DateTime::from_chrono(now) },
                "crypto.version": "v1",
                "crypto.node_pubkey": "",
            },
            doc! {
                "$set": {
                    "crypto.node_pubkey": node_pubkey,
                    "remote_state": remote_state_bson(RemoteCryptoState::PubkeyPosted)?,
                },
            },
        )
        .return_document(ReturnDocument::After)
        .await?;

    match updated {
        Some(updated) => Ok(updated),
        None => {
            let current =
                load_active_unexpired_pending_credential(db, node_id, pending_id, now).await?;
            match current.crypto.as_ref() {
                Some(crypto) if crypto.version == "v1" && !crypto.node_pubkey.is_empty() => {
                    Ok(current)
                }
                _ => Err(AppError::NotFound(
                    "Pending credential not found".to_string(),
                )),
            }
        }
    }
}

pub async fn get_pending_credential_for_admin(
    db: &mongodb::Database,
    actor_user_id: &str,
    node_id: &str,
    pending_id: &str,
) -> AppResult<NodePendingCredential> {
    node_service::ensure_node_writable_by_actor(db, actor_user_id, node_id).await?;
    load_active_unexpired_pending_credential(db, node_id, pending_id, Utc::now()).await
}

pub async fn store_pending_ciphertext_first_writer_wins(
    db: &mongodb::Database,
    actor_user_id: &str,
    node_id: &str,
    pending_id: &str,
    input: StorePendingCiphertextInput,
    node_connected: bool,
    now: DateTime<Utc>,
) -> AppResult<StorePendingCiphertextOutcome> {
    if input.ciphertext.len() > MAX_CIPHERTEXT_SIZE {
        return Err(AppError::PendingCredentialCiphertextTooLarge(
            input.ciphertext.len(),
        ));
    }

    node_service::ensure_node_writable_by_actor(db, actor_user_id, node_id).await?;
    let pending = load_active_unexpired_pending_credential(db, node_id, pending_id, now).await?;
    if pending_pubkey_missing(&pending) {
        return Err(AppError::PendingCredentialPubkeyAwaiting(
            pending_id.to_string(),
        ));
    }
    if has_ciphertext(&pending) {
        return stored_ciphertext_outcome(pending);
    }

    let state = if node_connected {
        RemoteCryptoState::CiphertextReceived
    } else {
        if active_unexpired_queued_ciphertext_count(db, node_id, now).await?
            >= MAX_OFFLINE_CIPHERTEXT_QUEUE_PER_NODE
        {
            return Ok(StorePendingCiphertextOutcome::QueueFull);
        }
        RemoteCryptoState::CiphertextQueued
    };

    let now_bson = bson::DateTime::from_chrono(now);
    let mut set_doc = doc! {
        "crypto.admin_pubkey": input.admin_pubkey.as_str(),
        "crypto.nonce": input.nonce.as_str(),
        "crypto.ciphertext": Bson::Binary(bson::Binary {
            subtype: bson::spec::BinarySubtype::Generic,
            bytes: input.ciphertext.as_slice().to_vec(),
        }),
        "remote_state": remote_state_bson(state.clone())?,
    };
    let mut unset_doc = doc! {};
    if node_connected {
        unset_doc.insert("ciphertext_queued_at", "");
        unset_doc.insert("ciphertext_expires_at", "");
    } else {
        set_doc.insert("ciphertext_queued_at", now_bson);
        set_doc.insert(
            "ciphertext_expires_at",
            bson::DateTime::from_chrono(now + Duration::seconds(OFFLINE_CIPHERTEXT_QUEUE_TTL_SECS)),
        );
    }

    let mut update_doc = doc! { "$set": set_doc };
    if !unset_doc.is_empty() {
        update_doc.insert("$unset", unset_doc);
    }

    let updated = db
        .collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .find_one_and_update(
            doc! {
                "_id": pending_id,
                "node_id": node_id,
                "is_active": true,
                "expires_at": { "$gt": bson::DateTime::from_chrono(now) },
                "crypto.node_pubkey": { "$type": "string" },
                "$or": [
                    { "crypto.ciphertext": { "$exists": false } },
                    { "crypto.ciphertext": Bson::Null },
                ],
            },
            update_doc,
        )
        .return_document(ReturnDocument::After)
        .await?;

    match updated {
        Some(updated) if node_connected => {
            Ok(StorePendingCiphertextOutcome::StoredForOnlineNode(updated))
        }
        Some(updated) => Ok(StorePendingCiphertextOutcome::QueuedOffline(updated)),
        None => {
            let current =
                load_active_unexpired_pending_credential(db, node_id, pending_id, now).await?;
            if pending_pubkey_missing(&current) {
                Err(AppError::PendingCredentialPubkeyAwaiting(
                    pending_id.to_string(),
                ))
            } else if has_ciphertext(&current) {
                stored_ciphertext_outcome(current)
            } else {
                Err(AppError::PendingCredentialPubkeyAwaiting(
                    pending_id.to_string(),
                ))
            }
        }
    }
}

#[cfg(test)]
pub async fn expire_queued_ciphertexts(
    db: &mongodb::Database,
    now: DateTime<Utc>,
) -> AppResult<u64> {
    let result = db
        .collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .update_many(
            doc! {
                "is_active": true,
                "remote_state": "ciphertext_queued",
                "ciphertext_expires_at": { "$lte": bson::DateTime::from_chrono(now) },
            },
            doc! {
                "$set": {
                    "remote_state": "expired",
                    "is_active": false,
                },
                "$unset": {
                    "crypto.admin_pubkey": "",
                    "crypto.nonce": "",
                    "crypto.ciphertext": "",
                    "ciphertext_queued_at": "",
                    "ciphertext_expires_at": "",
                },
            },
        )
        .await?;

    Ok(result.modified_count)
}

pub async fn mark_pending_ciphertext_queued_after_send_failure(
    db: &mongodb::Database,
    node_id: &str,
    pending_id: &str,
    now: DateTime<Utc>,
) -> AppResult<NodePendingCredential> {
    db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .find_one_and_update(
            doc! {
                "_id": pending_id,
                "node_id": node_id,
                "is_active": true,
                "expires_at": { "$gt": bson::DateTime::from_chrono(now) },
                "crypto.admin_pubkey": { "$type": "string" },
                "crypto.nonce": { "$type": "string" },
                "crypto.ciphertext": { "$exists": true },
            },
            doc! {
                "$set": {
                    "remote_state": remote_state_bson(RemoteCryptoState::CiphertextQueued)?,
                    "ciphertext_queued_at": bson::DateTime::from_chrono(now),
                    "ciphertext_expires_at": bson::DateTime::from_chrono(now + Duration::seconds(OFFLINE_CIPHERTEXT_QUEUE_TTL_SECS)),
                },
            },
        )
        .return_document(ReturnDocument::After)
        .await?
        .ok_or_else(|| AppError::NotFound("Pending credential not found".to_string()))
}

pub async fn mark_queued_ciphertext_sent(
    db: &mongodb::Database,
    node_id: &str,
    pending_id: &str,
    now: DateTime<Utc>,
) -> AppResult<NodePendingCredential> {
    db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .find_one_and_update(
            doc! {
                "_id": pending_id,
                "node_id": node_id,
                "is_active": true,
                "remote_state": "ciphertext_queued",
                "expires_at": { "$gt": bson::DateTime::from_chrono(now) },
                "ciphertext_expires_at": { "$gt": bson::DateTime::from_chrono(now) },
                "crypto.admin_pubkey": { "$type": "string" },
                "crypto.nonce": { "$type": "string" },
                "crypto.ciphertext": { "$exists": true },
            },
            doc! {
                "$set": {
                    "remote_state": remote_state_bson(RemoteCryptoState::CiphertextReceived)?,
                },
                "$unset": {
                    "ciphertext_queued_at": "",
                    "ciphertext_expires_at": "",
                },
            },
        )
        .return_document(ReturnDocument::After)
        .await?
        .ok_or_else(|| AppError::NotFound("Pending credential not found".to_string()))
}

pub async fn list_deliverable_queued_ciphertexts_for_node(
    db: &mongodb::Database,
    node_id: &str,
    limit: i64,
    now: DateTime<Utc>,
) -> AppResult<Vec<NodePendingCredential>> {
    db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .find(doc! {
            "node_id": node_id,
            "is_active": true,
            "remote_state": "ciphertext_queued",
            "expires_at": { "$gt": bson::DateTime::from_chrono(now) },
            "ciphertext_expires_at": { "$gt": bson::DateTime::from_chrono(now) },
            "crypto.version": "v1",
            "crypto.node_pubkey": { "$type": "string", "$ne": "" },
            "crypto.admin_pubkey": { "$type": "string" },
            "crypto.nonce": { "$type": "string" },
            "crypto.ciphertext": { "$exists": true },
        })
        .sort(doc! { "ciphertext_queued_at": 1, "created_at": 1 })
        .limit(limit.max(0))
        .await?
        .try_collect()
        .await
        .map_err(AppError::from)
}

pub async fn record_pending_credential_decrypt_result(
    db: &mongodb::Database,
    node_id: &str,
    pending_id: &str,
    outcome: PendingCredentialDecryptOutcome,
    now: DateTime<Utc>,
) -> AppResult<NodePendingCredential> {
    let (state, consumed_at) = match outcome {
        PendingCredentialDecryptOutcome::Ok => (RemoteCryptoState::Consumed, Some(now)),
        PendingCredentialDecryptOutcome::Error => (RemoteCryptoState::DecryptFailed, None),
    };

    let mut set_doc = doc! {
        "remote_state": remote_state_bson(state)?,
        "is_active": false,
    };
    if let Some(consumed_at) = consumed_at {
        set_doc.insert("consumed_at", bson::DateTime::from_chrono(consumed_at));
    }

    db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .find_one_and_update(
            doc! {
                "_id": pending_id,
                "node_id": node_id,
                "is_active": true,
                "expires_at": { "$gt": bson::DateTime::from_chrono(now) },
            },
            doc! {
                "$set": set_doc,
                "$unset": {
                    "crypto.admin_pubkey": "",
                    "crypto.nonce": "",
                    "crypto.ciphertext": "",
                    "ciphertext_queued_at": "",
                    "ciphertext_expires_at": "",
                },
            },
        )
        .return_document(ReturnDocument::After)
        .await?
        .ok_or_else(|| AppError::NotFound("Pending credential not found".to_string()))
}

pub async fn cancel_pending_credential(
    db: &mongodb::Database,
    actor_user_id: &str,
    node_id: &str,
    pending_id: &str,
) -> AppResult<NodePendingCredential> {
    node_service::ensure_node_writable_by_actor(db, actor_user_id, node_id).await?;

    // Consume rejects expired pushes because accepting stale setup metadata is
    // correctness-critical. Cancel intentionally remains admin housekeeping:
    // it can deactivate an expired active row so cleanup is idempotent.
    let now = bson::DateTime::from_chrono(Utc::now());
    db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .find_one_and_update(
            doc! {
                "_id": pending_id,
                "node_id": node_id,
                "is_active": true,
            },
            doc! { "$set": { "is_active": false, "updated_at": &now } },
        )
        .await?
        .ok_or_else(|| AppError::NotFound("Pending credential not found".to_string()))
}

pub async fn consume_pending_credential_for_node(
    db: &mongodb::Database,
    node_id: &str,
    pending_id: &str,
) -> AppResult<NodePendingCredential> {
    complete_pending_credential_for_node(db, node_id, pending_id, CompletionKind::Consumed).await
}

pub async fn decline_pending_credential_for_node(
    db: &mongodb::Database,
    node_id: &str,
    pending_id: &str,
) -> AppResult<NodePendingCredential> {
    complete_pending_credential_for_node(db, node_id, pending_id, CompletionKind::Declined).await
}

enum CompletionKind {
    Consumed,
    Declined,
}

async fn complete_pending_credential_for_node(
    db: &mongodb::Database,
    node_id: &str,
    pending_id: &str,
    kind: CompletionKind,
) -> AppResult<NodePendingCredential> {
    let now_chrono = Utc::now();
    let now = bson::DateTime::from_chrono(now_chrono);
    let timestamp_field = match kind {
        CompletionKind::Consumed => "consumed_at",
        CompletionKind::Declined => "declined_at",
    };
    let mut set_doc = doc! { "is_active": false };
    set_doc.insert(timestamp_field, now);

    db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .find_one_and_update(
            doc! {
                "_id": pending_id,
                "node_id": node_id,
                "is_active": true,
                "expires_at": { "$gt": bson::DateTime::from_chrono(now_chrono) },
            },
            doc! { "$set": set_doc },
        )
        .await?
        .ok_or_else(|| AppError::NotFound("Pending credential not found".to_string()))
}

async fn load_active_unexpired_pending_credential(
    db: &mongodb::Database,
    node_id: &str,
    pending_id: &str,
    now: DateTime<Utc>,
) -> AppResult<NodePendingCredential> {
    db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .find_one(doc! {
            "_id": pending_id,
            "node_id": node_id,
            "is_active": true,
            "expires_at": { "$gt": bson::DateTime::from_chrono(now) },
        })
        .await?
        .ok_or_else(|| AppError::NotFound("Pending credential not found".to_string()))
}

async fn active_unexpired_queued_ciphertext_count(
    db: &mongodb::Database,
    node_id: &str,
    now: DateTime<Utc>,
) -> AppResult<u64> {
    db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .count_documents(doc! {
            "node_id": node_id,
            "is_active": true,
            "remote_state": "ciphertext_queued",
            "ciphertext_expires_at": { "$gt": bson::DateTime::from_chrono(now) },
        })
        .await
        .map_err(AppError::from)
}

fn has_ciphertext(pending: &NodePendingCredential) -> bool {
    pending
        .crypto
        .as_ref()
        .and_then(|crypto| crypto.ciphertext.as_ref())
        .is_some()
}

fn pending_pubkey_missing(pending: &NodePendingCredential) -> bool {
    match pending.crypto.as_ref() {
        Some(crypto) => crypto.node_pubkey.is_empty(),
        None => true,
    }
}

fn stored_ciphertext_outcome(
    pending: NodePendingCredential,
) -> AppResult<StorePendingCiphertextOutcome> {
    if matches!(
        pending.remote_state.as_ref(),
        Some(RemoteCryptoState::CiphertextQueued)
    ) {
        Ok(StorePendingCiphertextOutcome::QueuedOffline(pending))
    } else {
        Ok(StorePendingCiphertextOutcome::StoredForOnlineNode(pending))
    }
}

fn remote_state_bson(state: RemoteCryptoState) -> AppResult<Bson> {
    bson::to_bson(&state)
        .map_err(|err| AppError::Internal(format!("remote state serialization failed: {err}")))
}

fn validate_service_slug(slug: &str) -> AppResult<()> {
    if slug.is_empty() || slug.len() > 64 {
        return Err(AppError::ValidationError(
            "service_slug must be 1-64 characters".to_string(),
        ));
    }
    let valid = slug
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && slug
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && slug
            .chars()
            .last()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    if !valid {
        return Err(AppError::ValidationError(
            "service_slug must be lowercase alphanumeric with optional hyphens, and cannot start or end with hyphen".to_string(),
        ));
    }
    Ok(())
}

fn validate_field_name(field_name: &str, injection_method: &InjectionMethod) -> AppResult<()> {
    if field_name.is_empty() || field_name.len() > 128 {
        return Err(AppError::ValidationError(
            "field_name must be 1-128 characters".to_string(),
        ));
    }

    match injection_method {
        InjectionMethod::Header => {
            for ch in field_name.chars() {
                if !is_http_token_char(ch) {
                    return Err(disallowed_field_char_error("header", ch));
                }
            }
        }
        InjectionMethod::QueryParam => {
            validate_percent_encoding(field_name, "query-param")?;
            for ch in field_name.chars() {
                if ch == '%' {
                    continue;
                }
                if !is_rfc3986_unreserved(ch) {
                    return Err(disallowed_field_char_error("query-param", ch));
                }
            }
        }
        InjectionMethod::PathPrefix => {
            validate_percent_encoding(field_name, "path-prefix")?;
            for ch in field_name.chars() {
                if ch == '%' {
                    continue;
                }
                if ch.is_control() || ch.is_whitespace() || matches!(ch, '?' | '#') {
                    return Err(disallowed_field_char_error("path-prefix", ch));
                }
                if !ch.is_ascii() {
                    return Err(disallowed_field_char_error("path-prefix", ch));
                }
            }
        }
    }

    Ok(())
}

fn is_http_token_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric()
        || matches!(
            ch,
            '!' | '#'
                | '$'
                | '%'
                | '&'
                | '\''
                | '*'
                | '+'
                | '-'
                | '.'
                | '^'
                | '_'
                | '`'
                | '|'
                | '~'
        )
}

fn is_rfc3986_unreserved(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '-' | '.' | '_' | '~')
}

fn validate_percent_encoding(value: &str, method: &str) -> AppResult<()> {
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let valid = index + 2 < bytes.len()
                && bytes[index + 1].is_ascii_hexdigit()
                && bytes[index + 2].is_ascii_hexdigit();
            if !valid {
                return Err(AppError::ValidationError(format!(
                    "field_name for {method} contains invalid percent-encoding"
                )));
            }
            index += 3;
        } else {
            index += 1;
        }
    }
    Ok(())
}

fn disallowed_field_char_error(method: &str, ch: char) -> AppError {
    let display = match ch {
        ' ' => "space".to_string(),
        '\t' => "tab".to_string(),
        '\n' => "newline".to_string(),
        '\r' => "carriage return".to_string(),
        _ => ch.to_string(),
    };
    AppError::ValidationError(format!(
        "field_name for {method} contains disallowed character '{display}'"
    ))
}

fn clean_optional_string(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::node::{COLLECTION_NAME as NODES, Node, NodeMetrics, NodeStatus};
    use crate::models::org_membership::{
        COLLECTION_NAME as ORG_MEMBERSHIPS, OrgMembership, OrgRole,
    };
    use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
    use crate::services::node_service;
    use crate::test_utils::{connect_test_database, test_membership, test_user};

    async fn test_db(prefix: &str) -> mongodb::Database {
        connect_test_database(prefix)
            .await
            .expect("local MongoDB required for pending credential tests")
    }

    fn test_node(owner_id: &str, name: &str) -> Node {
        let now = Utc::now();
        Node {
            id: Uuid::new_v4().to_string(),
            user_id: owner_id.to_string(),
            name: name.to_string(),
            status: NodeStatus::Offline,
            auth_token_hash: "auth-hash".to_string(),
            signing_secret_encrypted: None,
            signing_secret_hash: "signing-hash".to_string(),
            last_heartbeat_at: None,
            connected_at: None,
            metadata: None,
            metrics: NodeMetrics::default(),
            is_active: true,
            created_at: now,
            updated_at: now,
        }
    }

    fn credential_input(service_slug: &str) -> CreatePendingCredentialInput {
        CreatePendingCredentialInput {
            service_slug: service_slug.to_string(),
            injection_method: InjectionMethod::Header,
            field_name: "X-API-Key".to_string(),
            target_url: None,
            label: Some("Production".to_string()),
            ttl_secs: 86_400,
            remote_crypto: false,
        }
    }

    fn remote_credential_input(service_slug: &str) -> CreatePendingCredentialInput {
        CreatePendingCredentialInput {
            remote_crypto: true,
            ..credential_input(service_slug)
        }
    }

    fn ciphertext_input(
        admin_pubkey: impl Into<String>,
        nonce: impl Into<String>,
        ciphertext: Vec<u8>,
    ) -> StorePendingCiphertextInput {
        StorePendingCiphertextInput::new(admin_pubkey.into(), nonce.into(), ciphertext)
    }

    async fn insert_users(db: &mongodb::Database, users: Vec<User>) {
        db.collection::<User>(USERS)
            .insert_many(users)
            .await
            .expect("insert users");
    }

    async fn insert_membership(db: &mongodb::Database, membership: OrgMembership) {
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(membership)
            .await
            .expect("insert membership");
    }

    async fn insert_node(db: &mongodb::Database, node: &Node) {
        db.collection::<Node>(NODES)
            .insert_one(node)
            .await
            .expect("insert node");
    }

    async fn load_pending(db: &mongodb::Database, pending_id: &str) -> NodePendingCredential {
        db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
            .find_one(doc! { "_id": pending_id })
            .await
            .expect("query pending credential")
            .expect("pending credential exists")
    }

    fn assert_pubkey_only_pending(pending: &NodePendingCredential, expected_node_pubkey: &str) {
        assert!(pending.is_active);
        assert_eq!(pending.remote_state, Some(RemoteCryptoState::PubkeyPosted));
        assert!(pending.ciphertext_queued_at.is_none());
        assert!(pending.ciphertext_expires_at.is_none());
        let crypto = pending.crypto.as_ref().expect("crypto metadata");
        assert_eq!(crypto.version, "v1");
        assert_eq!(crypto.node_pubkey, expected_node_pubkey);
        assert!(crypto.admin_pubkey.is_none());
        assert!(crypto.nonce.is_none());
        assert!(crypto.ciphertext.is_none());
    }

    fn assert_invalid_field_name(method: InjectionMethod, field_name: &str, expected: &str) {
        let err = validate_field_name(field_name, &method).expect_err("field name should fail");
        assert!(
            matches!(err, AppError::ValidationError(ref message) if message.contains(expected)),
            "unexpected error: {err:?}"
        );
    }

    #[test]
    fn store_pending_ciphertext_input_debug_redacts_material() {
        let input = ciphertext_input("admin-pubkey-secret", "nonce-secret", vec![1, 2, 3]);

        let debug = format!("{input:?}");

        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("admin-pubkey-secret"));
        assert!(!debug.contains("nonce-secret"));
        assert!(!debug.contains("[1, 2, 3]"));
    }

    #[test]
    fn store_pending_ciphertext_outcome_debug_redacts_pending_crypto() {
        let now = Utc::now();
        let pending = NodePendingCredential {
            id: "pending-id".to_string(),
            node_id: "node-id".to_string(),
            service_slug: "openclaw".to_string(),
            injection_method: InjectionMethod::Header,
            field_name: "X-API-Key".to_string(),
            target_url: None,
            label: None,
            created_by_user_id: "user-id".to_string(),
            owner_user_id: "user-id".to_string(),
            created_at: now,
            expires_at: now + Duration::hours(1),
            consumed_at: None,
            declined_at: None,
            crypto: Some(crate::models::node_pending_credential::CryptoBundle {
                version: "v1".to_string(),
                node_pubkey: "node-pubkey-secret".to_string(),
                admin_pubkey: Some("admin-pubkey-secret".to_string()),
                nonce: Some("nonce-secret".to_string()),
                ciphertext: Some(vec![1, 2, 3]),
            }),
            remote_state: Some(RemoteCryptoState::CiphertextQueued),
            ciphertext_queued_at: Some(now),
            ciphertext_expires_at: Some(now + Duration::minutes(15)),
            is_active: true,
        };

        let debug = format!(
            "{:?}",
            StorePendingCiphertextOutcome::QueuedOffline(pending)
        );

        assert!(debug.contains("pending-id"));
        assert!(!debug.contains("node-pubkey-secret"));
        assert!(!debug.contains("admin-pubkey-secret"));
        assert!(!debug.contains("nonce-secret"));
        assert!(!debug.contains("[1, 2, 3]"));
    }

    #[test]
    fn validates_header_field_name_as_http_token() {
        validate_field_name("X-API-Key", &InjectionMethod::Header).expect("valid header");
        validate_field_name("X_Custom!#$%&'*+-.^`|~", &InjectionMethod::Header)
            .expect("valid token chars");

        assert_invalid_field_name(InjectionMethod::Header, "X API Key", "space");
        assert_invalid_field_name(InjectionMethod::Header, "X:API-Key", ":");
        assert_invalid_field_name(InjectionMethod::Header, "X,API-Key", ",");
        assert_invalid_field_name(InjectionMethod::Header, "X-ÄPI-Key", "Ä");
    }

    #[test]
    fn validates_query_param_field_name_as_url_safe() {
        validate_field_name("api_key", &InjectionMethod::QueryParam).expect("valid param");
        validate_field_name("api-key.%7E", &InjectionMethod::QueryParam)
            .expect("valid percent-encoded param");

        assert_invalid_field_name(InjectionMethod::QueryParam, "api key", "space");
        assert_invalid_field_name(InjectionMethod::QueryParam, "api&key", "&");
        assert_invalid_field_name(InjectionMethod::QueryParam, "api=key", "=");
        assert_invalid_field_name(InjectionMethod::QueryParam, "api?key", "?");
        assert_invalid_field_name(InjectionMethod::QueryParam, "api#key", "#");
        assert_invalid_field_name(InjectionMethod::QueryParam, "api%key", "percent-encoding");
    }

    #[test]
    fn validates_path_prefix_field_name_as_path_component() {
        validate_field_name("v1/api/%2Ftenant", &InjectionMethod::PathPrefix)
            .expect("valid path prefix");

        assert_invalid_field_name(InjectionMethod::PathPrefix, "v1/api key", "space");
        assert_invalid_field_name(InjectionMethod::PathPrefix, "v1/api?key", "?");
        assert_invalid_field_name(InjectionMethod::PathPrefix, "v1/api#key", "#");
        assert_invalid_field_name(InjectionMethod::PathPrefix, "v1/%key", "percent-encoding");
    }

    #[tokio::test]
    async fn admin_push_creates_pending_credential_with_acl_fields() {
        let db = test_db("pending_credential_push").await;

        let admin_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        insert_users(
            &db,
            vec![
                test_user(&admin_id, UserType::Person),
                test_user(&org_id, UserType::Org),
            ],
        )
        .await;
        insert_membership(
            &db,
            test_membership(&org_id, &admin_id, OrgRole::Admin, None),
        )
        .await;
        let node = test_node(&org_id, "org-node");
        insert_node(&db, &node).await;

        let pending =
            create_pending_credential(&db, &admin_id, &node.id, credential_input("openclaw"))
                .await
                .expect("admin can push");

        assert_eq!(pending.node_id, node.id);
        assert_eq!(pending.service_slug, "openclaw");
        assert_eq!(pending.created_by_user_id, admin_id);
        assert_eq!(pending.owner_user_id, org_id);
        assert!(pending.is_active);

        let listed = list_pending_credentials_for_admin(&db, &admin_id, &node.id, false)
            .await
            .expect("admin can list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, pending.id);
    }

    #[tokio::test]
    async fn create_remote_crypto_false_keeps_legacy_crypto_none() {
        let db = test_db("pending_credential_remote_false").await;

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node = test_node(&actor_id, "personal-node");
        insert_node(&db, &node).await;

        let pending =
            create_pending_credential(&db, &actor_id, &node.id, credential_input("openclaw"))
                .await
                .expect("push succeeds");

        assert!(pending.crypto.is_none());
        assert!(pending.remote_state.is_none());
    }

    #[tokio::test]
    async fn create_remote_crypto_true_initializes_v1_without_pubkey() {
        let db = test_db("pending_credential_remote_true").await;

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node = test_node(&actor_id, "personal-node");
        insert_node(&db, &node).await;

        let pending = create_pending_credential(
            &db,
            &actor_id,
            &node.id,
            remote_credential_input("openclaw"),
        )
        .await
        .expect("remote push succeeds");

        let crypto = pending.crypto.expect("remote crypto metadata");
        assert_eq!(crypto.version, "v1");
        assert!(crypto.node_pubkey.is_empty());
        assert!(crypto.admin_pubkey.is_none());
        assert!(crypto.nonce.is_none());
        assert!(crypto.ciphertext.is_none());
        assert!(pending.remote_state.is_none());
    }

    #[tokio::test]
    async fn member_cannot_push_pending_credential_for_org_node() {
        let db = test_db("pending_credential_member_denied").await;

        let member_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        insert_users(
            &db,
            vec![
                test_user(&member_id, UserType::Person),
                test_user(&org_id, UserType::Org),
            ],
        )
        .await;
        insert_membership(
            &db,
            test_membership(&org_id, &member_id, OrgRole::Member, None),
        )
        .await;
        let node = test_node(&org_id, "org-node");
        insert_node(&db, &node).await;

        let err =
            create_pending_credential(&db, &member_id, &node.id, credential_input("openclaw"))
                .await
                .expect_err("member cannot push");
        assert!(matches!(err, AppError::NodeNotFound(_)));
    }

    #[tokio::test]
    async fn push_for_nonexistent_node_returns_not_found() {
        let db = test_db("pending_credential_missing_node").await;

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;

        let err = create_pending_credential(
            &db,
            &actor_id,
            &Uuid::new_v4().to_string(),
            credential_input("openclaw"),
        )
        .await
        .expect_err("missing node should fail");
        assert!(matches!(err, AppError::NodeNotFound(_)));
    }

    #[tokio::test]
    async fn duplicate_pending_slug_returns_conflict_with_existing_id() {
        let db = test_db("pending_credential_duplicate").await;

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node = test_node(&actor_id, "personal-node");
        insert_node(&db, &node).await;

        let first =
            create_pending_credential(&db, &actor_id, &node.id, credential_input("openclaw"))
                .await
                .expect("first push succeeds");
        let err = create_pending_credential(&db, &actor_id, &node.id, credential_input("openclaw"))
            .await
            .expect_err("duplicate push should fail");

        match err {
            AppError::Conflict(message) => {
                assert!(message.contains(&first.id));
            }
            other => panic!("expected conflict, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn push_accepts_internal_target_url() {
        let db = test_db("pending_credential_internal_url").await;

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node = test_node(&actor_id, "personal-node");
        insert_node(&db, &node).await;
        let mut input = credential_input("openclaw");
        input.target_url = Some("http://127.0.0.1:8080".to_string());

        let pending = create_pending_credential(&db, &actor_id, &node.id, input)
            .await
            .expect("internal URL is node-local advisory metadata");
        assert_eq!(pending.target_url.as_deref(), Some("http://127.0.0.1:8080"));
    }

    #[tokio::test]
    async fn node_consumes_own_pending_credential() {
        let db = test_db("pending_credential_consume").await;

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node = test_node(&actor_id, "personal-node");
        insert_node(&db, &node).await;
        let pending =
            create_pending_credential(&db, &actor_id, &node.id, credential_input("openclaw"))
                .await
                .expect("push succeeds");

        let returned = consume_pending_credential_for_node(&db, &node.id, &pending.id)
            .await
            .expect("node consumes own pending");
        assert_eq!(returned.id, pending.id);

        let stored = load_pending(&db, &pending.id).await;
        assert!(!stored.is_active);
        assert!(stored.consumed_at.is_some());
        assert!(stored.declined_at.is_none());
    }

    #[tokio::test]
    async fn node_cannot_consume_another_nodes_pending_credential() {
        let db = test_db("pending_credential_wrong_node").await;

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node_a = test_node(&actor_id, "node-a");
        let node_b = test_node(&actor_id, "node-b");
        insert_node(&db, &node_a).await;
        insert_node(&db, &node_b).await;
        let pending =
            create_pending_credential(&db, &actor_id, &node_a.id, credential_input("openclaw"))
                .await
                .expect("push succeeds");

        let err = consume_pending_credential_for_node(&db, &node_b.id, &pending.id)
            .await
            .expect_err("other node cannot consume");
        assert!(matches!(err, AppError::NotFound(_)));

        let stored = load_pending(&db, &pending.id).await;
        assert!(stored.is_active);
        assert!(stored.consumed_at.is_none());
    }

    #[tokio::test]
    async fn node_declines_pending_credential() {
        let db = test_db("pending_credential_decline").await;

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node = test_node(&actor_id, "personal-node");
        insert_node(&db, &node).await;
        let pending =
            create_pending_credential(&db, &actor_id, &node.id, credential_input("openclaw"))
                .await
                .expect("push succeeds");

        decline_pending_credential_for_node(&db, &node.id, &pending.id)
            .await
            .expect("node declines");

        let stored = load_pending(&db, &pending.id).await;
        assert!(!stored.is_active);
        assert!(stored.declined_at.is_some());
        assert!(stored.consumed_at.is_none());
    }

    #[tokio::test]
    async fn admin_cancel_prevents_later_consume() {
        let db = test_db("pending_credential_cancel").await;

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node = test_node(&actor_id, "personal-node");
        insert_node(&db, &node).await;
        let pending =
            create_pending_credential(&db, &actor_id, &node.id, credential_input("openclaw"))
                .await
                .expect("push succeeds");

        cancel_pending_credential(&db, &actor_id, &node.id, &pending.id)
            .await
            .expect("admin cancels");

        let err = consume_pending_credential_for_node(&db, &node.id, &pending.id)
            .await
            .expect_err("canceled row is not consumable");
        assert!(matches!(err, AppError::NotFound(_)));

        let stored = load_pending(&db, &pending.id).await;
        assert!(!stored.is_active);
        assert!(stored.consumed_at.is_none());
    }

    #[tokio::test]
    async fn expired_pending_credentials_are_not_listed() {
        let db = test_db("pending_credential_expired").await;

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node = test_node(&actor_id, "personal-node");
        insert_node(&db, &node).await;
        let now = Utc::now();
        let expired = NodePendingCredential {
            id: Uuid::new_v4().to_string(),
            node_id: node.id.clone(),
            service_slug: "expired".to_string(),
            injection_method: InjectionMethod::Header,
            field_name: "X-API-Key".to_string(),
            target_url: None,
            label: None,
            created_by_user_id: actor_id.clone(),
            owner_user_id: actor_id.clone(),
            created_at: now - Duration::hours(2),
            expires_at: now - Duration::hours(1),
            consumed_at: None,
            declined_at: None,
            crypto: None,
            remote_state: None,
            ciphertext_queued_at: None,
            ciphertext_expires_at: None,
            is_active: true,
        };
        db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
            .insert_one(&expired)
            .await
            .expect("insert expired pending");

        let admin_list = list_pending_credentials_for_admin(&db, &actor_id, &node.id, false)
            .await
            .expect("admin list succeeds");
        let node_list = list_pending_credentials_for_node(&db, &node.id)
            .await
            .expect("node list succeeds");

        assert!(admin_list.is_empty());
        assert!(node_list.is_empty());
    }

    #[tokio::test]
    async fn get_pending_credential_for_admin_returns_active_unexpired_row() {
        let db = test_db("pending_credential_admin_get").await;

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node = test_node(&actor_id, "personal-node");
        insert_node(&db, &node).await;
        let pending =
            create_pending_credential(&db, &actor_id, &node.id, credential_input("openclaw"))
                .await
                .expect("push succeeds");

        let returned = get_pending_credential_for_admin(&db, &actor_id, &node.id, &pending.id)
            .await
            .expect("admin can read active pending credential");

        assert_eq!(returned.id, pending.id);
        assert_eq!(returned.node_id, node.id);
        assert!(returned.is_active);
    }

    #[tokio::test]
    async fn get_pending_credential_for_admin_rejects_actor_without_node_access() {
        let db = test_db("pending_credential_admin_get_acl").await;

        let owner_id = Uuid::new_v4().to_string();
        let stranger_id = Uuid::new_v4().to_string();
        insert_users(
            &db,
            vec![
                test_user(&owner_id, UserType::Person),
                test_user(&stranger_id, UserType::Person),
            ],
        )
        .await;
        let node = test_node(&owner_id, "personal-node");
        insert_node(&db, &node).await;
        let pending =
            create_pending_credential(&db, &owner_id, &node.id, credential_input("openclaw"))
                .await
                .expect("push succeeds");

        let err = get_pending_credential_for_admin(&db, &stranger_id, &node.id, &pending.id)
            .await
            .expect_err("stranger cannot read another owner's pending credential");

        assert!(matches!(err, AppError::NodeNotFound(_)));
    }

    #[tokio::test]
    async fn get_pending_credential_for_admin_filters_inactive_and_expired_rows() {
        let db = test_db("pending_credential_admin_get_filters").await;

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node = test_node(&actor_id, "personal-node");
        insert_node(&db, &node).await;
        let inactive =
            create_pending_credential(&db, &actor_id, &node.id, credential_input("inactive"))
                .await
                .expect("push succeeds");
        let expired =
            create_pending_credential(&db, &actor_id, &node.id, credential_input("expired"))
                .await
                .expect("push succeeds");
        db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
            .update_one(
                doc! { "_id": &inactive.id },
                doc! { "$set": { "is_active": false } },
            )
            .await
            .expect("mark inactive");
        db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
            .update_one(
                doc! { "_id": &expired.id },
                doc! {
                    "$set": {
                        "expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::hours(1)),
                    },
                },
            )
            .await
            .expect("mark expired");

        let inactive_err = get_pending_credential_for_admin(&db, &actor_id, &node.id, &inactive.id)
            .await
            .expect_err("inactive pending credential is filtered");
        let expired_err = get_pending_credential_for_admin(&db, &actor_id, &node.id, &expired.id)
            .await
            .expect_err("expired pending credential is filtered");

        assert!(matches!(inactive_err, AppError::NotFound(_)));
        assert!(matches!(expired_err, AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn store_ciphertext_first_writer_wins_sets_ciphertext_once() {
        let db = test_db("pending_credential_ciphertext_first_writer").await;

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node = test_node(&actor_id, "personal-node");
        insert_node(&db, &node).await;
        let pending = create_pending_credential(
            &db,
            &actor_id,
            &node.id,
            remote_credential_input("openclaw"),
        )
        .await
        .expect("push succeeds");
        record_pending_credential_pubkey(&db, &node.id, &pending.id, "v1", "node-pubkey")
            .await
            .expect("record pubkey");

        let now = Utc::now();
        let first = store_pending_ciphertext_first_writer_wins(
            &db,
            &actor_id,
            &node.id,
            &pending.id,
            ciphertext_input("admin-pubkey-1", "nonce-1", vec![1, 2, 3]),
            true,
            now,
        )
        .await
        .expect("first writer stores ciphertext");
        match first {
            StorePendingCiphertextOutcome::StoredForOnlineNode(stored) => {
                assert_eq!(
                    stored.remote_state,
                    Some(RemoteCryptoState::CiphertextReceived)
                );
                assert_eq!(
                    stored.crypto.and_then(|crypto| crypto.ciphertext),
                    Some(vec![1, 2, 3])
                );
            }
            other => panic!("expected online storage, got {other:?}"),
        }

        let second = store_pending_ciphertext_first_writer_wins(
            &db,
            &actor_id,
            &node.id,
            &pending.id,
            ciphertext_input("admin-pubkey-2", "nonce-2", vec![9, 9, 9]),
            true,
            now,
        )
        .await
        .expect("second writer observes existing ciphertext");
        match second {
            StorePendingCiphertextOutcome::StoredForOnlineNode(stored) => {
                assert_eq!(
                    stored.crypto.and_then(|crypto| crypto.ciphertext),
                    Some(vec![1, 2, 3])
                );
            }
            other => panic!("expected existing online storage, got {other:?}"),
        }

        let stored = load_pending(&db, &pending.id).await;
        let crypto = stored.crypto.expect("crypto bundle");
        assert_eq!(crypto.admin_pubkey.as_deref(), Some("admin-pubkey-1"));
        assert_eq!(crypto.nonce.as_deref(), Some("nonce-1"));
        assert_eq!(crypto.ciphertext, Some(vec![1, 2, 3]));
    }

    #[tokio::test]
    async fn store_ciphertext_rejects_non_writable_actor_without_state_change() {
        let db = test_db("pending_credential_ciphertext_acl_denied").await;

        let admin_id = Uuid::new_v4().to_string();
        let member_id = Uuid::new_v4().to_string();
        let stranger_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        insert_users(
            &db,
            vec![
                test_user(&admin_id, UserType::Person),
                test_user(&member_id, UserType::Person),
                test_user(&stranger_id, UserType::Person),
                test_user(&org_id, UserType::Org),
            ],
        )
        .await;
        insert_membership(
            &db,
            test_membership(&org_id, &admin_id, OrgRole::Admin, None),
        )
        .await;
        insert_membership(
            &db,
            test_membership(&org_id, &member_id, OrgRole::Member, None),
        )
        .await;
        let node = test_node(&org_id, "org-node");
        insert_node(&db, &node).await;
        let pending = create_pending_credential(
            &db,
            &admin_id,
            &node.id,
            remote_credential_input("openclaw"),
        )
        .await
        .expect("org admin can create pending credential");
        record_pending_credential_pubkey(&db, &node.id, &pending.id, "v1", "node-pubkey")
            .await
            .expect("record pubkey");
        let before = load_pending(&db, &pending.id).await;
        assert_pubkey_only_pending(&before, "node-pubkey");

        for denied_actor_id in [&member_id, &stranger_id] {
            let err = store_pending_ciphertext_first_writer_wins(
                &db,
                denied_actor_id,
                &node.id,
                &pending.id,
                ciphertext_input("admin-pubkey", "nonce", vec![1, 2, 3]),
                false,
                Utc::now(),
            )
            .await
            .expect_err("actor without node write access cannot store ciphertext");

            assert!(matches!(err, AppError::NodeNotFound(_)));
            let stored = load_pending(&db, &pending.id).await;
            assert_pubkey_only_pending(&stored, "node-pubkey");
        }
    }

    #[tokio::test]
    async fn record_pubkey_is_first_writer_and_does_not_overwrite() {
        let db = test_db("pending_credential_pubkey_first_writer").await;

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node = test_node(&actor_id, "personal-node");
        insert_node(&db, &node).await;
        let pending = create_pending_credential(
            &db,
            &actor_id,
            &node.id,
            remote_credential_input("openclaw"),
        )
        .await
        .expect("push succeeds");

        let first = record_pending_credential_pubkey(&db, &node.id, &pending.id, "v1", "node-1")
            .await
            .expect("first pubkey records");
        let second = record_pending_credential_pubkey(&db, &node.id, &pending.id, "v1", "node-2")
            .await
            .expect("second pubkey returns existing");

        assert_eq!(
            first
                .crypto
                .as_ref()
                .map(|crypto| crypto.node_pubkey.as_str()),
            Some("node-1")
        );
        assert_eq!(
            second
                .crypto
                .as_ref()
                .map(|crypto| crypto.node_pubkey.as_str()),
            Some("node-1")
        );
        assert_eq!(second.remote_state, Some(RemoteCryptoState::PubkeyPosted));
    }

    #[tokio::test]
    async fn send_failure_queue_marking_and_mark_sent_transition() {
        let db = test_db("pending_credential_send_failure_queue").await;

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node = test_node(&actor_id, "personal-node");
        insert_node(&db, &node).await;
        let pending = create_pending_credential(
            &db,
            &actor_id,
            &node.id,
            remote_credential_input("openclaw"),
        )
        .await
        .expect("push succeeds");
        record_pending_credential_pubkey(&db, &node.id, &pending.id, "v1", "node-pubkey")
            .await
            .expect("record pubkey");
        let now = Utc::now();
        let stored = store_pending_ciphertext_first_writer_wins(
            &db,
            &actor_id,
            &node.id,
            &pending.id,
            ciphertext_input("admin-pubkey", "nonce", vec![1, 2, 3]),
            true,
            now,
        )
        .await
        .expect("store online");
        assert!(matches!(
            stored,
            StorePendingCiphertextOutcome::StoredForOnlineNode(_)
        ));

        let queued =
            mark_pending_ciphertext_queued_after_send_failure(&db, &node.id, &pending.id, now)
                .await
                .expect("mark queued");
        assert_eq!(
            queued.remote_state,
            Some(RemoteCryptoState::CiphertextQueued)
        );
        assert!(queued.ciphertext_queued_at.is_some());

        let deliverable = list_deliverable_queued_ciphertexts_for_node(&db, &node.id, 10, now)
            .await
            .expect("list queued");
        assert_eq!(deliverable.len(), 1);
        assert_eq!(deliverable[0].id, pending.id);

        let sent = mark_queued_ciphertext_sent(&db, &node.id, &pending.id, now)
            .await
            .expect("mark sent");
        assert_eq!(
            sent.remote_state,
            Some(RemoteCryptoState::CiphertextReceived)
        );
        assert!(sent.ciphertext_queued_at.is_none());
        assert!(sent.ciphertext_expires_at.is_none());
    }

    #[tokio::test]
    async fn decrypt_result_ok_and_error_clear_ciphertext_without_persisted_error_code() {
        let db = test_db("pending_credential_decrypt_result").await;

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node = test_node(&actor_id, "personal-node");
        insert_node(&db, &node).await;
        let now = Utc::now();
        for (service_slug, outcome, expected_state, expect_consumed) in [
            (
                "decrypt-ok",
                PendingCredentialDecryptOutcome::Ok,
                RemoteCryptoState::Consumed,
                true,
            ),
            (
                "decrypt-error",
                PendingCredentialDecryptOutcome::Error,
                RemoteCryptoState::DecryptFailed,
                false,
            ),
        ] {
            let pending = create_pending_credential(
                &db,
                &actor_id,
                &node.id,
                remote_credential_input(service_slug),
            )
            .await
            .expect("push succeeds");
            record_pending_credential_pubkey(&db, &node.id, &pending.id, "v1", "node-pubkey")
                .await
                .expect("record pubkey");
            store_pending_ciphertext_first_writer_wins(
                &db,
                &actor_id,
                &node.id,
                &pending.id,
                ciphertext_input("admin-pubkey", "nonce", vec![1, 2, 3]),
                true,
                now,
            )
            .await
            .expect("store ciphertext");

            let returned =
                record_pending_credential_decrypt_result(&db, &node.id, &pending.id, outcome, now)
                    .await
                    .expect("record decrypt result");
            assert!(!returned.is_active);
            assert_eq!(returned.remote_state, Some(expected_state));
            assert_eq!(returned.consumed_at.is_some(), expect_consumed);

            let stored = db
                .collection::<bson::Document>(NODE_PENDING_CREDENTIALS)
                .find_one(doc! { "_id": &pending.id })
                .await
                .expect("query raw pending")
                .expect("pending exists");
            let forbidden_field = ["remote", "error", "code"].join("_");
            assert!(stored.get(&forbidden_field).is_none());
            let crypto = stored.get_document("crypto").expect("crypto document");
            assert!(crypto.get("admin_pubkey").is_none());
            assert!(crypto.get("nonce").is_none());
            assert!(crypto.get("ciphertext").is_none());
        }
    }

    #[tokio::test]
    async fn store_ciphertext_offline_returns_queue_full_when_cap_reached() {
        let db = test_db("pending_credential_queue_full").await;

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node = test_node(&actor_id, "personal-node");
        insert_node(&db, &node).await;
        let now = Utc::now();
        for index in 0..MAX_OFFLINE_CIPHERTEXT_QUEUE_PER_NODE {
            let pending = create_pending_credential(
                &db,
                &actor_id,
                &node.id,
                remote_credential_input(&format!("service-{index}")),
            )
            .await
            .expect("push succeeds");
            record_pending_credential_pubkey(&db, &node.id, &pending.id, "v1", "node-pubkey")
                .await
                .expect("record pubkey");
            let outcome = store_pending_ciphertext_first_writer_wins(
                &db,
                &actor_id,
                &node.id,
                &pending.id,
                ciphertext_input(
                    format!("admin-pubkey-{index}"),
                    format!("nonce-{index}"),
                    vec![index as u8],
                ),
                false,
                now,
            )
            .await
            .expect("queue ciphertext offline");
            assert!(matches!(
                outcome,
                StorePendingCiphertextOutcome::QueuedOffline(_)
            ));
        }

        let pending = create_pending_credential(
            &db,
            &actor_id,
            &node.id,
            remote_credential_input("service-full"),
        )
        .await
        .expect("push succeeds");
        record_pending_credential_pubkey(&db, &node.id, &pending.id, "v1", "node-pubkey")
            .await
            .expect("record pubkey");
        let outcome = store_pending_ciphertext_first_writer_wins(
            &db,
            &actor_id,
            &node.id,
            &pending.id,
            ciphertext_input("admin-pubkey-full", "nonce-full", vec![42]),
            false,
            now,
        )
        .await
        .expect("full offline queue returns a business outcome");

        assert!(matches!(outcome, StorePendingCiphertextOutcome::QueueFull));
        let stored = load_pending(&db, &pending.id).await;
        assert_eq!(stored.remote_state, Some(RemoteCryptoState::PubkeyPosted));
        assert!(stored.crypto.and_then(|crypto| crypto.ciphertext).is_none());
    }

    #[tokio::test]
    async fn store_ciphertext_rejects_oversized_ciphertext() {
        let db = test_db("pending_credential_ciphertext_too_large").await;

        let err = store_pending_ciphertext_first_writer_wins(
            &db,
            "actor",
            "node",
            "pending",
            ciphertext_input("admin-pubkey", "nonce", vec![0; MAX_CIPHERTEXT_SIZE + 1]),
            true,
            Utc::now(),
        )
        .await
        .expect_err("oversized ciphertext should fail before storing");

        assert!(matches!(
            err,
            AppError::PendingCredentialCiphertextTooLarge(size)
                if size == MAX_CIPHERTEXT_SIZE + 1
        ));
    }

    #[tokio::test]
    async fn store_ciphertext_before_pubkey_returns_pubkey_awaiting() {
        let db = test_db("pending_credential_pubkey_awaiting").await;

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node = test_node(&actor_id, "personal-node");
        insert_node(&db, &node).await;
        let pending = create_pending_credential(
            &db,
            &actor_id,
            &node.id,
            remote_credential_input("openclaw"),
        )
        .await
        .expect("push succeeds");

        let err = store_pending_ciphertext_first_writer_wins(
            &db,
            &actor_id,
            &node.id,
            &pending.id,
            ciphertext_input("admin-pubkey", "nonce", vec![1, 2, 3]),
            true,
            Utc::now(),
        )
        .await
        .expect_err("ciphertext cannot be stored before node pubkey");

        assert!(matches!(
            err,
            AppError::PendingCredentialPubkeyAwaiting(id) if id == pending.id
        ));
        let stored = load_pending(&db, &pending.id).await;
        assert!(
            stored
                .crypto
                .as_ref()
                .is_some_and(|crypto| crypto.node_pubkey.is_empty())
        );
    }

    #[tokio::test]
    async fn queue_cap_counts_only_active_unexpired_ciphertexts() {
        let db = test_db("pending_credential_queue_cap").await;

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node = test_node(&actor_id, "personal-node");
        insert_node(&db, &node).await;
        let now = Utc::now();
        let mut pending_ids = Vec::new();
        for index in 0..MAX_OFFLINE_CIPHERTEXT_QUEUE_PER_NODE {
            let pending = create_pending_credential(
                &db,
                &actor_id,
                &node.id,
                remote_credential_input(&format!("service-{index}")),
            )
            .await
            .expect("push succeeds");
            record_pending_credential_pubkey(&db, &node.id, &pending.id, "v1", "node-pubkey")
                .await
                .expect("record pubkey");
            store_pending_ciphertext_first_writer_wins(
                &db,
                &actor_id,
                &node.id,
                &pending.id,
                ciphertext_input(
                    format!("admin-pubkey-{index}"),
                    format!("nonce-{index}"),
                    vec![index as u8],
                ),
                false,
                now,
            )
            .await
            .expect("queue ciphertext offline");
            pending_ids.push(pending.id);
        }

        let count = active_unexpired_queued_ciphertext_count(&db, &node.id, now)
            .await
            .expect("count queued");
        assert_eq!(count, MAX_OFFLINE_CIPHERTEXT_QUEUE_PER_NODE);

        db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
            .update_one(
                doc! { "_id": &pending_ids[0] },
                doc! { "$set": { "is_active": false } },
            )
            .await
            .expect("mark inactive");
        db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
            .update_one(
                doc! { "_id": &pending_ids[1] },
                doc! {
                    "$set": {
                        "ciphertext_expires_at": bson::DateTime::from_chrono(now - Duration::seconds(1)),
                    },
                },
            )
            .await
            .expect("mark expired");

        let count = active_unexpired_queued_ciphertext_count(&db, &node.id, now)
            .await
            .expect("count queued");
        assert_eq!(count, MAX_OFFLINE_CIPHERTEXT_QUEUE_PER_NODE - 2);

        let pending = create_pending_credential(
            &db,
            &actor_id,
            &node.id,
            remote_credential_input("service-extra"),
        )
        .await
        .expect("push succeeds");
        record_pending_credential_pubkey(&db, &node.id, &pending.id, "v1", "node-pubkey")
            .await
            .expect("record pubkey");
        let outcome = store_pending_ciphertext_first_writer_wins(
            &db,
            &actor_id,
            &node.id,
            &pending.id,
            ciphertext_input("admin-pubkey-extra", "nonce-extra", vec![42]),
            false,
            now,
        )
        .await
        .expect("queue should have capacity after inactive and expired rows");
        assert!(matches!(
            outcome,
            StorePendingCiphertextOutcome::QueuedOffline(_)
        ));
    }

    #[tokio::test]
    async fn expire_queued_ciphertexts_marks_expired_and_unsets_ciphertext() {
        let db = test_db("pending_credential_expire_queued").await;

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node = test_node(&actor_id, "personal-node");
        insert_node(&db, &node).await;
        let pending = create_pending_credential(
            &db,
            &actor_id,
            &node.id,
            remote_credential_input("openclaw"),
        )
        .await
        .expect("push succeeds");
        record_pending_credential_pubkey(&db, &node.id, &pending.id, "v1", "node-pubkey")
            .await
            .expect("record pubkey");
        let now = Utc::now();
        store_pending_ciphertext_first_writer_wins(
            &db,
            &actor_id,
            &node.id,
            &pending.id,
            ciphertext_input("admin-pubkey", "nonce", vec![7, 8, 9]),
            false,
            now,
        )
        .await
        .expect("queue ciphertext offline");

        let modified = expire_queued_ciphertexts(
            &db,
            now + Duration::seconds(OFFLINE_CIPHERTEXT_QUEUE_TTL_SECS + 1),
        )
        .await
        .expect("expire queued ciphertexts");
        assert_eq!(modified, 1);

        let stored = load_pending(&db, &pending.id).await;
        assert!(!stored.is_active);
        assert_eq!(stored.remote_state, Some(RemoteCryptoState::Expired));
        assert!(stored.ciphertext_queued_at.is_none());
        assert!(stored.ciphertext_expires_at.is_none());
        let crypto = stored
            .crypto
            .expect("crypto bundle remains for pubkey metadata");
        assert_eq!(crypto.version, "v1");
        assert_eq!(crypto.node_pubkey, "node-pubkey");
        assert!(crypto.admin_pubkey.is_none());
        assert!(crypto.nonce.is_none());
        assert!(crypto.ciphertext.is_none());
    }

    #[tokio::test]
    async fn transfer_deactivates_pending_credentials_for_node() {
        let db = test_db("pending_credential_transfer").await;

        let actor_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        insert_users(
            &db,
            vec![
                test_user(&actor_id, UserType::Person),
                test_user(&org_id, UserType::Org),
            ],
        )
        .await;
        insert_membership(
            &db,
            test_membership(&org_id, &actor_id, OrgRole::Admin, None),
        )
        .await;
        let node = test_node(&actor_id, "personal-node");
        insert_node(&db, &node).await;
        let pending =
            create_pending_credential(&db, &actor_id, &node.id, credential_input("openclaw"))
                .await
                .expect("push succeeds");

        let transfer = node_service::transfer_node_owner(&db, &actor_id, &node.id, &org_id, 10)
            .await
            .expect("transfer succeeds");
        assert_eq!(transfer.deactivated_pending_credentials_count, 1);

        let stored = load_pending(&db, &pending.id).await;
        assert!(!stored.is_active);
    }
}
