//! Shared reserve-then-commit receipt discipline for assistant action
//! effects (WS-0).
//!
//! Wave 1 established the pattern inside
//! `assistant_action_execution_service` (`key.create` / `key.rotate`): a
//! durable, secret-free [`AssistantActionReceipt`] is reserved *before* the
//! effect is attempted, uniquely keyed by `(user_id, action,
//! action_request_id)`, carrying a canonical fingerprint of the typed
//! request and the pre-reserved safe resource identity. An exact retry
//! replays the committed resource id without re-running the effect; reuse of
//! the same `actionRequestId` with different content fails closed with a
//! conflict. That service stays self-contained so the shipped verbs keep
//! their hardened, test-covered path byte-for-byte; Wave-2 effect handlers
//! use this generic module instead of re-implementing the discipline.
//!
//! Caller contract per verb:
//! 1. Normalize the typed request; build its fingerprint with
//!    [`fingerprint_canonical`] over a struct that embeds the action name.
//! 2. Call [`reserve_or_replay`]. On [`ReceiptOutcome::Replay`], return the
//!    receipt's `resource_id` (and never any one-time material). On
//!    [`ReceiptOutcome::Reserved`], perform the effect against
//!    `receipt.resource_id`, then call [`mark_completed`].
//! 3. For create-shaped verbs pass a freshly minted UUID as the resource
//!    identity; for verbs acting on an existing resource pass that
//!    resource's id, so the replay answer is stable either way.

// WS-0 lands this module ahead of its consumers; the first Wave-2 effect
// handler that uses it must remove this allow so dead paths surface again.
#![allow(dead_code)]

use mongodb::bson::doc;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::errors::{AppError, AppResult};
use crate::models::assistant_action_receipt::{
    AssistantActionReceipt, AssistantActionReceiptStatus,
    COLLECTION_NAME as ASSISTANT_ACTION_RECEIPTS,
};

const MAX_ACTION_REQUEST_ID_LEN: usize = 256;

/// Outcome of a receipt reservation attempt.
#[derive(Debug)]
pub enum ReceiptOutcome {
    /// First appearance of this action identity: the caller owns performing
    /// the effect against `receipt.resource_id`, then [`mark_completed`].
    Reserved(AssistantActionReceipt),
    /// Exact retry of an already-reserved identity: the caller must answer
    /// from the receipt without re-running the effect and without returning
    /// any one-time material.
    Replay(AssistantActionReceipt),
}

/// Mirror of the browser control-identity rules for `actionRequestId`
/// (same semantics as the Wave-1 service and the frontend
/// `actionControlIdentitySchema`).
pub fn normalize_action_request_id(value: String) -> AppResult<String> {
    let value = value.trim().to_string();
    if value.is_empty()
        || value.len() > MAX_ACTION_REQUEST_ID_LEN
        || value.chars().any(|character| {
            character.is_whitespace()
                || character.is_control()
                || matches!(character, '/' | '\\' | '?' | '#')
        })
    {
        return Err(AppError::ValidationError(
            "actionRequestId must be a valid control identity of at most 256 characters"
                .to_string(),
        ));
    }
    Ok(value)
}

/// SHA-256 over the canonical JSON of a fingerprint struct. The struct must
/// embed the action name and every semantic request field (and nothing
/// secret), so equal fingerprints mean "the same request", exactly as the
/// Wave-1 per-verb fingerprint structs do.
pub fn fingerprint_canonical<T: Serialize>(payload: &T) -> AppResult<String> {
    let canonical = serde_json::to_vec(payload)
        .map_err(|error| AppError::Internal(format!("failed to fingerprint action: {error}")))?;
    Ok(hex::encode(Sha256::digest(canonical)))
}

fn utc_now_at_bson_precision() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp_millis(chrono::Utc::now().timestamp_millis())
        .expect("current UTC timestamp must fit BSON precision")
}

fn duplicate_key(error: &mongodb::error::Error) -> bool {
    matches!(
        error.kind.as_ref(),
        mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(write_error))
            if write_error.code == 11000
    )
}

async fn find_receipt(
    db: &mongodb::Database,
    user_id: &str,
    action: &str,
    action_request_id: &str,
) -> AppResult<Option<AssistantActionReceipt>> {
    Ok(db
        .collection::<AssistantActionReceipt>(ASSISTANT_ACTION_RECEIPTS)
        .find_one(doc! {
            "user_id": user_id,
            "action": action,
            "action_request_id": action_request_id,
        })
        .await?)
}

fn check_fingerprint(
    receipt: AssistantActionReceipt,
    fingerprint: &str,
) -> AppResult<AssistantActionReceipt> {
    if receipt.request_fingerprint != fingerprint {
        return Err(AppError::Conflict(
            "actionRequestId was already used with different request content".to_string(),
        ));
    }
    Ok(receipt)
}

/// Reserve the receipt for `(user_id, action, action_request_id)` or replay
/// the existing one. Fails closed with a conflict when the identity was
/// already used with a different fingerprint; survives insert races through
/// the unique index (duplicate key re-reads the winner).
pub async fn reserve_or_replay(
    db: &mongodb::Database,
    user_id: &str,
    action: &str,
    action_request_id: &str,
    fingerprint: &str,
    resource_id: String,
) -> AppResult<ReceiptOutcome> {
    if let Some(existing) = find_receipt(db, user_id, action, action_request_id).await? {
        return Ok(ReceiptOutcome::Replay(check_fingerprint(
            existing,
            fingerprint,
        )?));
    }

    let receipt = AssistantActionReceipt {
        id: uuid::Uuid::new_v4().to_string(),
        user_id: user_id.to_string(),
        action: action.to_string(),
        action_request_id: action_request_id.to_string(),
        request_fingerprint: fingerprint.to_string(),
        resource_id,
        status: AssistantActionReceiptStatus::Pending,
        created_at: utc_now_at_bson_precision(),
        completed_at: None,
    };
    match db
        .collection::<AssistantActionReceipt>(ASSISTANT_ACTION_RECEIPTS)
        .insert_one(&receipt)
        .await
    {
        Ok(_) => Ok(ReceiptOutcome::Reserved(receipt)),
        Err(error) if duplicate_key(&error) => {
            let winner = find_receipt(db, user_id, action, action_request_id)
                .await?
                .ok_or_else(|| {
                    AppError::Conflict("assistant action receipt reservation raced".to_string())
                })?;
            Ok(ReceiptOutcome::Replay(check_fingerprint(
                winner,
                fingerprint,
            )?))
        }
        Err(error) => Err(error.into()),
    }
}

/// Mark a reserved receipt completed after the effect committed.
pub async fn mark_completed(
    db: &mongodb::Database,
    receipt: &AssistantActionReceipt,
) -> AppResult<()> {
    db.collection::<AssistantActionReceipt>(ASSISTANT_ACTION_RECEIPTS)
        .update_one(
            doc! { "_id": &receipt.id },
            doc! { "$set": {
                "status": "completed",
                "completed_at": bson::DateTime::from_chrono(utc_now_at_bson_precision()),
            } },
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct ExampleFingerprint<'a> {
        action: &'static str,
        key_id: &'a str,
        name: Option<&'a str>,
    }

    #[test]
    fn action_request_id_rules_match_the_wave1_service() {
        assert_eq!(
            normalize_action_request_id("  act-1  ".to_string()).unwrap(),
            "act-1"
        );
        for invalid in [
            "".to_string(),
            "   ".to_string(),
            "has space".to_string(),
            "has/slash".to_string(),
            "has\\backslash".to_string(),
            "has?query".to_string(),
            "has#fragment".to_string(),
            "line\nbreak".to_string(),
            "x".repeat(257),
        ] {
            assert!(
                normalize_action_request_id(invalid.clone()).is_err(),
                "accepted invalid control identity: {invalid:?}"
            );
        }
    }

    #[test]
    fn fingerprints_are_deterministic_and_content_sensitive() {
        let first = fingerprint_canonical(&ExampleFingerprint {
            action: "key.update",
            key_id: "key-1",
            name: Some("renamed"),
        })
        .unwrap();
        let repeat = fingerprint_canonical(&ExampleFingerprint {
            action: "key.update",
            key_id: "key-1",
            name: Some("renamed"),
        })
        .unwrap();
        let different = fingerprint_canonical(&ExampleFingerprint {
            action: "key.update",
            key_id: "key-1",
            name: Some("other"),
        })
        .unwrap();

        assert_eq!(first, repeat);
        assert_ne!(first, different);
        assert_eq!(first.len(), 64, "hex-encoded SHA-256");
    }

    #[test]
    fn identity_reuse_with_different_content_conflicts() {
        let receipt = AssistantActionReceipt {
            id: "receipt-1".to_string(),
            user_id: "user-1".to_string(),
            action: "key.update".to_string(),
            action_request_id: "act-1".to_string(),
            request_fingerprint: "fingerprint-a".to_string(),
            resource_id: "key-1".to_string(),
            status: AssistantActionReceiptStatus::Pending,
            created_at: utc_now_at_bson_precision(),
            completed_at: None,
        };

        assert!(check_fingerprint(receipt.clone(), "fingerprint-a").is_ok());
        assert!(matches!(
            check_fingerprint(receipt, "fingerprint-b"),
            Err(AppError::Conflict(_))
        ));
    }
}
