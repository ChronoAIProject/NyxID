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
//! 2. Call [`reserve_or_replay`]. Match every [`ReceiptOutcome`] variant
//!    explicitly — a pending reservation is **not** a replay:
//!    - [`ReceiptOutcome::Replay`]: the receipt is `completed`. Answer from
//!      it and never re-run the effect or return one-time material.
//!    - [`ReceiptOutcome::Reserved`]: this caller owns performing the effect
//!      against `receipt.resource_id`, then [`mark_completed`].
//!    - [`ReceiptOutcome::InProgress`]: a matching pending reservation
//!      already exists. Do **not** treat this as a fresh reservation.
//! 3. For create-shaped verbs pass a freshly minted UUID as the resource
//!    identity; for verbs acting on an existing resource pass that
//!    resource's id, so the replay answer is stable either way.
//!
//! ## Pending-receipt semantics
//!
//! A pending receipt means the identity is reserved but [`mark_completed`]
//! has not run. That covers (a) an in-flight first attempt, (b) a first
//! attempt that committed the effect then failed to mark the receipt, and
//! (c) two concurrent exact requests after the unique-index winner inserted.
//!
//! Chosen contract: **resume idempotently**. The helper cannot know whether
//! the verb already committed, so it returns [`ReceiptOutcome::InProgress`]
//! instead of [`ReceiptOutcome::Replay`] or [`ReceiptOutcome::Reserved`].
//! Callers must either:
//! - inspect resource state, apply the effect only if it has not already
//!   committed, then [`mark_completed`] (so an interrupted-then-retried
//!   delete replays success instead of 404, and an interrupted update does
//!   not advance `state_version` twice), or
//! - return [`in_progress_conflict`] when they cannot prove the effect is
//!   already committed. That 409 is retryable; it is the safe fallback, not
//!   a license to mutate.
//!
//! Matching `Replay(_)` loosely (or folding pending into the mutation
//! branch) is the defect this variant exists to make a compile error.

use mongodb::bson::doc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use utoipa::ToSchema;

use crate::errors::{AppError, AppResult};
use crate::models::assistant_action_receipt::{
    AssistantActionReceipt, AssistantActionReceiptStatus,
    COLLECTION_NAME as ASSISTANT_ACTION_RECEIPTS,
};

const MAX_ACTION_REQUEST_ID_LEN: usize = 256;

/// Whether a response included one-time material that cannot be re-issued on
/// an exact replay.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OneTimeMaterialAvailability {
    #[default]
    Delivered,
    Unavailable,
}

/// Outcome of a receipt reservation attempt.
#[derive(Debug)]
pub enum ReceiptOutcome {
    /// First appearance of this action identity: the caller owns performing
    /// the effect against `receipt.resource_id`, then [`mark_completed`].
    Reserved(AssistantActionReceipt),
    /// Exact retry of a **completed** receipt: the caller must answer from
    /// the receipt without re-running the effect and without returning any
    /// one-time material. Never carries a pending row.
    Replay(AssistantActionReceipt),
    /// Matching fingerprint, but the receipt is still `pending`. The caller
    /// must resume without double-applying or return [`in_progress_conflict`].
    /// Folding this into the mutation branch is the double-effect bug.
    InProgress(AssistantActionReceipt),
}

/// Retryable 409 for an [`ReceiptOutcome::InProgress`] caller that cannot
/// prove the original effect already committed.
pub fn in_progress_conflict() -> AppError {
    AppError::Conflict("this assistant action is already in progress; retry shortly".to_string())
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

/// Domain-separated key for fingerprinting one-time material, initialised at
/// startup alongside the other derived HMAC keys.
static ACTION_FINGERPRINT_HMAC_KEY: std::sync::OnceLock<zeroize::Zeroizing<[u8; 32]>> =
    std::sync::OnceLock::new();

pub fn init_action_fingerprint_hmac_key(key: zeroize::Zeroizing<[u8; 32]>) {
    if ACTION_FINGERPRINT_HMAC_KEY.set(key).is_err() {
        tracing::warn!("assistant action fingerprint HMAC key was already initialized");
    }
}

/// Fingerprint caller-supplied material that may be low-entropy.
///
/// A receipt fingerprint is a plain SHA-256 over the canonical request, which
/// is correct for identifiers but wrong for material a user chooses: receipts
/// are stored, so an unkeyed digest of a weak value is an offline
/// brute-force oracle against a database snapshot. This keys the digest with
/// a domain-separated HMAC so a snapshot alone proves nothing -- the same
/// threat model the project already applies to CLI pairing codes.
///
/// Falls back to the unkeyed digest only when the key was never initialised
/// (tests and tooling that never see real material), and says so in the
/// prefix so the two can never silently compare equal.
pub fn fingerprint_sensitive_material(material: &str) -> String {
    match ACTION_FINGERPRINT_HMAC_KEY.get() {
        Some(key) => {
            use hmac::{Hmac, Mac};
            let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key.as_ref())
                .expect("HMAC accepts a 32-byte key");
            mac.update(material.as_bytes());
            format!("hmac-sha256:{}", hex::encode(mac.finalize().into_bytes()))
        }
        None => {
            let mut hasher = Sha256::new();
            hasher.update(material.as_bytes());
            format!("unkeyed-sha256:{}", hex::encode(hasher.finalize()))
        }
    }
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

fn existing_outcome(
    receipt: AssistantActionReceipt,
    fingerprint: &str,
) -> AppResult<ReceiptOutcome> {
    let receipt = check_fingerprint(receipt, fingerprint)?;
    match receipt.status {
        AssistantActionReceiptStatus::Completed => Ok(ReceiptOutcome::Replay(receipt)),
        AssistantActionReceiptStatus::Pending => Ok(ReceiptOutcome::InProgress(receipt)),
    }
}

/// Reserve the receipt for `(user_id, action, action_request_id)` or return
/// the existing one. Completed matching receipts are [`ReceiptOutcome::Replay`];
/// pending matching receipts are [`ReceiptOutcome::InProgress`] — never
/// [`ReceiptOutcome::Replay`], so a caller cannot fall through to mutation by
/// matching loosely. Fails closed with a conflict when the identity was
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
    reserve_or_replay_with_markers(
        db,
        user_id,
        action,
        action_request_id,
        fingerprint,
        resource_id,
        None,
        None,
    )
    .await
}

/// Reserve a receipt while recording optional secret-free monotonic markers
/// captured by the caller immediately before the effect.
#[allow(clippy::too_many_arguments)]
pub async fn reserve_or_replay_with_markers(
    db: &mongodb::Database,
    user_id: &str,
    action: &str,
    action_request_id: &str,
    fingerprint: &str,
    resource_id: String,
    resource_state_version: Option<i64>,
    resource_access_revision: Option<i64>,
) -> AppResult<ReceiptOutcome> {
    reserve_or_replay_with_all_markers(
        db,
        user_id,
        action,
        action_request_id,
        fingerprint,
        resource_id,
        resource_state_version,
        resource_access_revision,
        None,
    )
    .await
}

/// Reserve a receipt while recording a keyed fingerprint of secret material
/// captured immediately before a rotation.
#[allow(clippy::too_many_arguments)]
pub async fn reserve_or_replay_with_secret_marker(
    db: &mongodb::Database,
    user_id: &str,
    action: &str,
    action_request_id: &str,
    fingerprint: &str,
    resource_id: String,
    resource_material_fingerprint: Option<String>,
) -> AppResult<ReceiptOutcome> {
    reserve_or_replay_with_all_markers(
        db,
        user_id,
        action,
        action_request_id,
        fingerprint,
        resource_id,
        None,
        None,
        resource_material_fingerprint,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn reserve_or_replay_with_all_markers(
    db: &mongodb::Database,
    user_id: &str,
    action: &str,
    action_request_id: &str,
    fingerprint: &str,
    resource_id: String,
    resource_state_version: Option<i64>,
    resource_access_revision: Option<i64>,
    resource_material_fingerprint: Option<String>,
) -> AppResult<ReceiptOutcome> {
    if let Some(existing) = find_receipt(db, user_id, action, action_request_id).await? {
        return existing_outcome(existing, fingerprint);
    }

    let receipt = AssistantActionReceipt {
        id: uuid::Uuid::new_v4().to_string(),
        user_id: user_id.to_string(),
        action: action.to_string(),
        action_request_id: action_request_id.to_string(),
        request_fingerprint: fingerprint.to_string(),
        resource_id,
        resource_state_version,
        resource_access_revision,
        resource_material_fingerprint,
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
            existing_outcome(winner, fingerprint)
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

    /// Receipts are stored, so an unkeyed digest of caller-supplied material
    /// is an offline brute-force oracle against a database snapshot. The
    /// fingerprint must therefore never be a bare digest of the material.
    ///
    /// Falsifier: revert `fingerprint_sensitive_material` to a plain SHA-256
    /// (or leave the key uninitialised, which takes the labelled fallback
    /// branch) and this fails -- the output ends with the bare hex digest.
    #[test]
    fn sensitive_material_fingerprint_is_keyed_not_a_bare_digest() {
        // Idempotent: another test in this process may have set it already.
        let _ = ACTION_FINGERPRINT_HMAC_KEY.set(zeroize::Zeroizing::new([7u8; 32]));

        let material = "low-entropy-value";
        let fingerprint = fingerprint_sensitive_material(material);

        let mut hasher = Sha256::new();
        hasher.update(material.as_bytes());
        let bare = hex::encode(hasher.finalize());

        assert!(
            !fingerprint.ends_with(&bare),
            "fingerprint must not be a bare digest of the material: {fingerprint}"
        );
        assert!(fingerprint.starts_with("hmac-sha256:"), "{fingerprint}");
        assert_eq!(
            fingerprint,
            fingerprint_sensitive_material(material),
            "fingerprints must stay deterministic or replay detection breaks"
        );
    }

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
            resource_state_version: None,
            resource_access_revision: None,
            resource_material_fingerprint: None,
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

    #[test]
    fn pending_matching_receipt_is_in_progress_not_replay() {
        let pending = AssistantActionReceipt {
            id: "receipt-pending".to_string(),
            user_id: "user-1".to_string(),
            action: "key.update".to_string(),
            action_request_id: "act-1".to_string(),
            request_fingerprint: "fingerprint-a".to_string(),
            resource_id: "key-1".to_string(),
            resource_state_version: None,
            resource_access_revision: None,
            resource_material_fingerprint: None,
            status: AssistantActionReceiptStatus::Pending,
            created_at: utc_now_at_bson_precision(),
            completed_at: None,
        };
        assert!(matches!(
            existing_outcome(pending, "fingerprint-a"),
            Ok(ReceiptOutcome::InProgress(_))
        ));
    }

    #[test]
    fn completed_matching_receipt_is_replay() {
        let completed = AssistantActionReceipt {
            id: "receipt-done".to_string(),
            user_id: "user-1".to_string(),
            action: "key.update".to_string(),
            action_request_id: "act-1".to_string(),
            request_fingerprint: "fingerprint-a".to_string(),
            resource_id: "key-1".to_string(),
            resource_state_version: None,
            resource_access_revision: None,
            resource_material_fingerprint: None,
            status: AssistantActionReceiptStatus::Completed,
            created_at: utc_now_at_bson_precision(),
            completed_at: Some(utc_now_at_bson_precision()),
        };
        assert!(matches!(
            existing_outcome(completed, "fingerprint-a"),
            Ok(ReceiptOutcome::Replay(_))
        ));
    }
}
