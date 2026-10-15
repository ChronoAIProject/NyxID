//! Local policy assessment. Pinned source evidence is not deployment proof.
use super::platform_manager_contract::{ManagerRuntimePolicy, PublicReadiness, UnavailableReason};
use crate::{
    errors::{AppError, AppResult},
    models::platform_public_session::{PublicReadinessRecord, READINESS_RECORDS},
};
use mongodb::{
    Database,
    bson::{self, doc},
};
use sha2::{Digest, Sha256};

pub const UPSTREAM_SOURCE_BUILD: &str = "fe9745799cc1d46f1290e84effb366ff12600577";
pub const PROOF_GAPS: &[&str] = &[
    "deployed_build_and_effective_public_profile_binding",
    "public_purpose_discovery_and_unknown_request_field_denial",
    "nyxid_only_model_routing_including_compaction_and_no_fallback",
    "hard_per_call_and_whole_loop_model_token_cost_bounds",
    "provider_web_search_disabled_and_closed_public_wrappers",
    "absolute_session_expiry_and_cleanup_after_key_revocation",
    "every_replica_enforces_public_purpose_and_live_personal_origin",
    "runtime_acceptance_and_exact_money_admission_recovery",
];

pub(crate) fn unavailable() -> AppError {
    AppError::Conflict("Public runtime unavailable; keep public routing paused".into())
}

pub fn readiness() -> PublicReadiness {
    PublicReadiness::Unavailable(UnavailableReason::PublicNotReady)
}

/// Canonical local candidate identity. This is never a readiness proof digest.
pub fn candidate_digest(policy: &ManagerRuntimePolicy) -> AppResult<String> {
    let selection = policy.public_policy.as_ref().ok_or_else(unavailable)?;
    let encoded = serde_json::to_vec(&serde_json::json!([
        1,
        policy.manager.profile_id,
        policy.manager.manager_bot_id,
        policy.manager.telegram_bot_id,
        policy.manager.manager_authority_epoch,
        policy.public_policy_revision,
        selection.sponsor_owner_user_id,
        selection.nyxagent_profile,
        selection.total_daily_credits.to_string(),
        selection.sender_daily_credits.to_string(),
        UPSTREAM_SOURCE_BUILD,
    ]))
    .map_err(|_| unavailable())?;
    Ok(hex::encode(Sha256::digest(encoded)))
}

/// Metadata-only local assessment. No network request, provider bill or fake
/// Ready value is produced, including when an admin has selected a profile.
pub async fn assess(db: &Database, reviewed: &ManagerRuntimePolicy) -> AppResult<PublicReadiness> {
    if reviewed.public_policy.is_some() {
        let digest = candidate_digest(reviewed)?;
        let record = PublicReadinessRecord {
            id: uuid::Uuid::new_v4().to_string(),
            profile_id: reviewed.manager.profile_id.clone(),
            public_policy_revision: reviewed.public_policy_revision,
            candidate_digest: digest.clone(),
            status: "unavailable".into(),
            proof_gaps: PROOF_GAPS.iter().map(|gap| (*gap).into()).collect(),
            checked_at: chrono::Utc::now(),
        };
        // Keep one assessment per exact candidate; repeating a local preview
        // must not manufacture accumulating evidence or imply a live probe.
        db.collection::<PublicReadinessRecord>(READINESS_RECORDS)
            .update_one(
                doc! {"candidate_digest":digest},
                doc! {"$setOnInsert":bson::to_document(&record).map_err(|_|unavailable())?},
            )
            .upsert(true)
            .await?;
    }
    Ok(readiness())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_review_never_enables_public_runtime() {
        assert!(matches!(
            readiness(),
            PublicReadiness::Unavailable(UnavailableReason::PublicNotReady)
        ));
        assert!(PROOF_GAPS.contains(&"absolute_session_expiry_and_cleanup_after_key_revocation"));
        assert!(PROOF_GAPS.contains(&"hard_per_call_and_whole_loop_model_token_cost_bounds"));
    }
}
