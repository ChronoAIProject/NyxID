use crate::{
    errors::AppResult,
    models::{
        agent_service_binding::AgentServiceBinding, security_effect_intent::SecurityEffectIntent,
    },
    services::audit_service::{
        AuditActor,
        required::{self, TextField},
    },
};

pub(crate) fn created(
    actor: &AuditActor,
    binding: &AgentServiceBinding,
) -> AppResult<SecurityEffectIntent> {
    required::intent_preserving_actor(
        actor,
        "agent_binding_created",
        Some(serde_json::json!({
            "binding_id": required::text(TextField::Identifier, &binding.id),
            "api_key_id": required::text(TextField::Identifier, &binding.api_key_id),
            "user_service_id": required::text(TextField::Identifier, &binding.user_service_id),
            "user_api_key_id": required::text(TextField::Identifier, &binding.user_api_key_id),
        })),
    )
}

pub(crate) fn deleted(
    actor: &AuditActor,
    key_id: &str,
    binding_id: &str,
) -> AppResult<SecurityEffectIntent> {
    required::intent_preserving_actor(
        actor,
        "agent_binding_deleted",
        Some(serde_json::json!({
            "binding_id": required::text(TextField::Identifier, binding_id),
            "api_key_id": required::text(TextField::Identifier, key_id),
        })),
    )
}
