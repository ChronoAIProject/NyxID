//! Bounded audit facts for owner key-authority mutations.
use crate::{
    errors::AppResult,
    models::{
        api_key_credential::{ApiKeyCredential, CredentialRevokedReason},
        security_effect_intent::SecurityEffectIntent,
    },
    services::audit_service::{
        AuditActor,
        required::{self, TextField},
    },
};

fn id(value: &str) -> String {
    required::text(TextField::Identifier, value)
}

fn owner_actor(owner: &str) -> AuditActor {
    AuditActor {
        user_id: owner.into(),
        ip_address: None,
        user_agent: None,
        api_key_id: None,
        api_key_name: None,
    }
}

pub(crate) fn child_revoked(
    row: &ApiKeyCredential,
    reason: CredentialRevokedReason,
) -> AppResult<SecurityEffectIntent> {
    required::intent(
        Some(&owner_actor(&row.user_id)),
        "agent_key_credential_revoked",
        Some(serde_json::json!({
            "api_key_id": id(&row.api_key_id), "credential_id": id(&row.id), "reason": reason,
        })),
    )
}

pub(crate) fn operations_changed(
    actor: &str,
    owner: &str,
    key: &str,
    service: &str,
    revision: i64,
    operation_count: Option<usize>,
) -> AppResult<SecurityEffectIntent> {
    required::intent(
        Some(&owner_actor(actor)),
        "api_key_operations_changed",
        Some(serde_json::json!({
            "owner_id": id(owner), "api_key_id": id(key), "service_id": id(service), "revision": revision, "operation_count": operation_count,
        })),
    )
}

pub(crate) fn grants_provisioned(
    actor: &AuditActor,
    owner: &str,
    key: &str,
    grant_ids: &[String],
) -> AppResult<SecurityEffectIntent> {
    let mut data = required::identifier_list("grant_ids", grant_ids);
    data.extend(
        serde_json::json!({
            "api_key_id": id(key),
            "operation_count": grant_ids.len(),
            "owner_user_id": id(owner),
            "decision": "authorized",
        })
        .as_object()
        .unwrap()
        .clone(),
    );
    required::intent_preserving_actor(actor, "durable_grants_provisioned", Some(data.into()))
}

pub(crate) fn grant_revoked(
    actor: &AuditActor,
    key: &str,
    grant: &str,
    endpoint: &str,
    contract_digest: &str,
) -> AppResult<SecurityEffectIntent> {
    grant_revoked_with_reason(actor, key, grant, endpoint, contract_digest, None)
}

pub(crate) fn grant_revoked_for_reauthorization(
    actor: &AuditActor,
    key: &str,
    grant: &str,
    endpoint: &str,
    contract_digest: &str,
) -> AppResult<SecurityEffectIntent> {
    grant_revoked_with_reason(
        actor,
        key,
        grant,
        endpoint,
        contract_digest,
        Some("reauthorization"),
    )
}

fn grant_revoked_with_reason(
    actor: &AuditActor,
    key: &str,
    grant: &str,
    endpoint: &str,
    contract_digest: &str,
    reason: Option<&'static str>,
) -> AppResult<SecurityEffectIntent> {
    let mut data = serde_json::json!({
        "api_key_id": id(key), "grant_id": id(grant), "endpoint_id": id(endpoint),
        "contract_digest": contract_digest, "decision": "revoked",
    });
    if let Some(reason) = reason {
        data["reason"] = reason.into();
    }
    required::intent_preserving_actor(actor, "durable_grant_revoked", Some(data))
}

pub(crate) fn grants_reauthorized(
    actor: &AuditActor,
    key: &str,
    grant_ids: &[String],
) -> AppResult<SecurityEffectIntent> {
    let mut data = required::identifier_list("grant_ids", grant_ids);
    data.extend(
        serde_json::json!({
            "api_key_id": id(key), "operation_count": grant_ids.len(), "decision": "authorized",
        })
        .as_object()
        .unwrap()
        .clone(),
    );
    required::intent_preserving_actor(actor, "durable_grants_reauthorized", Some(data.into()))
}

pub(crate) fn permission_created(
    actor: &AuditActor,
    key: &str,
    service: &str,
) -> AppResult<SecurityEffectIntent> {
    required::intent_preserving_actor(
        actor,
        "permission_key_created",
        Some(serde_json::json!({"permission_key_id":id(key),"user_service_id":id(service)})),
    )
}

pub(crate) fn permission_paused(
    actor: &AuditActor,
    key: &str,
    paused: bool,
    revision: i64,
) -> AppResult<SecurityEffectIntent> {
    required::intent_preserving_actor(
        actor,
        "permission_key_pause_changed",
        Some(serde_json::json!({"permission_key_id":id(key),"paused":paused,"revision":revision})),
    )
}

pub(crate) fn permission_revoked(actor: &AuditActor, key: &str) -> AppResult<SecurityEffectIntent> {
    required::intent_preserving_actor(
        actor,
        "permission_key_revoked",
        Some(serde_json::json!({"permission_key_id":id(key)})),
    )
}
