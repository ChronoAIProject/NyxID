//! Bounded, explicit account export. Model structs are never serialized here.
use crate::{
    errors::{AppError, AppResult},
    services::personal_data_registry,
};
use bson::{Bson, Document, doc};
use chrono::Utc;
use futures::TryStreamExt;
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

pub const FORMAT: &str = "nyxid.account-export";
pub const VERSION: u32 = 1;
pub const MAX_BYTES: usize = 32 * 1024 * 1024;
pub const SUBSECTION_ROW_CAP: i64 = 10_000;
pub const QUERY_TIMEOUT: Duration = Duration::from_secs(3);
pub const FILE_PAYLOADS_METADATA_ONLY: bool = true;

#[derive(Debug, thiserror::Error)]
#[error("no account-export allowlist for collection {0}")]
pub struct ExportSpecError(pub String);

#[derive(Debug, Serialize, Clone, Default)]
pub struct ExportSubsection {
    pub items: Vec<serde_json::Value>,
    pub total_count: u64,
    pub truncated: bool,
}
#[derive(Debug, Serialize, Clone)]
pub struct ExportLimits {
    pub max_bytes: usize,
    pub subsection_row_cap: i64,
    pub query_timeout_ms: u64,
    pub file_payloads: &'static str,
}
#[derive(Debug, Serialize, Clone)]
pub struct AccountExport {
    pub format: &'static str,
    pub version: u32,
    pub generated_at: String,
    pub account_id: String,
    pub limits: ExportLimits,
    pub truncated_sections: Vec<String>,
    pub excluded: Vec<&'static str>,
    pub sections: BTreeMap<String, BTreeMap<String, ExportSubsection>>,
}

fn bson_json(value: &Bson) -> Option<serde_json::Value> {
    Some(match value {
        Bson::String(value) => serde_json::Value::String(value.clone()),
        Bson::Boolean(value) => serde_json::Value::Bool(*value),
        Bson::Int32(value) => serde_json::json!(*value),
        Bson::Int64(value) => serde_json::json!(*value),
        Bson::Double(value) if value.is_finite() => serde_json::json!(*value),
        Bson::DateTime(value) => serde_json::Value::String(value.to_chrono().to_rfc3339()),
        Bson::ObjectId(value) => serde_json::Value::String(value.to_hex()),
        Bson::Decimal128(value) => serde_json::Value::String(value.to_string()),
        Bson::Null => serde_json::Value::Null,
        Bson::Document(value) => serde_json::Value::Object(
            value
                .iter()
                .filter(|(key, _)| !forbidden_key(key))
                .filter_map(|(key, value)| bson_json(value).map(|value| (key.clone(), value)))
                .collect(),
        ),
        Bson::Array(values) => {
            serde_json::Value::Array(values.iter().filter_map(bson_json).collect())
        }
        Bson::Binary(_) => return None,
        _ => return None,
    })
}

/// URLs are exported as origins plus a safe path. Credentials, query values,
/// and fragments are always removed; fields whose providers commonly put
/// tokens in path segments opt into `drop_path` at the allowlist boundary.
fn sanitize_url(value: &str, drop_path: bool) -> String {
    let Ok(mut parsed) = url::Url::parse(value) else {
        return value.to_owned();
    };
    let _ = parsed.set_username("");
    let _ = parsed.set_password(None);
    parsed.set_query(None);
    parsed.set_fragment(None);
    if drop_path {
        parsed.set_path("");
    }
    parsed.to_string()
}

fn is_url_field(field: &str) -> bool {
    matches!(
        field,
        "avatar_url"
            | "url"
            | "base_url"
            | "endpoint"
            | "openapi_spec_url"
            | "target_url"
            | "project_url"
            | "redirect_uris"
            | "delivery.url"
            | "schedule.url"
            | "webhook_url"
            | "connection_webhook_url"
            | "revocation_webhook_url"
    )
}

fn url_path_is_opaque(field: &str) -> bool {
    matches!(
        field,
        "avatar_url"
            | "redirect_uris"
            | "target_url"
            | "project_url"
            | "delivery.url"
            | "schedule.url"
            | "webhook_url"
            | "connection_webhook_url"
            | "revocation_webhook_url"
    )
}

fn forbidden_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.contains("hash")
        || key.contains("hmac")
        || key.contains("encrypted")
        || key.contains("ciphertext")
        || key.contains("secret")
        || key.contains("password")
        || key.contains("private")
        || key.contains("token_value")
        || key == "token"
        || key.ends_with("_token")
        || key.contains("access_token")
        || key.contains("refresh_token")
        || key.contains("raw_token")
        || key.contains("credential")
        || key.contains("oauth")
        || key.contains("authorization_code")
        || key.contains("api_key")
        || key.contains("webhook")
        || key.contains("key_prefix")
        || key == "kid"
        || key == "key_id"
        || key.contains("wrapped")
        || key == "dek"
        || key.contains("nonce")
        || key.contains("signature")
}
fn get_path(row: &Document, path: &str) -> Option<Bson> {
    fn descend(value: &Bson, segments: &[&str]) -> Option<Bson> {
        let Some(segment) = segments.first() else {
            return Some(value.clone());
        };
        match value {
            Bson::Document(document) => descend(document.get(*segment)?, &segments[1..]),
            Bson::Array(values) => Some(Bson::Array(
                values
                    .iter()
                    .filter_map(|value| descend(value, segments))
                    .collect(),
            )),
            _ => None,
        }
    }
    let segments = path.split('.').collect::<Vec<_>>();
    descend(row.get(segments.first().copied()?)?, &segments[1..])
}

fn insert_path(
    target: &mut serde_json::Map<String, serde_json::Value>,
    path: &str,
    value: serde_json::Value,
) {
    let mut segments = path.split('.').peekable();
    let Some(first) = segments.next() else {
        return;
    };
    if let Some(next) = segments.next() {
        let entry = target
            .entry(first.to_owned())
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
        let Some(object) = entry.as_object_mut() else {
            return;
        };
        let remaining = std::iter::once(next)
            .chain(segments)
            .collect::<Vec<_>>()
            .join(".");
        insert_path(object, &remaining, value);
    } else {
        target.insert(first.to_owned(), value);
    }
}

fn sanitize_json_url(value: serde_json::Value, field: &str) -> serde_json::Value {
    match value {
        serde_json::Value::String(value) if is_url_field(field) => {
            serde_json::Value::String(sanitize_url(&value, url_path_is_opaque(field)))
        }
        serde_json::Value::Array(values) if is_url_field(field) => serde_json::Value::Array(
            values
                .into_iter()
                .map(|value| sanitize_json_url(value, field))
                .collect(),
        ),
        other => other,
    }
}

fn safe_item(row: &Document, fields: &[&str]) -> serde_json::Value {
    let mut result = serde_json::Map::new();
    for field in fields {
        if *field == "_id" || forbidden_key(field) {
            continue;
        }
        if let Some(value) = get_path(row, field).as_ref().and_then(bson_json) {
            insert_path(&mut result, field, sanitize_json_url(value, field));
        }
    }
    serde_json::Value::Object(result)
}

/// One registry `Export::Section` maps to exactly one explicit subsection.
fn export_spec(collection: &str) -> Option<(&'static str, &'static str)> {
    match collection {
        "agent_key_login_requests" => Some(("account", "agent_key_login_requests")),
        "agent_service_bindings" => Some(("account", "agent_service_bindings")),
        "billing_wallet" => Some(("billing", "wallet")),
        "api_key_credentials" => Some(("services", "api_key_credentials")),
        "api_keys" => Some(("services", "api_keys")),
        "approval_grants" => Some(("account", "approval_grants")),
        "approval_requests" => Some(("account", "approval_requests")),
        "assistant_acknowledgements" => Some(("assistant", "assistant_acknowledgements")),
        "assistant_action_receipts" => Some(("account_activity", "assistant_action_receipts")),
        "assistant_agent_consents" => Some(("assistant", "assistant_agent_consents")),
        "assistant_agent_credentials" => Some(("assistant", "assistant_agent_credentials")),
        "assistant_agent_learning" => Some(("assistant", "assistant_agent_learning")),
        "assistant_agent_learning_members" => {
            Some(("assistant", "assistant_agent_learning_members"))
        }
        "assistant_agent_learning_proposals" => {
            Some(("assistant", "assistant_agent_learning_proposals"))
        }
        "assistant_agent_learning_runs" => Some(("assistant", "assistant_agent_learning_runs")),
        "assistant_agent_learning_skill_roots" => {
            Some(("assistant", "assistant_agent_learning_skill_roots"))
        }
        "assistant_agent_private_notes" => Some(("assistant", "assistant_agent_private_notes")),
        "assistant_agent_shares" => Some(("assistant", "assistant_agent_shares")),
        "assistant_agent_sharing" => Some(("assistant", "assistant_agent_sharing")),
        "assistant_agents" => Some(("assistant", "assistant_agents")),
        "assistant_artifact_deliveries" => Some(("assistant", "assistant_artifact_deliveries")),
        "assistant_async_operations" => Some(("assistant", "assistant_async_operations")),
        "assistant_async_quotas" => Some(("assistant", "assistant_async_quotas")),
        "assistant_attachment_expiry" => Some(("assistant", "assistant_attachment_expiry")),
        "assistant_attachments" => Some(("assistant", "assistant_attachments")),
        "assistant_authored_draft_claims" => Some(("assistant", "assistant_authored_draft_claims")),
        "assistant_checkpoints" => Some(("assistant", "assistant_checkpoints")),
        "assistant_cma_agents" => Some(("assistant", "assistant_cma_agents")),
        "assistant_cma_creations" => Some(("assistant", "assistant_cma_creations")),
        "assistant_cma_sandboxes" => Some(("assistant", "assistant_cma_sandboxes")),
        "assistant_cma_session_origins" => {
            Some(("account_activity", "assistant_cma_session_origins"))
        }
        "assistant_conversations" => Some(("assistant", "assistant_conversations")),
        "assistant_direct_handoffs" => Some(("assistant", "assistant_direct_handoffs")),
        "assistant_group_agent_access" => {
            Some(("account_activity", "assistant_group_agent_access"))
        }
        "assistant_group_agent_proposals" => Some(("assistant", "assistant_group_agent_proposals")),
        "assistant_group_execution_slots" => {
            Some(("account_activity", "assistant_group_execution_slots"))
        }
        "assistant_group_invites" => Some(("assistant", "assistant_group_invites")),
        "assistant_group_mention_notifications" => {
            Some(("assistant", "assistant_group_mention_notifications"))
        }
        "assistant_group_messages" => Some(("assistant", "assistant_group_messages")),
        "assistant_group_read_states" => Some(("assistant", "assistant_group_read_states")),
        "assistant_group_requests" => Some(("assistant", "assistant_group_requests")),
        "assistant_group_transfers" => Some(("account_activity", "assistant_group_transfers")),
        "assistant_groups" => Some(("assistant", "assistant_groups")),
        "assistant_invocation_turns" => Some(("assistant", "assistant_invocation_turns")),
        "assistant_invocations" => Some(("assistant", "assistant_invocations")),
        "assistant_learning_migrations" => {
            Some(("account_activity", "assistant_learning_migrations"))
        }
        "assistant_learning_publication_targets" => {
            Some(("account_activity", "assistant_learning_publication_targets"))
        }
        "assistant_message_feedback" => Some(("assistant", "assistant_message_feedback")),
        "assistant_messages" => Some(("assistant", "assistant_messages")),
        "assistant_notification_push_budgets" => {
            Some(("account", "assistant_notification_push_budgets"))
        }
        "assistant_notification_push_sources" => {
            Some(("account", "assistant_notification_push_sources"))
        }
        "assistant_notifications" => Some(("assistant", "assistant_notifications")),
        "assistant_onboarding" => Some(("assistant", "assistant_onboarding")),
        "assistant_resources" => Some(("assistant", "assistant_resources")),
        "assistant_service_execution" => Some(("assistant", "assistant_service_execution")),
        "assistant_settings" => Some(("assistant", "assistant_settings")),
        "assistant_skill_draft_attempts" => Some(("assistant", "assistant_skill_draft_attempts")),
        "assistant_tool_results" => Some(("assistant", "assistant_tool_results")),
        "assistant_voice_announcements" => Some(("assistant", "assistant_voice_announcements")),
        "assistant_voice_requests" => Some(("assistant", "assistant_voice_requests")),
        "assistant_voice_windows" => Some(("account_activity", "assistant_voice_windows")),
        "assistant_wire_logs" => Some(("account_activity", "assistant_wire_logs")),
        "async_service_operation_artifacts" => {
            Some(("assistant", "async_service_operation_artifacts"))
        }
        "auth_device_codes" => Some(("account", "auth_device_codes")),
        "auth_device_codes_v2" => Some(("account", "auth_device_codes_v2")),
        "authorization_codes" => Some(("account", "authorization_codes")),
        "catalog_delegation_grants" => Some(("services", "catalog_delegation_grants")),
        "channel_activity_notifications" => Some(("channels", "channel_activity_notifications")),
        "channel_adapter_accounts" => Some(("account_activity", "channel_adapter_accounts")),
        "channel_adapter_allocations" => Some(("channels", "channel_adapter_allocations")),
        "channel_adapter_bindings" => Some(("channels", "channel_adapter_bindings")),
        "channel_adapter_events" => Some(("channels", "channel_adapter_events")),
        "channel_adapter_resource_claims" => Some(("channels", "channel_adapter_resource_claims")),
        "channel_adapter_sessions" => Some(("channels", "channel_adapter_sessions")),
        "channel_bots" => Some(("channels", "channel_bots")),
        "channel_connect_links" => Some(("channels", "channel_connect_links")),
        "channel_conversations" => Some(("channels", "channel_conversations")),
        "channel_delivery_receipts" => Some(("channels", "channel_delivery_receipts")),
        "channel_email_batches" => Some(("channels", "channel_email_batches")),
        "channel_email_receipts" => Some(("channels", "channel_email_receipts")),
        "channel_email_sends" => Some(("channels", "channel_email_sends")),
        "channel_email_subscriptions" => Some(("channels", "channel_email_subscriptions")),
        "channel_messages" => Some(("channels", "channel_messages")),
        "channel_reply_deliveries" => Some(("channels", "channel_reply_deliveries")),
        "channel_text_deliveries" => Some(("channels", "channel_text_deliveries")),
        "channel_webhook_rejections" => Some(("channels", "channel_webhook_rejections")),
        "channel_whatsapp_native" => Some(("channels", "channel_whatsapp_native")),
        "cli_pairings" => Some(("account", "cli_pairings")),
        "connect_links" => Some(("account", "connect_links")),
        "consents" => Some(("account", "consents")),
        "credit_grants" => Some(("billing", "credit_grants")),
        "device_codes" => Some(("security", "device_codes")),
        "device_onboard_credentials" => Some(("account", "device_onboard_credentials")),
        "encrypted_record_migration_findings" => {
            Some(("account_activity", "encrypted_record_migration_findings"))
        }
        "encrypted_record_migration_runs" => {
            Some(("account_activity", "encrypted_record_migration_runs"))
        }
        "execution_permission_deliveries" => Some(("account", "execution_permission_deliveries")),
        "feature_flag_overrides" => Some(("account_activity", "feature_flag_overrides")),
        "generated_artifact_content_series" => {
            Some(("account", "generated_artifact_content_series"))
        }
        "invite_codes" => Some(("account_activity", "invite_codes")),
        "invocation_diagnostics" => Some(("account_activity", "invocation_diagnostics")),
        "lark_base_write_photos" => Some(("account", "lark_base_write_photos")),
        "lark_base_writes" => Some(("account", "lark_base_writes")),
        "login_approvals" => Some(("security", "login_approvals")),
        "login_codes" => Some(("security", "login_codes")),
        "machine_authority_leases" => Some(("machines", "machine_authority_leases")),
        "machine_confirmations" => Some(("machines", "machine_confirmations")),
        "machine_contexts" => Some(("machines", "machine_contexts")),
        "machine_desktops" => Some(("machines", "machine_desktops")),
        "machine_external_grants" => Some(("machines", "machine_external_grants")),
        "machine_free_slots" => Some(("machines", "machine_free_slots")),
        "machine_jobs" => Some(("machines", "machine_jobs")),
        "machine_oauth_consent_requests" => Some(("machines", "machine_oauth_consent_requests")),
        "machine_orphan_inventory" => Some(("machines", "machine_orphan_inventory")),
        "machine_revocations" => Some(("machines", "machine_revocations")),
        "machine_separation_compatibility" => {
            Some(("machines", "machine_separation_compatibility"))
        }
        "machine_setups" => Some(("machines", "machine_setups")),
        "machine_update_attempts" => Some(("machines", "machine_update_attempts")),
        "machine_updates" => Some(("machines", "machine_updates")),
        "mcp_session_notifications" => Some(("account_activity", "mcp_session_notifications")),
        "mcp_session_slots" => Some(("account", "mcp_session_slots")),
        "mfa_challenges" => Some(("account", "mfa_challenges")),
        "mfa_factors" => Some(("security", "mfa_factors")),
        "node_registration_tokens" => Some(("account", "node_registration_tokens")),
        "nodes" => Some(("machines", "nodes")),
        "notification_channels" => Some(("security", "notification_channels")),
        "nyxbot_channels" => Some(("channels", "nyxbot_channels")),
        "nyxbot_event_bindings" => Some(("assistant", "nyxbot_event_bindings")),
        "nyxbot_events" => Some(("assistant", "nyxbot_events")),
        "nyxbot_gateway_responses" => Some(("assistant", "nyxbot_gateway_responses")),
        "nyxbot_image_deliveries" => Some(("assistant", "nyxbot_image_deliveries")),
        "nyxbot_image_deliveries_v2" => Some(("assistant", "nyxbot_image_deliveries_v2")),
        "nyxbot_threads" => Some(("assistant", "nyxbot_threads")),
        "nyxbot_watches" => Some(("assistant", "nyxbot_watches")),
        "oauth_broker_bindings" => Some(("services", "oauth_broker_bindings")),
        "oauth_clients" => Some(("services", "oauth_clients")),
        "oauth_consent_requests" => Some(("services", "oauth_consent_requests")),
        "oauth_delegation_authorities" => Some(("services", "oauth_delegation_authorities")),
        "oauth_states" => Some(("services", "oauth_states")),
        "openclaw_channel_mappings" => Some(("account_activity", "openclaw_channel_mappings")),
        "oracle_pools" => Some(("account", "oracle_pools")),
        "org_invites" => Some(("account", "org_invites")),
        "org_memberships" => Some(("account", "org_memberships")),
        "permission_key_policies" => Some(("account", "permission_key_policies")),
        "platform_bot_cleanup" => Some(("account", "platform_bot_cleanup")),
        "platform_bot_configs" => Some(("account", "platform_bot_configs")),
        "platform_bot_installations" => Some(("account", "platform_bot_installations")),
        "platform_bot_runtime_controls" => Some(("account", "platform_bot_runtime_controls")),
        "platform_delivery_tests" => Some(("account", "platform_delivery_tests")),
        "platform_discord_control_installation_history" => {
            Some(("account", "platform_discord_control_installation_history"))
        }
        "platform_discord_control_installations" => {
            Some(("account", "platform_discord_control_installations"))
        }
        "platform_guide_selections" => Some(("account_activity", "platform_guide_selections")),
        "platform_native_entry_receipts" => Some(("account", "platform_native_entry_receipts")),
        "platform_native_replies" => Some(("account_activity", "platform_native_replies")),
        "platform_onboarding_binding_requests" => {
            Some(("account_activity", "platform_onboarding_binding_requests"))
        }
        "platform_onboarding_bindings" => {
            Some(("account_activity", "platform_onboarding_bindings"))
        }
        "platform_personal_decision_spacing" => {
            Some(("account_activity", "platform_personal_decision_spacing"))
        }
        "platform_personal_setup_selections" => {
            Some(("account_activity", "platform_personal_setup_selections"))
        }
        "platform_personal_stop_effects" => Some(("account", "platform_personal_stop_effects")),
        "platform_public_deployment_activations" => {
            Some(("account", "platform_public_deployment_activations"))
        }
        "platform_public_deployment_proofs" => {
            Some(("account", "platform_public_deployment_proofs"))
        }
        "platform_public_probe_consents" => Some(("account", "platform_public_probe_consents")),
        "platform_public_readiness_probes" => Some(("account", "platform_public_readiness_probes")),
        "platform_setup_notices" => Some(("account", "platform_setup_notices")),
        "platform_setup_steps" => Some(("account", "platform_setup_steps")),
        "platform_setups" => Some(("account", "platform_setups")),
        "refresh_tokens" => Some(("security", "refresh_tokens")),
        "saved_logins" => Some(("account", "saved_logins")),
        "service_account_key_read_grants" => Some(("services", "service_account_key_read_grants")),
        "service_account_tokens" => Some(("services", "service_account_tokens")),
        "service_accounts" => Some(("services", "service_accounts")),
        "service_approval_configs" => Some(("services", "service_approval_configs")),
        "service_change_events" => Some(("account_activity", "service_change_events")),
        "service_pool_member_health" => Some(("services", "service_pool_member_health")),
        "service_pools" => Some(("services", "service_pools")),
        "service_preferences" => Some(("services", "service_preferences")),
        "sessions" => Some(("security", "sessions")),
        "telegram_bot_claims" => Some(("account_activity", "telegram_bot_claims")),
        "telegram_bot_requests" => Some(("account_activity", "telegram_bot_requests")),
        "telegram_managed_bot_events" => Some(("account_activity", "telegram_managed_bot_events")),
        "trigger_deliveries" => Some(("account", "trigger_deliveries")),
        "trigger_owner_limits" => Some(("account", "trigger_owner_limits")),
        "trigger_runs" => Some(("account", "trigger_runs")),
        "trigger_work" => Some(("account", "trigger_work")),
        "triggers" => Some(("account", "triggers")),
        "usage_workspaces" => Some(("billing", "usage_workspaces")),
        "usage_meter" => Some(("billing", "usage_meter")),
        "user_api_keys" => Some(("services", "user_api_keys")),
        "user_endpoints" => Some(("services", "user_endpoints")),
        "user_provider_credentials" => Some(("services", "user_provider_credentials")),
        "user_provider_tokens" => Some(("services", "user_provider_tokens")),
        "user_service_connections" => Some(("services", "user_service_connections")),
        "user_services" => Some(("services", "user_services")),
        "assistant_agent_learning_members" => {
            Some(("assistant", "assistant_agent_learning_members"))
        }
        "assistant_agent_learning_proposals" => {
            Some(("assistant", "assistant_agent_learning_proposals"))
        }
        "assistant_agent_learning_skill_roots" => {
            Some(("assistant", "assistant_agent_learning_skill_roots"))
        }
        "assistant_cma_session_origins" => {
            Some(("account_activity", "assistant_cma_session_origins"))
        }
        "assistant_group_agent_access" => {
            Some(("account_activity", "assistant_group_agent_access"))
        }
        "assistant_group_execution_slots" => {
            Some(("account_activity", "assistant_group_execution_slots"))
        }
        "assistant_group_mention_notifications" => {
            Some(("assistant", "assistant_group_mention_notifications"))
        }
        "assistant_learning_migrations" => {
            Some(("account_activity", "assistant_learning_migrations"))
        }
        "assistant_learning_publication_targets" => {
            Some(("account_activity", "assistant_learning_publication_targets"))
        }
        "assistant_notification_push_budgets" => {
            Some(("account", "assistant_notification_push_budgets"))
        }
        "assistant_notification_push_sources" => {
            Some(("account", "assistant_notification_push_sources"))
        }
        "assistant_voice_sessions" => Some(("account_activity", "assistant_voice_sessions")),
        "async_service_operation_artifacts" => {
            Some(("assistant", "async_service_operation_artifacts"))
        }
        "encrypted_record_migration_findings" => {
            Some(("account_activity", "encrypted_record_migration_findings"))
        }
        "encrypted_record_migration_runs" => {
            Some(("account_activity", "encrypted_record_migration_runs"))
        }
        "generated_artifact_content_series" => {
            Some(("account", "generated_artifact_content_series"))
        }
        "node_pending_credentials" => Some(("services", "node_pending_credentials")),
        "oracle_login_snapshots" => Some(("account", "oracle_login_snapshots")),
        "oracle_sessions" => Some(("assistant", "oracle_sessions")),
        "oracle_tasks" => Some(("assistant", "oracle_tasks")),
        "oracle_worker_commands" => Some(("account_activity", "oracle_worker_commands")),
        "oracle_workers" => Some(("account", "oracle_workers")),
        "org_channel_lanes" => Some(("channels", "org_channel_lanes")),
        "org_channel_link_challenges" => Some(("account_activity", "org_channel_link_challenges")),
        "org_channel_messages" => Some(("channels", "org_channel_messages")),
        "org_channel_sender_links" => Some(("channels", "org_channel_sender_links")),
        "platform_discord_control_installation_history" => {
            Some(("account", "platform_discord_control_installation_history"))
        }
        "platform_discord_control_installations" => {
            Some(("account", "platform_discord_control_installations"))
        }
        "platform_onboarding_binding_requests" => {
            Some(("account_activity", "platform_onboarding_binding_requests"))
        }
        "platform_onboarding_bindings" => {
            Some(("account_activity", "platform_onboarding_bindings"))
        }
        "platform_personal_bindings" => Some(("account", "platform_personal_bindings")),
        "platform_personal_decision_spacing" => {
            Some(("account_activity", "platform_personal_decision_spacing"))
        }
        "platform_personal_link_challenges" => {
            Some(("account_activity", "platform_personal_link_challenges"))
        }
        "platform_personal_link_intents" => {
            Some(("account_activity", "platform_personal_link_intents"))
        }
        "platform_personal_setup_selections" => {
            Some(("account_activity", "platform_personal_setup_selections"))
        }
        "platform_public_deployment_activations" => {
            Some(("account", "platform_public_deployment_activations"))
        }
        "platform_public_deployment_proofs" => {
            Some(("account", "platform_public_deployment_proofs"))
        }
        "users" => Some(("account", "users")),
        _ => None,
    }
}

#[derive(Clone, Copy)]
struct ExplicitExportSpec {
    section: &'static str,
    subsection: &'static str,
    fields: &'static [&'static str],
}

/// Resolve the section, subsection, and field allowlist together.  Keeping
/// this as one explicit lookup means a new registry export cannot silently
/// fall through to a broad common projection.
fn explicit_export_spec(collection: &str) -> Result<ExplicitExportSpec, ExportSpecError> {
    let (section, subsection) =
        export_spec(collection).ok_or_else(|| ExportSpecError(collection.to_owned()))?;
    Ok(ExplicitExportSpec {
        section,
        subsection,
        fields: fields_for(collection)?,
    })
}
const ACCOUNT_RECORD_FIELDS: &[&str] = &[
    "user_id",
    "owner_id",
    "owner_user_id",
    "approved_user_id",
    "member_user_id",
    "conversation_id",
    "service_id",
    "service_slug",
    "slug",
    "name",
    "display_name",
    "kind",
    "type",
    "status",
    "state",
    "role",
    "action",
    "direction",
    "content_type",
    "filename",
    "size",
    "created_at",
    "updated_at",
    "completed_at",
    "expires_at",
    "deleted_at",
    "email_verified",
    "last_login_at",
    "enabled",
    "is_active",
];
const SERVICE_RECORD_FIELDS: &[&str] = &[
    "user_id",
    "owner_id",
    "owner_user_id",
    "service_id",
    "service_slug",
    "slug",
    "name",
    "display_name",
    "kind",
    "type",
    "status",
    "state",
    "enabled",
    "is_active",
    "created_at",
    "updated_at",
    "completed_at",
    "expires_at",
];
const ASSISTANT_RECORD_FIELDS: &[&str] = &[
    "user_id",
    "owner_id",
    "actor_user_id",
    "actor_id",
    "subject_id",
    "conversation_id",
    "service_id",
    "service_slug",
    "name",
    "display_name",
    "kind",
    "type",
    "role",
    "status",
    "state",
    "action",
    "content",
    "text",
    "created_at",
    "updated_at",
    "completed_at",
    "expires_at",
];
const ASSISTANT_AGENT_FIELDS: &[&str] = &[
    "user_id",
    "name",
    "display_name",
    "description",
    "specialty",
    "persona",
    "voice_instructions",
    "memory.text",
    "created_at",
    "updated_at",
];
const ASSISTANT_CONVERSATION_FIELDS: &[&str] = &[
    "user_id",
    "title",
    "model",
    "created_at",
    "updated_at",
    "message_count",
];
const GROUP_MESSAGE_FIELDS: &[&str] = &[
    "user_id",
    "group_id",
    "author_user_id",
    "role",
    "text",
    "created_at",
];
const FEEDBACK_FIELDS: &[&str] = &[
    "user_id",
    "message_id",
    "rating",
    "comment",
    "created_at",
    "updated_at",
];
const PRIVATE_NOTES_FIELDS: &[&str] = &[
    "actor_user_id",
    "agent_id",
    "revision",
    "notes.text",
    "notes.created_at",
    "notes.updated_at",
    "updated_at",
];
const TRIGGER_FIELDS: &[&str] = &[
    "user_id",
    "label",
    "status",
    "source",
    "schedule",
    "schedule_state",
    "overlap",
    "delivery",
    "created_at",
    "updated_at",
];
const USER_ENDPOINT_FIELDS: &[&str] = &[
    "user_id",
    "label",
    "url",
    "catalog_service_id",
    "openapi_spec_url",
    "recommended_skills",
    "created_at",
    "updated_at",
];
const SERVICE_CONNECTION_FIELDS: &[&str] = &[
    "user_id",
    "service_id",
    "credential_type",
    "credential_label",
    "metadata",
    "is_active",
    "created_at",
    "updated_at",
];
const NODE_FIELDS: &[&str] = &[
    "user_id",
    "name",
    "status",
    "is_active",
    "created_at",
    "updated_at",
];
const CHANNEL_BOT_FIELDS: &[&str] = &[
    "user_id",
    "label",
    "platform",
    "status",
    "is_active",
    "created_at",
    "updated_at",
];
const OAUTH_CLIENT_FIELDS: &[&str] = &[
    "created_by",
    "client_name",
    "redirect_uris",
    "allowed_scopes",
    "grant_types",
    "client_type",
    "is_active",
    "delegation_scopes",
    "default_service_catalog_slugs",
    "broker_capability_enabled",
    "connection_webhook_url",
    "revocation_webhook_url",
    "connection_webhook_enabled",
    "created_at",
    "updated_at",
];
const SERVICE_ACCOUNT_FIELDS: &[&str] = &[
    "owner_user_id",
    "created_by",
    "name",
    "description",
    "role_ids",
    "allowed_scopes",
    "is_active",
    "purpose",
    "rate_limit_override",
    "created_at",
    "updated_at",
    "last_authenticated_at",
];
const CHANNEL_RECORD_FIELDS: &[&str] = &[
    "user_id",
    "owner_id",
    "conversation_id",
    "channel_id",
    "direction",
    "text",
    "content",
    "content_type",
    "status",
    "state",
    "created_at",
    "updated_at",
    "completed_at",
    "expires_at",
];
const ACTIVITY_RECORD_FIELDS: &[&str] = &[
    "user_id",
    "owner_id",
    "owner_user_id",
    "actor_user_id",
    "actor_id",
    "subject_id",
    "service_id",
    "conversation_id",
    "kind",
    "type",
    "action",
    "status",
    "state",
    "name",
    "display_name",
    "created_at",
    "updated_at",
    "completed_at",
    "expires_at",
    "actor.person_id",
    "actor.app_id",
    "actor.name",
];
const INVITE_USAGE_FIELDS: &[&str] = &[
    "used_count",
    "max_uses",
    "usages.user_id",
    "usages.used_at",
    "created_at",
    "updated_at",
    "expires_at",
];
const SERVICE_HISTORY_FIELDS: &[&str] = &[
    "owner_id",
    "service_id",
    "event_type",
    "created_at",
    "updated_at",
    "actor.person_id",
    "actor.app_id",
    "actor.name",
];
const MACHINE_RECORD_FIELDS: &[&str] = &[
    "user_id",
    "owner_id",
    "owner_user_id",
    "node_id",
    "machine_id",
    "service_id",
    "kind",
    "type",
    "status",
    "state",
    "is_active",
    "created_at",
    "updated_at",
    "completed_at",
    "expires_at",
];
const SECURITY_RECORD_FIELDS: &[&str] = &[
    "user_id",
    "owner_id",
    "approved_user_id",
    "approved_by_user_id",
    "service_id",
    "kind",
    "type",
    "status",
    "state",
    "enabled",
    "is_active",
    "created_at",
    "updated_at",
    "completed_at",
    "expires_at",
];
const MESSAGE_FIELDS: &[&str] = &[
    "user_id",
    "conversation_id",
    "role",
    "content",
    "created_at",
];
const CHANNEL_FIELDS: &[&str] = &[
    "user_id",
    "conversation_id",
    "direction",
    "text",
    "created_at",
];
const FILE_METADATA_FIELDS: &[&str] = &[
    "user_id",
    "filename",
    "content_type",
    "size",
    "created_at",
    "expires_at",
];
const PROFILE_FIELDS: &[&str] = &[
    "email",
    "display_name",
    "avatar_url",
    "email_verified",
    "mfa_enabled",
    "social_provider",
    "created_at",
    "updated_at",
    "last_login_at",
    "profile_config.security.skip_mfa_for_approved_sign_ins",
    "profile_config.analytics.enabled",
    "profile_config.services_view",
    "profile_config.service_views",
    "profile_config.onboarding.ai_services_completed_at",
];
const BILLING_WALLET_FIELDS: &[&str] = &[
    "owner_id",
    "plan_kind",
    "balance_credits",
    "reserved_credits",
    "pending_lago_debits",
    "pending_topup_expiry_credits",
    "has_payment_instrument",
    "overdraft_cap_credits",
    "suspended",
    "collection_state",
    "balance_synced_at",
    "created_at",
    "updated_at",
];
const CREDIT_GRANT_FIELDS: &[&str] = &[
    "recipient_user_id",
    "target_kind",
    "amount",
    "remaining",
    "reserved",
    "scope",
    "expires_at",
    "reason",
    "status",
    "created_at",
    "updated_at",
    "consumed_at",
    "expired_at",
    "revoked_at",
];
const USAGE_FIELDS: &[&str] = &[
    "billing_owner_id",
    "service_id",
    "service_slug",
    "metric",
    "quantity",
    "status",
    "created_at",
    "settled_at",
    "wallet_funded",
    "grant_funded",
    "allowance_funded",
    "wallet_charge_credits",
];

fn fields_for(collection: &str) -> Result<&'static [&'static str], ExportSpecError> {
    let fields = match collection {
        "agent_key_login_requests" => ACCOUNT_RECORD_FIELDS,
        "agent_service_bindings" => ACCOUNT_RECORD_FIELDS,
        "api_key_credentials" => SERVICE_RECORD_FIELDS,
        "api_keys" => SERVICE_RECORD_FIELDS,
        "approval_grants" => ACCOUNT_RECORD_FIELDS,
        "approval_requests" => ACCOUNT_RECORD_FIELDS,
        "assistant_acknowledgements" => ASSISTANT_RECORD_FIELDS,
        "assistant_action_receipts" => ACTIVITY_RECORD_FIELDS,
        "assistant_agent_consents" => ASSISTANT_RECORD_FIELDS,
        "assistant_agent_learning" => ASSISTANT_RECORD_FIELDS,
        "assistant_agent_learning_members" => ACCOUNT_RECORD_FIELDS,
        "assistant_agent_learning_proposals" => ACCOUNT_RECORD_FIELDS,
        "assistant_agent_learning_runs" => ASSISTANT_RECORD_FIELDS,
        "assistant_agent_learning_skill_roots" => ACCOUNT_RECORD_FIELDS,
        "assistant_agent_private_notes" => PRIVATE_NOTES_FIELDS,
        "assistant_agent_shares" => ASSISTANT_RECORD_FIELDS,
        "assistant_agent_sharing" => ASSISTANT_RECORD_FIELDS,
        "assistant_agents" => ASSISTANT_AGENT_FIELDS,
        "assistant_artifact_deliveries" => ASSISTANT_RECORD_FIELDS,
        "assistant_async_operations" => ASSISTANT_RECORD_FIELDS,
        "assistant_async_quotas" => ASSISTANT_RECORD_FIELDS,
        "assistant_attachment_expiry" => ASSISTANT_RECORD_FIELDS,
        "assistant_attachments" => FILE_METADATA_FIELDS,
        "assistant_authored_draft_claims" => ASSISTANT_RECORD_FIELDS,
        "assistant_checkpoints" => ASSISTANT_RECORD_FIELDS,
        "assistant_cma_agents" => ASSISTANT_RECORD_FIELDS,
        "assistant_cma_creations" => ASSISTANT_RECORD_FIELDS,
        "assistant_cma_sandboxes" => ASSISTANT_RECORD_FIELDS,
        "assistant_cma_session_origins" => ACCOUNT_RECORD_FIELDS,
        "assistant_conversations" => ASSISTANT_CONVERSATION_FIELDS,
        "assistant_direct_handoffs" => ASSISTANT_RECORD_FIELDS,
        "assistant_group_agent_access" => ACCOUNT_RECORD_FIELDS,
        "assistant_group_agent_proposals" => ASSISTANT_RECORD_FIELDS,
        "assistant_group_execution_slots" => ACCOUNT_RECORD_FIELDS,
        "assistant_group_invites" => ASSISTANT_RECORD_FIELDS,
        "assistant_group_mention_notifications" => ACCOUNT_RECORD_FIELDS,
        "assistant_group_messages" => GROUP_MESSAGE_FIELDS,
        "assistant_group_read_states" => ASSISTANT_RECORD_FIELDS,
        "assistant_group_requests" => ASSISTANT_RECORD_FIELDS,
        "assistant_groups" => ASSISTANT_RECORD_FIELDS,
        "assistant_invocation_turns" => ASSISTANT_RECORD_FIELDS,
        "assistant_invocations" => ASSISTANT_RECORD_FIELDS,
        "assistant_learning_migrations" => ACCOUNT_RECORD_FIELDS,
        "assistant_learning_publication_targets" => ACCOUNT_RECORD_FIELDS,
        "assistant_message_feedback" => FEEDBACK_FIELDS,
        "assistant_messages" => MESSAGE_FIELDS,
        "assistant_notification_push_budgets" => ACCOUNT_RECORD_FIELDS,
        "assistant_notification_push_sources" => ACCOUNT_RECORD_FIELDS,
        "assistant_notifications" => ASSISTANT_RECORD_FIELDS,
        "assistant_onboarding" => ASSISTANT_RECORD_FIELDS,
        "assistant_resources" => ASSISTANT_RECORD_FIELDS,
        "assistant_service_execution" => ASSISTANT_RECORD_FIELDS,
        "assistant_settings" => ASSISTANT_RECORD_FIELDS,
        "assistant_skill_draft_attempts" => ASSISTANT_RECORD_FIELDS,
        "assistant_tool_results" => ASSISTANT_RECORD_FIELDS,
        "assistant_voice_announcements" => ASSISTANT_RECORD_FIELDS,
        "assistant_voice_requests" => ASSISTANT_RECORD_FIELDS,
        "assistant_voice_windows" => ACTIVITY_RECORD_FIELDS,
        "assistant_wire_logs" => ACTIVITY_RECORD_FIELDS,
        "async_service_operation_artifacts" => FILE_METADATA_FIELDS,
        "auth_device_codes" => ACCOUNT_RECORD_FIELDS,
        "auth_device_codes_v2" => ACCOUNT_RECORD_FIELDS,
        "authorization_codes" => ACCOUNT_RECORD_FIELDS,
        "billing_wallet" => BILLING_WALLET_FIELDS,
        "catalog_delegation_grants" => SERVICE_RECORD_FIELDS,
        "channel_activity_notifications" => CHANNEL_RECORD_FIELDS,
        "channel_adapter_accounts" => ACTIVITY_RECORD_FIELDS,
        "channel_adapter_allocations" => CHANNEL_RECORD_FIELDS,
        "channel_adapter_bindings" => CHANNEL_RECORD_FIELDS,
        "channel_adapter_events" => CHANNEL_RECORD_FIELDS,
        "channel_adapter_resource_claims" => CHANNEL_RECORD_FIELDS,
        "channel_adapter_sessions" => CHANNEL_RECORD_FIELDS,
        "channel_bots" => CHANNEL_RECORD_FIELDS,
        "channel_connect_links" => CHANNEL_RECORD_FIELDS,
        "channel_conversations" => CHANNEL_RECORD_FIELDS,
        "channel_delivery_receipts" => CHANNEL_RECORD_FIELDS,
        "channel_email_batches" => CHANNEL_RECORD_FIELDS,
        "channel_email_receipts" => CHANNEL_RECORD_FIELDS,
        "channel_email_sends" => CHANNEL_RECORD_FIELDS,
        "channel_email_subscriptions" => CHANNEL_RECORD_FIELDS,
        "channel_messages" => CHANNEL_FIELDS,
        "channel_reply_deliveries" => CHANNEL_RECORD_FIELDS,
        "channel_text_deliveries" => CHANNEL_RECORD_FIELDS,
        "channel_webhook_rejections" => CHANNEL_RECORD_FIELDS,
        "channel_whatsapp_native" => CHANNEL_RECORD_FIELDS,
        "cli_pairings" => ACCOUNT_RECORD_FIELDS,
        "connect_links" => ACCOUNT_RECORD_FIELDS,
        "consents" => ACCOUNT_RECORD_FIELDS,
        "credit_grants" => CREDIT_GRANT_FIELDS,
        "device_codes" => SECURITY_RECORD_FIELDS,
        "device_onboard_credentials" => ACCOUNT_RECORD_FIELDS,
        "encrypted_record_migration_findings" => ACCOUNT_RECORD_FIELDS,
        "encrypted_record_migration_runs" => ACCOUNT_RECORD_FIELDS,
        "execution_permission_deliveries" => ACCOUNT_RECORD_FIELDS,
        "feature_flag_overrides" => ACTIVITY_RECORD_FIELDS,
        "generated_artifact_content_series" => ACCOUNT_RECORD_FIELDS,
        "invite_codes" => INVITE_USAGE_FIELDS,
        "invocation_diagnostics" => ACTIVITY_RECORD_FIELDS,
        "lark_base_write_photos" => ACCOUNT_RECORD_FIELDS,
        "lark_base_writes" => ACCOUNT_RECORD_FIELDS,
        "login_approvals" => SECURITY_RECORD_FIELDS,
        "login_codes" => SECURITY_RECORD_FIELDS,
        "machine_authority_leases" => MACHINE_RECORD_FIELDS,
        "machine_confirmations" => MACHINE_RECORD_FIELDS,
        "machine_contexts" => MACHINE_RECORD_FIELDS,
        "machine_desktops" => MACHINE_RECORD_FIELDS,
        "machine_external_grants" => MACHINE_RECORD_FIELDS,
        "machine_free_slots" => MACHINE_RECORD_FIELDS,
        "machine_jobs" => MACHINE_RECORD_FIELDS,
        "machine_oauth_consent_requests" => MACHINE_RECORD_FIELDS,
        "machine_orphan_inventory" => MACHINE_RECORD_FIELDS,
        "machine_revocations" => MACHINE_RECORD_FIELDS,
        "machine_separation_compatibility" => ACCOUNT_RECORD_FIELDS,
        "machine_setups" => MACHINE_RECORD_FIELDS,
        "machine_update_attempts" => MACHINE_RECORD_FIELDS,
        "machine_updates" => MACHINE_RECORD_FIELDS,
        "mcp_session_notifications" => ACTIVITY_RECORD_FIELDS,
        "mcp_session_slots" => ACCOUNT_RECORD_FIELDS,
        "mfa_challenges" => ACCOUNT_RECORD_FIELDS,
        "mfa_factors" => SECURITY_RECORD_FIELDS,
        "node_registration_tokens" => ACCOUNT_RECORD_FIELDS,
        "nodes" => MACHINE_RECORD_FIELDS,
        "notification_channels" => SECURITY_RECORD_FIELDS,
        "nyxbot_channels" => CHANNEL_RECORD_FIELDS,
        "nyxbot_event_bindings" => ASSISTANT_RECORD_FIELDS,
        "nyxbot_events" => ASSISTANT_RECORD_FIELDS,
        "nyxbot_gateway_responses" => ASSISTANT_RECORD_FIELDS,
        "nyxbot_image_deliveries" => ASSISTANT_RECORD_FIELDS,
        "nyxbot_image_deliveries_v2" => ASSISTANT_RECORD_FIELDS,
        "nyxbot_threads" => ASSISTANT_RECORD_FIELDS,
        "nyxbot_watches" => ASSISTANT_RECORD_FIELDS,
        "oauth_broker_bindings" => SERVICE_RECORD_FIELDS,
        "oauth_clients" => OAUTH_CLIENT_FIELDS,
        "oauth_consent_requests" => SERVICE_RECORD_FIELDS,
        "oauth_delegation_authorities" => SERVICE_RECORD_FIELDS,
        "oauth_states" => SERVICE_RECORD_FIELDS,
        "openclaw_channel_mappings" => ACTIVITY_RECORD_FIELDS,
        "oracle_pools" => ACCOUNT_RECORD_FIELDS,
        "org_invites" => ACCOUNT_RECORD_FIELDS,
        "org_memberships" => ACCOUNT_RECORD_FIELDS,
        "permission_key_policies" => ACCOUNT_RECORD_FIELDS,
        "platform_bot_cleanup" => ACCOUNT_RECORD_FIELDS,
        "platform_bot_configs" => ACCOUNT_RECORD_FIELDS,
        "platform_bot_installations" => ACCOUNT_RECORD_FIELDS,
        "platform_bot_runtime_controls" => ACCOUNT_RECORD_FIELDS,
        "platform_delivery_tests" => ACCOUNT_RECORD_FIELDS,
        "platform_discord_control_installation_history" => ACCOUNT_RECORD_FIELDS,
        "platform_discord_control_installations" => ACCOUNT_RECORD_FIELDS,
        "platform_guide_selections" => ACTIVITY_RECORD_FIELDS,
        "platform_native_entry_receipts" => ACCOUNT_RECORD_FIELDS,
        "platform_native_replies" => ACTIVITY_RECORD_FIELDS,
        "platform_onboarding_binding_requests" => ACCOUNT_RECORD_FIELDS,
        "platform_onboarding_bindings" => ACCOUNT_RECORD_FIELDS,
        "platform_personal_decision_spacing" => ACCOUNT_RECORD_FIELDS,
        "platform_personal_setup_selections" => ACCOUNT_RECORD_FIELDS,
        "platform_personal_stop_effects" => ACCOUNT_RECORD_FIELDS,
        "platform_public_deployment_activations" => ACCOUNT_RECORD_FIELDS,
        "platform_public_deployment_proofs" => ACCOUNT_RECORD_FIELDS,
        "platform_public_probe_consents" => ACCOUNT_RECORD_FIELDS,
        "platform_public_readiness_probes" => ACCOUNT_RECORD_FIELDS,
        "platform_setup_notices" => ACCOUNT_RECORD_FIELDS,
        "platform_setup_steps" => ACCOUNT_RECORD_FIELDS,
        "platform_setups" => ACCOUNT_RECORD_FIELDS,
        "refresh_tokens" => SECURITY_RECORD_FIELDS,
        "saved_logins" => ACCOUNT_RECORD_FIELDS,
        "service_account_key_read_grants" => SERVICE_RECORD_FIELDS,
        "service_account_tokens" => SERVICE_RECORD_FIELDS,
        "service_accounts" => SERVICE_ACCOUNT_FIELDS,
        "service_approval_configs" => SERVICE_RECORD_FIELDS,
        "service_change_events" => SERVICE_HISTORY_FIELDS,
        "service_pool_member_health" => SERVICE_RECORD_FIELDS,
        "service_pools" => SERVICE_RECORD_FIELDS,
        "service_preferences" => SERVICE_RECORD_FIELDS,
        "sessions" => SECURITY_RECORD_FIELDS,
        "telegram_bot_claims" => ACTIVITY_RECORD_FIELDS,
        "telegram_bot_requests" => ACTIVITY_RECORD_FIELDS,
        "telegram_managed_bot_events" => ACTIVITY_RECORD_FIELDS,
        "trigger_deliveries" => ACCOUNT_RECORD_FIELDS,
        "trigger_owner_limits" => ACCOUNT_RECORD_FIELDS,
        "trigger_runs" => ACCOUNT_RECORD_FIELDS,
        "trigger_work" => ACCOUNT_RECORD_FIELDS,
        "triggers" => TRIGGER_FIELDS,
        "usage_meter" => USAGE_FIELDS,
        "usage_workspaces" => ACCOUNT_RECORD_FIELDS,
        "user_api_keys" => SERVICE_RECORD_FIELDS,
        "user_endpoints" => USER_ENDPOINT_FIELDS,
        "user_provider_credentials" => SERVICE_RECORD_FIELDS,
        "user_provider_tokens" => SERVICE_RECORD_FIELDS,
        "user_service_connections" => SERVICE_CONNECTION_FIELDS,
        "user_services" => SERVICE_RECORD_FIELDS,
        "assistant_voice_sessions" => ACTIVITY_RECORD_FIELDS,
        "generated_artifact_content_series" => FILE_METADATA_FIELDS,
        "node_pending_credentials" => SERVICE_RECORD_FIELDS,
        "oracle_login_snapshots" => ACCOUNT_RECORD_FIELDS,
        "oracle_sessions" => ASSISTANT_RECORD_FIELDS,
        "oracle_tasks" => ASSISTANT_RECORD_FIELDS,
        "oracle_worker_commands" => ACTIVITY_RECORD_FIELDS,
        "oracle_workers" => ACCOUNT_RECORD_FIELDS,
        "org_channel_lanes" => CHANNEL_RECORD_FIELDS,
        "org_channel_link_challenges" => ACTIVITY_RECORD_FIELDS,
        "org_channel_messages" => CHANNEL_FIELDS,
        "org_channel_sender_links" => CHANNEL_RECORD_FIELDS,
        "platform_personal_bindings" => ACCOUNT_RECORD_FIELDS,
        "platform_personal_link_challenges" => ACTIVITY_RECORD_FIELDS,
        "platform_personal_link_intents" => ACTIVITY_RECORD_FIELDS,
        "users" => PROFILE_FIELDS,
        _ => return Err(ExportSpecError(collection.to_owned())),
    };
    Ok(fields)
}
fn owner_filter(entry: &personal_data_registry::Entry, account_id: &str) -> Option<Document> {
    if entry.collection == "assistant_group_messages" {
        // Group rows are keyed by the group owner, but the export is a
        // person's authored record. Never include agent, notice, or another
        // participant's text merely because it shares the group.
        return Some(doc! {
            "user_id": account_id,
            "author_user_id": account_id,
        });
    }
    let fields = match entry.owner {
        personal_data_registry::OwnerSelector::Fields(fields) if !fields.is_empty() => fields,
        _ => return None,
    };
    if fields.len() == 1 && fields[0] == "_id" {
        return Some(doc! {"_id": account_id});
    }
    let clauses: Vec<Document> = fields
        .iter()
        .map(|field| {
            let mut d = Document::new();
            d.insert(*field, account_id);
            d
        })
        .collect();
    Some(if clauses.len() == 1 {
        clauses.into_iter().next().unwrap()
    } else {
        doc! {"$or": clauses}
    })
}

struct ExportBudget {
    remaining: usize,
}

impl ExportBudget {
    fn new() -> Self {
        // Leave room for the fixed envelope, section names, and truncation
        // metadata. The final cap pass remains a correctness backstop.
        Self {
            remaining: MAX_BYTES.saturating_sub(64 * 1024),
        }
    }

    fn admit(&mut self, value: &serde_json::Value) -> bool {
        let bytes = serde_json::to_vec(value)
            .map(|value| value.len())
            .unwrap_or(usize::MAX);
        let needed = bytes.saturating_add(1);
        if needed > self.remaining {
            return false;
        }
        self.remaining -= needed;
        true
    }
}

async fn rows(
    db: &mongodb::Database,
    collection: &str,
    filter: Document,
    fields: &[&str],
    sort: Document,
    budget: &mut ExportBudget,
) -> AppResult<ExportSubsection> {
    let total_count = db
        .collection::<Document>(collection)
        .count_documents(filter.clone())
        .max_time(QUERY_TIMEOUT)
        .await?;
    let mut projection = Document::new();
    for field in fields {
        projection.insert(*field, 1);
    }
    let mut cursor = db
        .collection::<Document>(collection)
        .find(filter)
        .projection(projection)
        .sort(sort)
        .limit(SUBSECTION_ROW_CAP + 1)
        .max_time(QUERY_TIMEOUT)
        .await?;
    let mut items = Vec::new();
    let mut truncated = total_count > SUBSECTION_ROW_CAP as u64;
    while let Some(row) = cursor.try_next().await? {
        if items.len() == SUBSECTION_ROW_CAP as usize {
            truncated = true;
            break;
        }
        let item = safe_item(&row, fields);
        if !budget.admit(&item) {
            truncated = true;
            break;
        }
        items.push(item);
    }
    Ok(ExportSubsection {
        items,
        total_count,
        truncated,
    })
}
fn empty_sections() -> BTreeMap<String, BTreeMap<String, ExportSubsection>> {
    personal_data_registry::export_sections()
        .map(|section| (section.to_owned(), BTreeMap::new()))
        .collect()
}
fn enforce_cap_with_limit(export: &mut AccountExport, max_bytes: usize) {
    // Serialize the fixed envelope once, then account for each item as it is
    // admitted or removed. This keeps cap enforcement linear in the number of
    // rows instead of repeatedly serializing the complete export (which was
    // quadratic for a large message subsection).
    let mut skeleton = export.clone();
    for subsections in skeleton.sections.values_mut() {
        for subsection in subsections.values_mut() {
            subsection.items.clear();
        }
    }
    skeleton.truncated_sections.clear();
    let mut current = serde_json::to_vec(&skeleton)
        .map(|value| value.len())
        .unwrap_or(usize::MAX);
    for subsections in export.sections.values() {
        for subsection in subsections.values() {
            let item_bytes: usize = subsection
                .items
                .iter()
                .map(|item| {
                    serde_json::to_vec(item)
                        .map(|value| value.len())
                        .unwrap_or(0)
                })
                .sum();
            current = current.saturating_add(item_bytes);
            current = current.saturating_add(subsection.items.len().saturating_sub(1));
        }
    }
    let mut truncated = BTreeSet::new();
    while current > max_bytes {
        let Some((section, subsection, value)) = export
            .sections
            .iter_mut()
            .flat_map(|(section, subs)| {
                subs.iter_mut()
                    .map(move |(sub, value)| (section, sub, value))
            })
            .max_by_key(|(_, _, value)| value.items.len())
        else {
            break;
        };
        let Some(item) = value.items.pop() else {
            break;
        };
        let item_bytes = serde_json::to_vec(&item)
            .map(|value| value.len())
            .unwrap_or(0);
        current = current.saturating_sub(item_bytes);
        if !value.items.is_empty() {
            current = current.saturating_sub(1);
        }
        if !value.truncated {
            // JSON `true` is one byte shorter than `false`.
            current = current.saturating_sub(1);
            value.truncated = true;
        }
        let name = format!("{section}.{subsection}");
        if truncated.insert(name.clone()) {
            // `truncated_sections` is an array of strings. The first item has
            // no comma; later additions do.
            current = current.saturating_add(name.len() + 2);
            if truncated.len() > 1 {
                current = current.saturating_add(1);
            }
        }
    }
    export.truncated_sections = truncated.into_iter().collect();
}

fn enforce_cap(export: &mut AccountExport) {
    enforce_cap_with_limit(export, MAX_BYTES);
}

pub async fn build(db: &mongodb::Database, account_id: &str) -> AppResult<AccountExport> {
    let mut sections = empty_sections();
    let mut budget = ExportBudget::new();
    let account = db.collection::<Document>(crate::models::user::COLLECTION_NAME).find_one(doc! {"_id": account_id}).projection(doc! {"_id": 1, "email": 1, "display_name": 1, "avatar_url": 1, "email_verified": 1, "mfa_enabled": 1, "social_provider": 1, "created_at": 1, "updated_at": 1, "last_login_at": 1, "profile_config": 1}).max_time(QUERY_TIMEOUT).await?.ok_or_else(|| AppError::NotFound("User not found".into()))?;
    sections.entry("account".into()).or_default().insert(
        "profile".into(),
        ExportSubsection {
            items: vec![safe_item(
                &account,
                &[
                    "email",
                    "display_name",
                    "avatar_url",
                    "email_verified",
                    "mfa_enabled",
                    "social_provider",
                    "created_at",
                    "updated_at",
                    "last_login_at",
                    "profile_config.security.skip_mfa_for_approved_sign_ins",
                    "profile_config.analytics.enabled",
                    "profile_config.services_view",
                    "profile_config.service_views",
                    "profile_config.onboarding.ai_services_completed_at",
                ],
            )],
            total_count: 1,
            truncated: false,
        },
    );
    let mut handled = BTreeSet::new();
    for (section, collection, fields, filter, subsection) in [
        (
            "services",
            crate::models::user_service::COLLECTION_NAME,
            &["service_id", "slug", "created_at", "updated_at"][..],
            doc! {"user_id": account_id},
            "user_services",
        ),
        (
            "security",
            crate::models::session::COLLECTION_NAME,
            &["created_at", "expires_at", "last_used_at"][..],
            doc! {"user_id": account_id},
            "sessions",
        ),
        (
            "assistant",
            crate::models::assistant_message::COLLECTION_NAME,
            &["conversation_id", "role", "content", "created_at"][..],
            doc! {"user_id": account_id},
            "assistant_messages",
        ),
        (
            "channels",
            crate::models::channel_message::COLLECTION_NAME,
            &["conversation_id", "direction", "text", "created_at"][..],
            doc! {"user_id": account_id},
            "channel_messages",
        ),
        (
            "account_activity",
            crate::models::assistant_action_receipt::COLLECTION_NAME,
            &["action", "status", "created_at", "completed_at"][..],
            doc! {"user_id": account_id},
            "assistant_action_receipts",
        ),
        (
            "machines",
            crate::models::machine_job::COLLECTION_NAME,
            &["status", "created_at", "completed_at"][..],
            doc! {"user_id": account_id},
            "machine_jobs",
        ),
    ] {
        sections.entry(section.into()).or_default().insert(
            subsection.into(),
            rows(
                db,
                collection,
                filter,
                fields,
                doc! {"created_at": -1},
                &mut budget,
            )
            .await?,
        );
        handled.insert(collection);
    }
    let memberships = rows(
        db,
        "org_memberships",
        doc! {"member_user_id": account_id},
        &[
            "org_user_id",
            "member_user_id",
            "role",
            "created_at",
            "updated_at",
        ],
        doc! {"created_at": -1},
        &mut budget,
    )
    .await?;
    sections
        .entry("account".into())
        .or_default()
        .insert("org_memberships".into(), memberships);
    handled.insert("org_memberships");
    let audit = rows(
        db,
        crate::models::audit_log::COLLECTION_NAME,
        doc! {"user_id": account_id},
        &["event_type", "created_at"],
        doc! {"created_at": -1},
        &mut budget,
    )
    .await?;
    sections
        .entry("account_activity".into())
        .or_default()
        .insert("audit_log".into(), audit);
    handled.insert(crate::models::audit_log::COLLECTION_NAME);

    // Financial rows are retained for integrity, but the person's own
    // user-visible summaries are part of the export. Only display fields are
    // projected; ledger hashes, provider references and settlement locks stay
    // excluded.
    let wallet = rows(
        db,
        crate::models::billing_wallet::COLLECTION_NAME,
        doc! {"owner_id": account_id},
        &[
            "owner_id",
            "plan_kind",
            "balance_credits",
            "reserved_credits",
            "pending_lago_debits",
            "pending_topup_expiry_credits",
            "has_payment_instrument",
            "overdraft_cap_credits",
            "suspended",
            "collection_state",
            "balance_synced_at",
            "created_at",
            "updated_at",
        ],
        doc! {"updated_at": -1},
        &mut budget,
    )
    .await?;
    sections
        .entry("billing".into())
        .or_default()
        .insert("wallet".into(), wallet);
    handled.insert(crate::models::billing_wallet::COLLECTION_NAME);
    let grants = rows(
        db,
        crate::models::credit_grant::COLLECTION_NAME,
        doc! {"recipient_user_id": account_id},
        &[
            "recipient_user_id",
            "target_kind",
            "amount",
            "remaining",
            "reserved",
            "scope",
            "expires_at",
            "reason",
            "status",
            "created_at",
            "updated_at",
            "consumed_at",
            "expired_at",
            "revoked_at",
        ],
        doc! {"created_at": -1},
        &mut budget,
    )
    .await?;
    sections
        .entry("billing".into())
        .or_default()
        .insert("credit_grants".into(), grants);
    handled.insert(crate::models::credit_grant::COLLECTION_NAME);
    let usage = rows(
        db,
        crate::models::usage_meter::COLLECTION_NAME,
        doc! {"billing_owner_id": account_id},
        &[
            "billing_owner_id",
            "service_id",
            "service_slug",
            "metric",
            "quantity",
            "status",
            "created_at",
            "settled_at",
            "wallet_funded",
            "grant_funded",
            "allowance_funded",
            "wallet_charge_credits",
        ],
        doc! {"created_at": -1},
        &mut budget,
    )
    .await?;
    sections
        .entry("billing".into())
        .or_default()
        .insert("usage_meter".into(), usage);
    handled.insert(crate::models::usage_meter::COLLECTION_NAME);
    for entry in personal_data_registry::entries() {
        let personal_data_registry::Export::Section(section) = entry.export else {
            continue;
        };
        if entry.collection == crate::models::user::COLLECTION_NAME
            || handled.contains(entry.collection)
        {
            continue;
        }
        let spec = explicit_export_spec(entry.collection)
            .map_err(|error| AppError::Internal(error.to_string()))?;
        debug_assert_eq!(section, spec.section);
        let Some(filter) = owner_filter(entry, account_id) else {
            continue;
        };
        let value = rows(
            db,
            entry.collection,
            filter,
            spec.fields,
            doc! {"created_at": -1},
            &mut budget,
        )
        .await?;
        sections
            .entry(spec.section.into())
            .or_default()
            .insert(spec.subsection.into(), value);
    }
    let mut export = AccountExport {
        format: FORMAT,
        version: VERSION,
        generated_at: Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        account_id: account_id.to_owned(),
        limits: ExportLimits {
            max_bytes: MAX_BYTES,
            subsection_row_cap: SUBSECTION_ROW_CAP,
            query_timeout_ms: QUERY_TIMEOUT.as_millis() as u64,
            file_payloads: if FILE_PAYLOADS_METADATA_ONLY {
                "metadata_only"
            } else {
                "included"
            },
        },
        truncated_sections: Vec::new(),
        excluded: vec![
            "password and authentication hashes",
            "tokens, keys, secrets, ciphertext, signatures and provider credentials",
            "raw uploaded/generated file payloads (metadata only)",
            "transient coordination, lease, deduplication, erasure and retained integrity records",
        ],
        sections,
    };
    enforce_cap(&mut export);
    Ok(export)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::user::UserType;
    use std::str::FromStr;
    #[test]
    fn recursive_values_preserve_documents_arrays_and_decimal_strings() {
        let value = bson_json(&Bson::Document(
            doc! {"nested": [doc! {"amount": bson::Decimal128::from_str("12.34").unwrap()}]},
        ))
        .unwrap();
        assert_eq!(value["nested"][0]["amount"], "12.34");
        assert!(
            bson_json(&Bson::Binary(bson::Binary {
                subtype: bson::spec::BinarySubtype::Generic,
                bytes: vec![1]
            }))
            .is_none()
        );
    }

    #[test]
    fn url_sanitizer_removes_userinfo_query_fragment_and_opaque_paths() {
        let url = sanitize_url(
            "https://user:URL_USERINFO@api.example.test/v1/PATH_TOKEN?api_key=URL_QUERY#URL_FRAGMENT",
            true,
        );
        assert_eq!(url, "https://api.example.test/");
        assert!(!url.contains("URL_USERINFO"));
        assert!(!url.contains("URL_QUERY"));
        assert!(!url.contains("URL_FRAGMENT"));
        assert!(!url.contains("PATH_TOKEN"));
        let endpoint = sanitize_url(
            "https://user:pass@api.example.test/v1?token=secret#fragment",
            false,
        );
        assert_eq!(endpoint, "https://api.example.test/v1");
    }

    #[test]
    fn export_budget_stops_admitting_rows_at_the_byte_cap() {
        let mut budget = ExportBudget { remaining: 16 };
        assert!(budget.admit(&serde_json::json!("123456")));
        assert!(!budget.admit(&serde_json::json!("this row is too large")));
    }

    #[test]
    fn per_collection_allowlists_keep_authored_text_and_only_own_memory() {
        let row = doc! {
            "title": "PERSON_TITLE",
            "persona": "PERSON_PERSONA",
            "memory": [{"text": "PERSON_MEMORY"}],
            "notes": [{"text": "PERSON_NOTE"}],
            "comment": "PERSON_FEEDBACK",
            "url": "https://user:pass@example.test/v1?token=URL_QUERY#URL_FRAGMENT",
            "shared_memory": "OTHER_PERSON_SHARED_MEMORY",
        };
        let value = safe_item(&row, ASSISTANT_AGENT_FIELDS);
        assert_eq!(value["title"], "PERSON_TITLE");
        assert_eq!(value["persona"], "PERSON_PERSONA");
        assert_eq!(value["memory"][0]["text"], "PERSON_MEMORY");
        assert!(
            safe_item(&row, ASSISTANT_RECORD_FIELDS)
                .get("memory")
                .is_none()
        );
        let notes = safe_item(&row, PRIVATE_NOTES_FIELDS);
        assert_eq!(notes["notes"][0]["text"], "PERSON_NOTE");
        assert!(
            !serde_json::to_string(&value)
                .unwrap()
                .contains("OTHER_PERSON_SHARED_MEMORY")
        );
    }

    #[test]
    fn every_url_allowlist_field_uses_the_common_sanitizer() {
        let row = doc! {
            "avatar_url": "https://u:URL_USERINFO@example.test/avatar/PATH_TOKEN?key=URL_QUERY#URL_FRAGMENT",
            "url": "https://u:URL_USERINFO@example.test/v1/PATH_TOKEN?key=URL_QUERY#URL_FRAGMENT",
            "redirect_uris": ["https://u:URL_USERINFO@example.test/cb/PATH_TOKEN?key=URL_QUERY#URL_FRAGMENT"],
            "delivery": {"url": "https://u:URL_USERINFO@example.test/hook/PATH_TOKEN?key=URL_QUERY#URL_FRAGMENT"},
        };
        let value = safe_item(
            &row,
            &["avatar_url", "url", "redirect_uris", "delivery.url"],
        );
        let bytes = serde_json::to_vec(&value).unwrap();
        for sentinel in ["URL_USERINFO", "URL_QUERY", "URL_FRAGMENT", "PATH_TOKEN"] {
            assert!(
                !bytes
                    .windows(sentinel.len())
                    .any(|window| window == sentinel.as_bytes())
            );
        }
        assert_eq!(value["url"], "https://example.test/v1/PATH_TOKEN");
    }

    #[test]
    fn safe_items_omit_credentials_ids_and_binary_values() {
        let row = doc! {
            "_id": "mcp-session-credential",
            "content": {"text": "kept", "ciphertext": "sentinel", "nested": [1_i32]},
            "token": "sentinel",
            "encrypted_blob": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: vec![1, 2] },
        };
        let value = safe_item(&row, &["_id", "content", "token", "encrypted_blob"]);
        assert_eq!(value["content"]["text"], "kept");
        assert!(value["content"].get("ciphertext").is_none());
        assert!(value.get("_id").is_none());
        assert!(value.get("token").is_none());
        assert!(value.get("encrypted_blob").is_none());
    }

    #[test]
    fn safe_items_keep_nested_allowlisted_paths_structured() {
        let row = doc! {"actor": {"person_id": "opaque", "name": "Deleted account"}};
        let value = safe_item(&row, &["actor.person_id", "actor.name"]);
        assert_eq!(value["actor"]["person_id"], "opaque");
        assert_eq!(value["actor"]["name"], "Deleted account");
    }

    #[test]
    fn safe_items_resolve_nested_array_paths_without_exporting_siblings() {
        let row = doc! {
            "usages": [
                {"user_id": "opaque-1", "used_at": bson::DateTime::now(), "email": "private@example.test"},
                {"user_id": "opaque-2", "used_at": bson::DateTime::now()},
            ],
        };
        let value = safe_item(&row, &["usages.user_id", "usages.used_at"]);
        assert_eq!(value["usages"]["user_id"][0], "opaque-1");
        assert_eq!(value["usages"]["user_id"][1], "opaque-2");
        assert!(value["usages"].get("email").is_none());
    }

    #[test]
    fn export_sentinel_fields_and_mcp_session_identity_are_excluded() {
        let row = doc! {
            "_id": "mcp-session-credential",
            "user_id": "account",
            "content": {
                "message": "visible sentinel-free content",
                "token_value": "SECRET_SENTINEL",
                "nested": [{"amount": Bson::Decimal128("12.50".parse().unwrap())}],
            },
            "password_hash": "SECRET_SENTINEL",
            "session_token_hash": "SECRET_SENTINEL",
            "payload_encrypted": Bson::Binary(bson::Binary {
                subtype: bson::spec::BinarySubtype::Generic,
                bytes: vec![0xaa, 0xbb],
            }),
        };
        let value = safe_item(
            &row,
            &[
                "_id",
                "user_id",
                "content",
                "password_hash",
                "session_token_hash",
                "payload_encrypted",
            ],
        );
        let bytes = serde_json::to_vec(&value).unwrap();
        assert!(
            !bytes
                .windows("SECRET_SENTINEL".len())
                .any(|window| { window == "SECRET_SENTINEL".as_bytes() })
        );
        assert!(value.get("_id").is_none());
        assert_eq!(value["content"]["nested"][0]["amount"], "12.50");
        assert_eq!(value["content"]["message"], "visible sentinel-free content");
    }

    #[test]
    fn byte_cap_marks_only_popped_subsections_as_truncated() {
        let mut sections = BTreeMap::new();
        sections.insert(
            "assistant".into(),
            BTreeMap::from([(
                "messages".into(),
                ExportSubsection {
                    items: vec![serde_json::json!({"body": "x".repeat(64)}); 3],
                    total_count: 3,
                    truncated: false,
                },
            )]),
        );
        let mut export = AccountExport {
            format: FORMAT,
            version: VERSION,
            generated_at: "2026-01-01T00:00:00Z".into(),
            account_id: "account".into(),
            limits: ExportLimits {
                max_bytes: MAX_BYTES,
                subsection_row_cap: SUBSECTION_ROW_CAP,
                query_timeout_ms: QUERY_TIMEOUT.as_millis() as u64,
                file_payloads: "metadata_only",
            },
            truncated_sections: Vec::new(),
            excluded: Vec::new(),
            sections,
        };
        enforce_cap_with_limit(&mut export, 400);
        assert!(export.sections["assistant"]["messages"].truncated);
        assert_eq!(export.truncated_sections, vec!["assistant.messages"]);
        assert!(serde_json::to_vec(&export).unwrap().len() <= 400);
    }
    #[test]
    fn registry_export_mapping_is_one_to_one() {
        let mut seen = BTreeSet::new();
        for entry in personal_data_registry::entries() {
            if matches!(entry.export, personal_data_registry::Export::Section(_)) {
                let spec = explicit_export_spec(entry.collection).expect(entry.collection);
                assert_eq!(
                    spec.section,
                    match entry.export {
                        personal_data_registry::Export::Section(section) => section,
                        _ => unreachable!(),
                    }
                );
                assert!(
                    !spec.fields.is_empty(),
                    "empty explicit field allowlist for {}",
                    entry.collection
                );
                assert!(seen.insert(spec.subsection), "duplicate subsection spec");
                assert!(
                    spec.fields.iter().all(|field| !forbidden_key(field)),
                    "forbidden field in explicit spec for {}",
                    entry.collection
                );
            }
        }
    }

    fn put_owner(document: &mut Document, path: &str, owner: &str) {
        let mut segments = path.split('.').peekable();
        let Some(first) = segments.next() else {
            return;
        };
        if segments.peek().is_none() {
            document.insert(first, owner);
            return;
        }
        let rest = segments.collect::<Vec<_>>().join(".");
        let child = document
            .entry(first.to_owned())
            .or_insert_with(|| Bson::Document(Document::new()));
        if let Bson::Document(child) = child {
            put_owner(child, &rest, owner);
        }
    }

    /// Mongo-backed sentinel coverage for the complete registry export map.
    /// Every source collection gets a target and bystander document; secret
    /// and binary fields are populated even though they are outside the
    /// allowlists. This exercises the real projection and recursive encoder.
    #[tokio::test]
    async fn export_sentinel_isolates_owner_rows_and_values() {
        let db =
            crate::test_utils::connect_transaction_test_database("account_export_sentinel").await;
        let owner = uuid::Uuid::new_v4().to_string();
        let bystander = uuid::Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_many([
                crate::test_utils::test_user(&owner, UserType::Person),
                crate::test_utils::test_user(&bystander, UserType::Person),
            ])
            .await
            .unwrap();
        let mut seeded = BTreeSet::new();
        for entry in personal_data_registry::entries() {
            let personal_data_registry::Export::Section(_) = entry.export else {
                continue;
            };
            let personal_data_registry::OwnerSelector::Fields(fields) = entry.owner else {
                continue;
            };
            if entry.collection == crate::models::user::COLLECTION_NAME
                || fields.is_empty()
                || !seeded.insert(entry.collection)
            {
                continue;
            }
            let target_id = if fields.iter().all(|field| *field == "_id") {
                owner.clone()
            } else {
                format!("target-{}", entry.collection)
            };
            let bystander_id = if fields.iter().all(|field| *field == "_id") {
                bystander.clone()
            } else {
                format!("bystander-{}", entry.collection)
            };
            let mut target = doc! {
                "_id": &target_id,
                "password_hash": "EXPORT_SECRET_SENTINEL",
                "token_value": "EXPORT_SECRET_SENTINEL",
                "ciphertext": "EXPORT_SECRET_SENTINEL",
                "payload_encrypted": Bson::Binary(bson::Binary {
                    subtype: bson::spec::BinarySubtype::Generic,
                    bytes: vec![0xde, 0xad, 0xbe, 0xef],
                }),
                "amount": Bson::Decimal128("17.25".parse().unwrap()),
                "created_at": bson::DateTime::now(),
                "updated_at": bson::DateTime::now(),
            };
            let mut other = target.clone();
            other.insert("_id", &bystander_id);
            for field in fields {
                if *field == "_id" {
                    continue;
                }
                put_owner(&mut target, field, &owner);
                put_owner(&mut other, field, &bystander);
            }
            if entry.collection == "assistant_group_messages" {
                put_owner(&mut target, "author_user_id", &owner);
                put_owner(&mut other, "author_user_id", &bystander);
            }
            db.collection::<Document>(entry.collection)
                .insert_many([target, other])
                .await
                .unwrap();
        }
        let export = build(&db, &owner).await.unwrap();
        let bytes = serde_json::to_vec(&export).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("EXPORT_SECRET_SENTINEL"));
        assert!(!String::from_utf8_lossy(&bytes).contains("bystander-"));
        for entry in personal_data_registry::entries() {
            let personal_data_registry::Export::Section(section) = entry.export else {
                continue;
            };
            let Ok(spec) = explicit_export_spec(entry.collection) else {
                continue;
            };
            if entry.collection == crate::models::user::COLLECTION_NAME {
                continue;
            }
            let subsection = export.sections[section]
                .get(spec.subsection)
                .unwrap_or_else(|| panic!("missing subsection {}", spec.subsection));
            assert!(
                subsection.total_count > 0,
                "seeded owner row missing from {}.{}",
                section,
                spec.subsection
            );
        }
        db.drop().await.unwrap();
    }
}
