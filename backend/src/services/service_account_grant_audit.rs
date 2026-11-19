//! Bounded evidence for service-account authority grants.
use crate::{
    errors::AppResult,
    models::{
        security_effect_intent::SecurityEffectIntent,
        service_account_key_read_grant::ServiceAccountKeyReadGrant,
    },
    services::audit_service::{
        AuditActor,
        required::{self, TextField},
    },
};

pub(super) fn curation(
    actor: &AuditActor,
    sa_id: &str,
    grant_id: Option<&str>,
) -> AppResult<SecurityEffectIntent> {
    let mut data = serde_json::json!({"target_sa_id":required::text(TextField::Identifier,sa_id)});
    let event = if let Some(id) = grant_id {
        data["grant_id"] = required::text(TextField::Identifier, id).into();
        "admin.sa.curation_granted"
    } else {
        "admin.sa.curation_revoked"
    };
    required::intent_preserving_actor(actor, event, Some(data))
}

pub(super) fn key_granted(
    actor: &AuditActor,
    grant: &ServiceAccountKeyReadGrant,
) -> AppResult<SecurityEffectIntent> {
    let pairs: Vec<_> = grant
        .targets
        .iter()
        .map(|target| (target.user_service_id.as_str(), target.owner_id.as_str()))
        .collect();
    let mut data = required::identifier_pairs("targets", ["user_service_id", "owner_id"], &pairs);
    data.insert(
        "target_sa_id".into(),
        required::text(TextField::Identifier, &grant.service_account_id).into(),
    );
    data.insert(
        "owner_id".into(),
        required::text(TextField::Identifier, &grant.owner_id).into(),
    );
    data.insert("expires_at".into(), serde_json::json!(grant.expires_at));
    required::intent_preserving_actor(actor, "admin.sa.key_read_granted", Some(data.into()))
}

pub(super) fn key_revoked(
    actor: &AuditActor,
    id: &str,
    previous: &ServiceAccountKeyReadGrant,
) -> AppResult<SecurityEffectIntent> {
    let ids: Vec<_> = previous
        .targets
        .iter()
        .map(|t| t.user_service_id.clone())
        .collect();
    // Keep the old GrantResponse projection, including its RFC3339 formatting.
    let mut prior = required::identifier_list("user_service_ids", &ids);
    prior.insert(
        "service_account_id".into(),
        required::text(TextField::Identifier, &previous.service_account_id).into(),
    );
    prior.insert(
        "issued_by".into(),
        required::text(TextField::Identifier, &previous.issued_by).into(),
    );
    prior.insert("issued_at".into(), previous.issued_at.to_rfc3339().into());
    prior.insert(
        "expires_at".into(),
        serde_json::json!(previous.expires_at.map(|t| t.to_rfc3339())),
    );
    required::intent_preserving_actor(
        actor,
        "admin.sa.key_read_revoked",
        Some(serde_json::json!({
            "target_sa_id":required::text(TextField::Identifier,id),"previous_grant":prior
        })),
    )
}
