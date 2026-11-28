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
pub const PARENT_ID_CAP: usize = 10_000;
pub const QUERY_BATCH_SIZE: u32 = 100;
pub const QUERY_TIMEOUT: Duration = Duration::from_secs(3);
/// Total source-read budget, starting at export admission (before any reads).
pub const BUILD_BUDGET: Duration = Duration::from_secs(60);
pub const FILE_PAYLOADS_METADATA_ONLY: bool = true;

#[derive(Debug, thiserror::Error)]
#[error("no account-export allowlist for collection {0}")]
pub struct ExportSpecError(pub String);

#[derive(Debug, Serialize, Clone, Default)]
pub struct ExportSubsection {
    pub items: Vec<serde_json::Value>,
    pub total_count: u64,
    /// False means total_count is a lower bound, not the full matching total.
    pub total_count_exact: bool,
    pub truncated: bool,
    pub unavailable: bool,
    /// Parent resolution was bounded; total_count covers only the resolved scope.
    pub scope_truncated: bool,
}
#[derive(Debug, Serialize, Clone)]
pub struct ExportLimits {
    pub max_bytes: usize,
    pub subsection_row_cap: i64,
    pub query_timeout_ms: u64,
    pub build_budget_ms: u64,
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
    pub unavailable_sections: Vec<String>,
    pub excluded: Vec<&'static str>,
    pub sections: BTreeMap<String, BTreeMap<String, ExportSubsection>>,
}

#[derive(Debug)]
struct SanitizedJson {
    value: serde_json::Value,
    url_reduced: bool,
}

#[cfg(test)]
fn bson_json(value: &Bson) -> Option<serde_json::Value> {
    sanitize_bson(value, "", false).map(|value| value.value)
}

/// Exported URL values are reduced to their origin. URL paths commonly carry
/// bot tokens, webhook secrets, or opaque identifiers, so retaining a path is
/// not a safe representation of the address. Explicit ports remain visible.
fn sanitize_url(value: &str) -> String {
    let Ok(parsed) = url::Url::parse(value) else {
        return String::new();
    };
    let Some(host) = parsed.host_str() else {
        return String::new();
    };
    let host = match parsed.host() {
        Some(url::Host::Ipv6(_)) => format!("[{host}]"),
        _ => host.to_owned(),
    };
    let mut origin = format!("{}://{host}", parsed.scheme());
    if let Some(port) = parsed.port() {
        origin.push(':');
        origin.push_str(&port.to_string());
    }
    origin
}

fn is_url_field(field: &str) -> bool {
    let leaf = field.rsplit('.').next().unwrap_or(field);
    leaf == "url"
        || leaf == "base_url"
        || leaf == "endpoint"
        || leaf == "redirect_uri"
        || leaf == "redirect_uris"
        || leaf.ends_with("_url")
}

fn is_free_text_field(field: &str) -> bool {
    matches!(
        field.rsplit('.').next().unwrap_or(field),
        "content"
            | "text"
            | "title"
            | "description"
            | "persona"
            | "voice_instructions"
            | "prompt"
            | "response"
            | "instruction"
            | "instructions"
            | "memory"
            | "notes"
            | "comment"
            | "reason"
            | "failure_reason"
            | "label"
    )
}

fn looks_like_absolute_url(value: &str) -> bool {
    url::Url::parse(value)
        .map(|url| url.has_host() && !url.scheme().is_empty())
        .unwrap_or(false)
}

fn sanitize_bson(value: &Bson, field: &str, preserve_text: bool) -> Option<SanitizedJson> {
    match value {
        Bson::String(value) => {
            let declared_url = is_url_field(field);
            let should_reduce = !preserve_text && (declared_url || looks_like_absolute_url(value));
            if should_reduce {
                let reduced = sanitize_url(value);
                return Some(SanitizedJson {
                    url_reduced: reduced != *value || declared_url,
                    value: serde_json::Value::String(reduced),
                });
            }
            Some(SanitizedJson {
                value: serde_json::Value::String(value.clone()),
                url_reduced: false,
            })
        }
        Bson::Boolean(value) => Some(SanitizedJson {
            value: serde_json::Value::Bool(*value),
            url_reduced: false,
        }),
        Bson::Int32(value) => Some(SanitizedJson {
            value: serde_json::json!(*value),
            url_reduced: false,
        }),
        Bson::Int64(value) => Some(SanitizedJson {
            value: serde_json::json!(*value),
            url_reduced: false,
        }),
        Bson::Double(value) if value.is_finite() => Some(SanitizedJson {
            value: serde_json::json!(*value),
            url_reduced: false,
        }),
        Bson::DateTime(value) => Some(SanitizedJson {
            value: serde_json::Value::String(value.to_chrono().to_rfc3339()),
            url_reduced: false,
        }),
        Bson::ObjectId(value) => Some(SanitizedJson {
            value: serde_json::Value::String(value.to_hex()),
            url_reduced: false,
        }),
        Bson::Decimal128(value) => Some(SanitizedJson {
            value: serde_json::Value::String(value.to_string()),
            url_reduced: false,
        }),
        Bson::Null => Some(SanitizedJson {
            value: serde_json::Value::Null,
            url_reduced: false,
        }),
        Bson::Document(value) => {
            let mut result = serde_json::Map::new();
            let mut url_reduced = false;
            for (key, value) in value {
                if forbidden_key(key) {
                    continue;
                }
                let child_text = preserve_text || is_free_text_field(key);
                let Some(child) = sanitize_bson(value, key, child_text) else {
                    continue;
                };
                url_reduced |= child.url_reduced;
                result.insert(key.clone(), child.value);
                if child.url_reduced {
                    result.insert(format!("{key}_path_omitted"), serde_json::Value::Bool(true));
                }
            }
            Some(SanitizedJson {
                value: serde_json::Value::Object(result),
                url_reduced,
            })
        }
        Bson::Array(values) => {
            let mut items = Vec::with_capacity(values.len());
            let mut url_reduced = false;
            for value in values {
                let Some(child) = sanitize_bson(value, field, preserve_text) else {
                    continue;
                };
                url_reduced |= child.url_reduced;
                items.push(child.value);
            }
            Some(SanitizedJson {
                value: serde_json::Value::Array(items),
                url_reduced,
            })
        }
        Bson::Binary(_) => None,
        _ => None,
    }
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
        || key == "credential"
        || key.ends_with("_credential")
        || key.contains("credential_id")
        || key.ends_with("_credentials")
        || key.contains("oauth")
        || key.contains("authorization_code")
        || key.contains("api_key")
        || key.contains("webhook_secret")
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

fn safe_item(row: &Document, fields: &[&str]) -> serde_json::Value {
    let mut result = serde_json::Map::new();
    for field in fields {
        if *field == "_id" || forbidden_key(field) {
            continue;
        }
        if let Some(value) = get_path(row, field)
            .as_ref()
            .and_then(|value| sanitize_bson(value, field, is_free_text_field(field)))
        {
            let SanitizedJson {
                value: sanitized,
                url_reduced,
            } = value;
            insert_path(&mut result, field, sanitized);
            if url_reduced {
                insert_path(
                    &mut result,
                    &format!("{field}_path_omitted"),
                    serde_json::Value::Bool(true),
                );
            }
        }
    }
    serde_json::Value::Object(result)
}

/// Each primary registry export maps to exactly one explicit subsection.
/// Additional scoped views of the same source are declared separately.
fn export_spec(collection: &str) -> Option<(&'static str, &'static str)> {
    match collection {
        "client_error_reports" => Some(("account_activity", "client_error_reports")),
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
        "assistant_groups" => Some(("assistant", "assistant_groups")),
        "assistant_invocation_turns" => Some(("assistant", "assistant_invocation_turns")),
        "assistant_invocations" => Some(("assistant", "assistant_invocations")),
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
        "channel_guest_setup" => Some(("channels", "channel_guest_setup")),
        "channel_messages" => Some(("channels", "channel_messages")),
        "channel_reply_deliveries" => Some(("channels", "channel_reply_deliveries")),
        "channel_text_deliveries" => Some(("channels", "channel_text_deliveries")),
        "channel_whatsapp_native" => Some(("channels", "channel_whatsapp_native")),
        "cli_pairings" => Some(("account", "cli_pairings")),
        "connect_links" => Some(("account", "connect_links")),
        "consents" => Some(("account", "consents")),
        "credit_grants" => Some(("billing", "credit_grants")),
        "credit_schedule_recipients" => Some(("billing", "credit_schedule_recipients")),
        "device_codes" => Some(("security", "device_codes")),
        "device_onboard_credentials" => Some(("account", "device_onboard_credentials")),
        "execution_permission_deliveries" => Some(("account", "execution_permission_deliveries")),
        "feature_flag_overrides" => Some(("account_activity", "feature_flag_overrides")),
        "generated_artifact_content_series" => {
            Some(("account", "generated_artifact_content_series"))
        }
        "image_publications" => Some(("assistant", "image_publications")),
        "invite_codes" => Some(("account_activity", "invite_codes")),
        "invocation_diagnostics" => Some(("account_activity", "invocation_diagnostics")),
        "lark_base_write_photos" => Some(("account", "lark_base_write_photos")),
        "lark_base_writes" => Some(("account", "lark_base_writes")),
        "login_approvals" => Some(("security", "login_approvals")),
        "login_codes" => Some(("security", "login_codes")),
        "machine_confirmations" => Some(("machines", "machine_confirmations")),
        "machine_free_slots" => Some(("machines", "machine_free_slots")),
        "machine_jobs" => Some(("machines", "machine_jobs")),
        "machine_orphan_inventory" => Some(("machines", "machine_orphan_inventory")),
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
        "platform_bot_installations" => Some(("account", "platform_bot_installations")),
        "platform_delivery_tests" => Some(("account_activity", "platform_delivery_tests")),
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
        "platform_personal_stop_effects" => {
            Some(("account_activity", "platform_personal_stop_effects"))
        }
        "platform_public_probe_consents" => Some(("account", "platform_public_probe_consents")),
        "platform_setup_notices" => Some(("account_activity", "platform_setup_notices")),
        "platform_setup_steps" => Some(("account_activity", "platform_setup_steps")),
        "platform_setups" => Some(("account", "platform_setups")),
        "refresh_tokens" => Some(("security", "refresh_tokens")),
        "saved_logins" => Some(("account", "saved_logins")),
        "service_account_key_read_grants" => Some(("services", "service_account_key_read_grants")),
        "service_accounts" => Some(("services", "service_accounts")),
        "service_approval_configs" => Some(("services", "service_approval_configs")),
        "service_change_events" => Some(("account_activity", "service_change_events")),
        "service_pool_member_health" => Some(("services", "service_pool_member_health")),
        "service_pools" => Some(("services", "service_pools")),
        "service_preferences" => Some(("services", "service_preferences")),
        "sessions" => Some(("security", "sessions")),
        "telegram_bot_claims" => Some(("account_activity", "telegram_bot_claims")),
        "telegram_bot_requests" => Some(("account_activity", "telegram_bot_requests")),
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
        "assistant_voice_sessions" => Some(("account_activity", "assistant_voice_sessions")),
        "oracle_login_snapshots" => Some(("account", "oracle_login_snapshots")),
        "oracle_sessions" => Some(("assistant", "oracle_sessions")),
        "oracle_tasks" => Some(("assistant", "oracle_tasks")),
        "oracle_worker_commands" => Some(("account_activity", "oracle_worker_commands")),
        "oracle_workers" => Some(("account", "oracle_workers")),
        "org_channel_lanes" => Some(("channels", "org_channel_lanes")),
        "org_channel_link_challenges" => Some(("account_activity", "org_channel_link_challenges")),
        "org_channel_messages" => Some(("channels", "org_channel_messages")),
        "org_channel_sender_links" => Some(("channels", "org_channel_sender_links")),
        "platform_personal_bindings" => Some(("account", "platform_personal_bindings")),
        "platform_personal_link_challenges" => {
            Some(("account_activity", "platform_personal_link_challenges"))
        }
        "platform_personal_link_intents" => {
            Some(("account_activity", "platform_personal_link_intents"))
        }
        "users" => Some(("account", "profile")),
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

fn additional_export_spec(
    view: &personal_data_registry::AdditionalExportView,
) -> ExplicitExportSpec {
    let fields = match view.selector {
        personal_data_registry::ExportViewSelector::AuditMetadata => &["event_type", "created_at"],
        personal_data_registry::ExportViewSelector::AuthoredGroupMessagesOutsideOwnedGroups => {
            AUTHORED_GROUP_MESSAGE_FIELDS
        }
    };
    ExplicitExportSpec {
        section: view.section,
        subsection: view.subsection,
        fields,
    }
}

pub(super) fn additional_export_filter(
    view: &personal_data_registry::AdditionalExportView,
    account_id: &str,
) -> Document {
    match view.selector {
        personal_data_registry::ExportViewSelector::AuditMetadata => doc! {"user_id":account_id},
        personal_data_registry::ExportViewSelector::AuthoredGroupMessagesOutsideOwnedGroups => {
            doc! {"author_user_id": account_id, "user_id": {"$ne": account_id}}
        }
    }
}
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
    "profile_config.quick_access",
    "profile_config.onboarding.ai_services_completed_at",
];
const AUTHORED_GROUP_MESSAGE_FIELDS: &[&str] = &["group_id", "role", "text", "created_at"];

fn fields_for(collection: &str) -> Result<&'static [&'static str], ExportSpecError> {
    Ok(match collection {
        "client_error_reports" => &[
            "received_at",
            "expires_at",
            "environment",
            "severity",
            "report.platform",
            "report.kind",
            "report.exception_class",
            "report.native_signal",
            "report.app_version",
            "report.app_build",
            "report.os_version",
            "report.runtime",
        ],
        "agent_key_login_requests" => &["approved_user_id", "status", "created_at", "expires_at"],
        "agent_service_bindings" => &["user_id", "created_at", "updated_at"],
        "api_key_credentials" => &["user_id", "kind", "is_active", "created_at", "expires_at"],
        "api_keys" => &[
            "user_id",
            "name",
            "is_active",
            "created_at",
            "updated_at",
            "expires_at",
        ],
        "approval_grants" => &["user_id", "service_id", "expires_at"],
        "approval_requests" => &[
            "user_id",
            "service_id",
            "service_slug",
            "status",
            "created_at",
            "expires_at",
        ],
        "assistant_acknowledgements" => &[
            "user_id",
            "conversation_id",
            "service_id",
            "service_slug",
            "kind",
            "status",
            "created_at",
            "expires_at",
        ],
        "assistant_action_receipts" => {
            &["user_id", "action", "status", "created_at", "completed_at"]
        }
        "assistant_agent_consents" => &[
            "actor_user_id",
            "conversation_id",
            "created_at",
            "updated_at",
            "expires_at",
        ],
        "assistant_agent_learning" => &["owner_id", "created_at", "updated_at"],
        "assistant_agent_learning_members" => &["member_user_id", "created_at", "updated_at"],
        "assistant_agent_learning_proposals" => &["owner_id", "status", "created_at", "updated_at"],
        "assistant_agent_learning_runs" => &["owner_id", "status", "created_at", "updated_at"],
        "assistant_agent_learning_skill_roots" => &["owner_id", "created_at"],
        "assistant_agent_private_notes" => &[
            "actor_user_id",
            "agent_id",
            "revision",
            "notes.text",
            "notes.created_at",
            "notes.updated_at",
            "updated_at",
        ],
        "assistant_agent_shares" => &["role", "created_at"],
        "assistant_agent_sharing" => &["owner_id", "updated_at"],
        "assistant_agents" => &[
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
        ],
        "assistant_artifact_deliveries" => &["user_id", "conversation_id", "expires_at"],
        "assistant_async_operations" => &[
            "user_id",
            "conversation_id",
            "service_id",
            "state",
            "created_at",
            "expires_at",
        ],
        "assistant_async_quotas" => &["user_id", "generation"],
        "assistant_attachment_expiry" => {
            &["user_id", "conversation_id", "created_at", "expired_at"]
        }
        "assistant_attachments" => &["user_id", "content_type", "size", "created_at"],
        "assistant_authored_draft_claims" => &["claim.conversation_id"],
        "assistant_checkpoints" => &["user_id", "status", "updated_at"],
        "assistant_cma_agents" => &["user_id", "created_at", "updated_at"],
        "assistant_cma_creations" => &["user_id", "created_at", "updated_at"],
        "assistant_cma_sandboxes" => &["user_id", "created_at", "updated_at"],
        "assistant_cma_session_origins" => {
            &["user_id", "conversation_id", "service_id", "created_at"]
        }
        "assistant_conversations" => &[
            "user_id",
            "title",
            "model",
            "created_at",
            "updated_at",
            "message_count",
        ],
        "assistant_direct_handoffs" => &["user_id", "state", "expires_at"],
        "assistant_group_agent_access" => &["owner_id", "updated_at"],
        "assistant_group_agent_proposals" => &["expires_at"],
        "assistant_group_execution_slots" => &["conversation_id"],
        "assistant_group_invites" => &["created_at", "expires_at"],
        "assistant_group_mention_notifications" => &["user_id", "created_at"],
        "assistant_group_messages" => &[
            "user_id",
            "group_id",
            "author_user_id",
            "role",
            "text",
            "created_at",
        ],
        "assistant_group_read_states" => &["user_id"],
        "assistant_group_requests" => &["actor_user_id", "created_at"],
        "assistant_groups" => &["user_id", "name", "created_at", "updated_at"],
        "assistant_invocation_turns" => &["user_id", "conversation_id", "state", "created_at"],
        "assistant_invocations" => &["user_id", "conversation_id", "created_at"],
        "assistant_learning_migrations" => &["completed_at"],
        "assistant_learning_publication_targets" => {
            &["owner_id", "state", "created_at", "updated_at"]
        }
        "assistant_message_feedback" => &[
            "user_id",
            "message_id",
            "vote",
            "comment",
            "created_at",
            "updated_at",
        ],
        "assistant_messages" => &["user_id", "conversation_id", "role", "text", "created_at"],
        "assistant_notification_push_budgets" => &["reservations.at"],
        "assistant_notification_push_sources" => &["user_id", "sent_at"],
        "assistant_notifications" => &["user_id", "kind", "created_at", "updated_at", "expires_at"],
        "assistant_onboarding" => &[
            "intro_version",
            "outcome",
            "topics",
            "work_style",
            "created_at",
            "updated_at",
        ],
        "assistant_resources" => &["user_id", "conversation_id", "created_at", "expires_at"],
        "assistant_service_execution" => &["user_id", "conversation_id"],
        "assistant_settings" => &[
            "access_mode",
            "skip_destructive_confirmation",
            "timezone",
            "voice.model",
            "voice.language",
            "voice.voice",
            "voice.input_mode",
            "notifications.push.automation_result",
            "notifications.push.automation_problem",
            "notifications.push.needs_decision",
            "notifications.push.agent_update",
            "notifications.push.chat_update",
            "notifications.push.group_mention",
            "updated_at",
        ],
        "assistant_skill_draft_attempts" => &["user_id", "conversation_id", "created_at"],
        "assistant_tool_results" => &["actor_id", "conversation_id", "expires_at"],
        "assistant_voice_announcements" => &[
            "user_id",
            "conversation_id",
            "state",
            "text",
            "created_at",
            "expires_at",
        ],
        "assistant_voice_requests" => &[
            "user_id",
            "conversation_id",
            "state",
            "created_at",
            "expires_at",
        ],
        "assistant_voice_windows" => &["user_id"],
        "assistant_wire_logs" => &["user_id", "conversation_id", "created_at", "expires_at"],
        "async_service_operation_artifacts" => &[
            "user_id",
            "filename",
            "content_type",
            "size",
            "created_at",
            "expires_at",
        ],
        "auth_device_codes" => &["approved_user_id", "status", "created_at", "expires_at"],
        "auth_device_codes_v2" => &["approved_user_id", "status", "created_at", "expires_at"],
        "authorization_codes" => &["user_id", "created_at", "expires_at"],
        "billing_wallet" => &[
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
        "catalog_delegation_grants" => &["user_id", "created_at", "expires_at"],
        "channel_activity_notifications" => &[
            "user_id",
            "conversation_id",
            "direction",
            "content_type",
            "platform",
            "created_at",
        ],
        "channel_adapter_accounts" => &["updated_at"],
        "channel_adapter_allocations" => &["user_id", "created_at"],
        "channel_adapter_bindings" => &["user_id", "created_at", "updated_at"],
        "channel_adapter_events" => &["user_id", "status", "created_at"],
        "channel_adapter_resource_claims" => &["user_id", "created_at"],
        "channel_adapter_sessions" => &[
            "user_id",
            "conversation_id",
            "state",
            "created_at",
            "expires_at",
        ],
        "channel_bots" => &[
            "user_id",
            "label",
            "platform",
            "status",
            "is_active",
            "created_at",
            "updated_at",
        ],
        "channel_connect_links" => &["user_id", "status", "created_at", "expires_at"],
        "channel_conversations" => &["user_id", "created_at", "updated_at"],
        "channel_delivery_receipts" => &["user_id", "expires_at"],
        "channel_email_batches" => &["user_id", "expires_at"],
        "channel_email_receipts" => &["user_id", "created_at"],
        "channel_email_sends" => &["user_id", "status", "created_at", "updated_at"],
        "channel_email_subscriptions" => &["user_id", "created_at"],
        "channel_guest_setup" => &[
            "owner_id",
            "revision",
            "pending_activation",
            "policy_revision",
            "updated_at",
        ],
        "channel_messages" => &[
            "user_id",
            "conversation_id",
            "direction",
            "platform",
            "content_type",
            "callback_status",
            "created_at",
        ],
        "channel_reply_deliveries" => &["user_id", "created_at"],
        "channel_text_deliveries" => &["user_id", "created_at"],
        "channel_whatsapp_native" => &["owner_id", "updated_at"],
        "cli_pairings" => &[
            "user_id",
            "kind",
            "status",
            "created_at",
            "completed_at",
            "expires_at",
        ],
        "connect_links" => &[
            "user_id",
            "service_id",
            "service_slug",
            "status",
            "created_at",
            "completed_at",
            "expires_at",
        ],
        "consents" => &["user_id", "expires_at"],
        "credit_grants" => &[
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
        "credit_schedule_recipients" => &[
            "schedule_id",
            "recipient_user_id",
            "anchor_at",
            "next_due_at",
            "is_eligible",
            "created_at",
            "updated_at",
        ],
        "device_codes" => &["approved_by_user_id", "status", "created_at", "expires_at"],
        "device_onboard_credentials" => &["owner_user_id", "created_at", "expires_at"],
        "encrypted_record_migration_findings" => &["collection", "category", "observed_at"],
        "encrypted_record_migration_runs" => &["phase", "scanned", "rewritten", "started_at"],
        "execution_permission_deliveries" => &[
            "user_id",
            "conversation_id",
            "state",
            "created_at",
            "expires_at",
        ],
        "feature_flag_overrides" => &["created_at", "updated_at"],
        "image_publications" => &[
            "user_id",
            "conversation_id",
            "company",
            "state",
            "stage",
            "post",
            "verification",
            "created_at",
            "updated_at",
        ],
        "invite_codes" => &[
            "usages.user_id",
            "usages.used_at",
            "created_at",
            "updated_at",
        ],
        "invocation_diagnostics" => &[
            "actor_user_id",
            "state",
            "created_at",
            "completed_at",
            "expires_at",
        ],
        "lark_base_write_photos" => &["user_id", "created_at", "expires_at"],
        "lark_base_writes" => &["user_id", "state", "created_at", "updated_at", "expires_at"],
        "login_approvals" => &["user_id", "expires_at"],
        "login_codes" => &["user_id", "status", "created_at", "expires_at"],
        "machine_confirmations" => &["user_id", "node_id", "state", "created_at", "expires_at"],
        "machine_free_slots" => &["machine_id", "state", "created_at", "updated_at"],
        "machine_jobs" => &["user_id", "node_id", "state", "created_at", "expires_at"],
        "machine_orphan_inventory" => &[
            "owner_id",
            "machine_id",
            "state",
            "reason",
            "created_at",
            "last_seen_at",
        ],
        "mcp_session_notifications" => &["created_at", "expires_at"],
        "mcp_session_slots" => &["user_id", "expires_at"],
        "mfa_challenges" => &["user_id", "created_at", "expires_at"],
        "mfa_factors" => &["user_id", "is_active", "created_at", "updated_at"],
        "node_registration_tokens" => &["user_id", "name", "created_at", "expires_at"],
        "nodes" => &[
            "user_id",
            "name",
            "status",
            "is_active",
            "created_at",
            "updated_at",
        ],
        "notification_channels" => &["user_id", "created_at", "updated_at"],
        "nyxbot_channels" => &["user_id", "status", "created_at", "updated_at"],
        "nyxbot_event_bindings" => &["user_id", "status", "created_at", "updated_at"],
        "nyxbot_events" => &[
            "user_id",
            "conversation_id",
            "status",
            "created_at",
            "expires_at",
        ],
        "nyxbot_gateway_responses" => &["user_id", "conversation_id", "created_at", "expires_at"],
        "nyxbot_image_deliveries" => &["user_id", "conversation_id", "state", "created_at"],
        "nyxbot_image_deliveries_v2" => &["user_id", "conversation_id", "state", "created_at"],
        "nyxbot_threads" => &[
            "user_id",
            "conversation_id",
            "kind",
            "created_at",
            "updated_at",
        ],
        "nyxbot_watches" => &[
            "user_id",
            "conversation_id",
            "kind",
            "status",
            "created_at",
            "expires_at",
        ],
        "oauth_broker_bindings" => &["user_id", "created_at"],
        "oauth_clients" => &[
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
        ],
        "oauth_consent_requests" => &["user_id", "expires_at"],
        "oauth_delegation_authorities" => &["user_id", "created_at", "expires_at"],
        "oauth_states" => &["user_id", "created_at", "expires_at"],
        "openclaw_channel_mappings" => &[
            "nyxid_user_id",
            "channel",
            "channel_user_id",
            "created_at",
            "updated_at",
        ],
        "oracle_pools" => &[
            "user_id",
            "slug",
            "name",
            "created_at",
            "updated_at",
            "deleted_at",
            "is_active",
        ],
        "org_invites" => &["role", "created_at", "expires_at"],
        "org_memberships" => &["org_user_id", "member_user_id", "role", "created_at"],
        "permission_key_policies" => &["user_id", "created_at", "updated_at"],
        "platform_bot_cleanup" => &["owner_user_id", "kind", "created_at"],
        "platform_bot_configs" => &["created_at", "updated_at", "deleted_at", "enabled"],
        "platform_bot_installations" => &["owner_user_id", "state", "created_at", "updated_at"],
        "platform_delivery_tests" => &["state", "expires_at"],
        "platform_discord_control_installation_history" => &["state", "created_at", "updated_at"],
        "platform_discord_control_installations" => &["state", "created_at", "updated_at"],
        "platform_guide_selections" => &["binding_id", "generation", "setup_id", "purge_after"],
        "platform_native_replies" => &["source_id", "kind", "purge_after"],
        "platform_onboarding_binding_requests" => {
            &["approved_user_id", "state", "created_at", "expires_at"]
        }
        "platform_onboarding_bindings" => &[
            "user_id",
            "created_at",
            "updated_at",
            "expires_at",
            "is_active",
        ],
        "platform_personal_decision_spacing" => &["last_decision_at"],
        "platform_personal_setup_selections" => &["user_id"],
        "platform_personal_stop_effects" => &["created_at"],
        "platform_public_deployment_activations" => &["activated_at"],
        "platform_public_deployment_proofs" => &["status"],
        "platform_public_probe_consents" => &["expires_at"],
        "platform_public_readiness_probes" => &["state", "expires_at"],
        "platform_setup_notices" => &["kind", "expires_at"],
        "platform_setup_steps" => &["created_at"],
        "platform_setups" => &[
            "owner_user_id",
            "state",
            "created_at",
            "updated_at",
            "expires_at",
        ],
        "refresh_tokens" => &["user_id", "created_at", "expires_at"],
        "saved_logins" => &["user_id", "created_at", "updated_at"],
        "service_account_key_read_grants" => &["owner_id", "expires_at"],
        "service_accounts" => &[
            "owner_user_id",
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
        ],
        "service_approval_configs" => &["user_id", "service_id", "created_at", "updated_at"],
        "service_change_events" => &["service_id", "actor.app_id"],
        "service_pool_member_health" => &["owner_id", "updated_at", "expires_at"],
        "service_pools" => &[
            "user_id",
            "slug",
            "name",
            "is_active",
            "created_at",
            "updated_at",
        ],
        "service_preferences" => &["version", "ordered", "updated_at"],
        "sessions" => &[
            "user_id",
            "purpose",
            "ip_address",
            "user_agent",
            "revoked",
            "created_at",
            "expires_at",
            "last_active_at",
        ],
        "telegram_bot_claims" => &[
            "owner_user_id",
            "actor_user_id",
            "status",
            "created_at",
            "expires_at",
        ],
        "telegram_bot_requests" => &[
            "owner_user_id",
            "actor_user_id",
            "status",
            "created_at",
            "expires_at",
        ],
        "telegram_managed_bot_events" => &["created_at"],
        "trigger_deliveries" => &[
            "user_id",
            "status",
            "created_at",
            "updated_at",
            "expires_at",
        ],
        "trigger_owner_limits" => &["version"],
        "trigger_runs" => &["user_id", "expires_at"],
        "trigger_work" => &["trigger_id", "user_id", "kind", "at"],
        "triggers" => &[
            "user_id",
            "label",
            "status",
            "source",
            "schedule.kind",
            "schedule.expression",
            "schedule.timezone",
            "schedule.amount",
            "schedule.unit",
            "schedule.anchor",
            "schedule.at",
            "schedule.start",
            "schedule.end",
            "schedule.max_runs",
            "schedule.grace_seconds",
            "schedule_state",
            "overlap",
            "delivery.type",
            "delivery.url",
            "delivery.conversation_id",
            "delivery.confirmation_policy",
            "delivery.agent_id",
            "delivery.thread_policy",
            "delivery.instruction",
            "delivery.deliver_to",
            "created_at",
            "updated_at",
        ],
        "usage_meter" => &[
            "billing_owner_id",
            "service_id",
            "service_slug",
            "metric",
            "quantity",
            "status",
            "created_at",
        ],
        "usage_workspaces" => &["config", "revision", "updated_at"],
        "user_api_keys" => &[
            "user_id",
            "status",
            "created_at",
            "updated_at",
            "expires_at",
        ],
        "user_endpoints" => &[
            "user_id",
            "label",
            "url",
            "catalog_service_id",
            "openapi_spec_url",
            "recommended_skills",
            "created_at",
            "updated_at",
        ],
        "user_provider_credentials" => &["user_id", "created_at", "updated_at"],
        "user_provider_tokens" => &[
            "user_id",
            "status",
            "created_at",
            "updated_at",
            "expires_at",
        ],
        "user_service_connections" => &[
            "user_id",
            "service_id",
            "credential_type",
            "credential_label",
            "metadata",
            "is_active",
            "created_at",
            "updated_at",
        ],
        "user_services" => &[
            "user_id",
            "slug",
            "endpoint_id",
            "catalog_service_id",
            "service_type",
            "is_active",
            "created_at",
            "updated_at",
        ],
        "assistant_voice_sessions" => &[
            "user_id",
            "conversation_id",
            "protocol",
            "dictation",
            "measured_ms",
            "state",
            "created_at",
            "deadline",
            "last_user_at",
            "closed_at",
            "observed_seconds",
            "playback_ms",
            "final_usage_confirmed",
            "billing_finalized",
        ],
        "generated_artifact_content_series" => {
            &["user_id", "conversation_id", "revision", "purge_after"]
        }
        "oracle_login_snapshots" => &[
            "created_by_user_id",
            "format_version",
            "envelope_size",
            "created_at",
            "expires_at",
        ],
        "oracle_sessions" => &[
            "owner_user_id",
            "origin",
            "tag",
            "turn_count",
            "last_task_id",
            "chatgpt_url",
            "owner_worker_label",
            "closed_at",
            "created_at",
            "updated_at",
        ],
        "oracle_tasks" => &[
            "submitter_user_id",
            "kind",
            "target_url",
            "prompt",
            "model_label",
            "project_url",
            "tag",
            "pdf_name",
            "attachment_name",
            "status",
            "conversation_id",
            "required_worker_label",
            "phase",
            "phase_detail",
            "response",
            "response_chars",
            "chatgpt_url",
            "failure_reason",
            "observed_model_switcher",
            "observed_model_effort",
            "created_at",
            "updated_at",
            "completed_at",
            "expires_at",
        ],
        "oracle_worker_commands" => &[
            "created_by_user_id",
            "worker_label",
            "kind",
            "status",
            "required_capability",
            "delivery_count",
            "result_code",
            "bundle_version",
            "delivered_at",
            "completed_at",
            "deadline_at",
            "expires_at",
            "created_at",
            "updated_at",
        ],
        "oracle_workers" => &[
            "enrollment.owner_user_id",
            "worker_label",
            "generation",
            "last_seen_at",
            "current_task_id",
            "script_version",
            "page_url",
            "first_seen_at",
            "provisioned_at",
            "platform",
            "capabilities",
            "desired_state",
            "logged_in",
            "chrome_alive",
            "last_error",
            "cooldown_until",
        ],
        "org_channel_lanes" => &["actor_id", "conversation_id", "created_at"],
        "org_channel_link_challenges" => {
            &["user_id", "org_id", "state", "created_at", "expires_at"]
        }
        "org_channel_messages" => &[
            "channel_id",
            "conversation_id",
            "role",
            "text",
            "created_at",
        ],
        "org_channel_sender_links" => &["user_id", "channel_id", "org_id", "active", "created_at"],
        "platform_personal_bindings" => &[
            "user_id",
            "conversation_id",
            "state",
            "created_at",
            "updated_at",
        ],
        "platform_personal_link_challenges" => &["state", "created_at", "expires_at"],
        "platform_personal_link_intents" => &["state", "created_at", "expires_at"],
        "users" => &[
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
            "profile_config.quick_access",
            "profile_config.onboarding.ai_services_completed_at",
        ],
        "audit_log" => &["event_type", "created_at"],
        _ => return Err(ExportSpecError(collection.to_owned())),
    })
}
pub(super) fn owner_filter(
    entry: &personal_data_registry::Entry,
    account_id: &str,
) -> Option<Document> {
    use personal_data_registry::ExportSelector;
    Some(match entry.export_selector {
        ExportSelector::None => return None,
        ExportSelector::Person(field) => doc! {field: account_id},
        ExportSelector::AnyPerson(fields) => {
            doc! {"$or": fields.iter().map(|field| doc! {*field: account_id}).collect::<Vec<_>>()}
        }
        // ServiceAccount::effective_owner_user_id uses the creator only for
        // legacy rows without an explicit owner. A creator does not own an org's row.
        ExportSelector::LegacyServiceAccount => doc! {"$or": [
            {"owner_user_id": account_id},
            {"owner_user_id": bson::Bson::Null, "created_by": account_id}
        ]},
        ExportSelector::AuthoredOwnedGroup => {
            doc! {"user_id": account_id, "$or": [
                {"author_user_id": account_id},
                {"role":"user", "author_user_id":Bson::Null, "collaboration":Bson::Null, "org_group":{"$ne":true}}
            ]}
        }
        // Resolved once, before querying children; never a child-side lookup.
        ExportSelector::JoinedPerson { .. } => return None,
    })
}

fn private_conversation_filter(prefix: &str) -> Document {
    let mut filter = doc! {};
    for field in ["collaboration", "platform_thread", "channel", "group_id"] {
        filter.insert(format!("{prefix}{field}"), Bson::Null);
    }
    for field in ["shared_agent_thread", "guest_turn", "invocation_controlled"] {
        filter.insert(format!("{prefix}{field}"), doc! {"$ne": true});
    }
    filter
}

fn parent_scope_sort(collection: &str) -> Document {
    match collection {
        "assistant_conversations" | "platform_setups" => doc! {"updated_at":-1,"_id":-1},
        "mcp_sessions" => doc! {"user_id":1},
        "platform_bot_installations" => doc! {"owner_user_id":1},
        _ => doc! {"_id":1},
    }
}

/// Resolve only projected identities and cap the in-memory $in list. Parent
/// owner predicates and child join keys have existing prefix indexes.
async fn resolve_ids(
    db: &mongodb::Database,
    collection: &str,
    filter: Document,
) -> AppResult<(Vec<Bson>, bool)> {
    let mut cursor = db
        .collection::<Document>(collection)
        .find(filter)
        .projection(doc! {"_id": 1})
        .sort(parent_scope_sort(collection))
        .limit(PARENT_ID_CAP as i64 + 1)
        .batch_size(QUERY_BATCH_SIZE)
        .max_time(QUERY_TIMEOUT)
        .await?;
    let mut ids = Vec::new();
    while let Some(row) = cursor.try_next().await? {
        if ids.len() == PARENT_ID_CAP {
            return Ok((ids, true));
        }
        if let Some(id) = row.get("_id") {
            ids.push(id.clone());
        }
    }
    Ok((ids, false))
}

async fn joined_parent_ids(
    db: &mongodb::Database,
    parent: &str,
    owner: Document,
) -> AppResult<(Vec<Bson>, bool)> {
    let (mut ids, truncated) = resolve_ids(db, parent, owner).await?;
    if parent == crate::models::mcp_session::MCP_SESSION_COLLECTION {
        // A legacy parent can migrate between scope resolution and the child query.
        let successors: Vec<_> = ids
            .iter()
            .filter_map(Bson::as_str)
            .filter(|id| uuid::Uuid::try_parse(id).is_ok())
            .map(|id| Bson::String(super::mcp_session_storage::hash(id)))
            .collect();
        ids.extend(successors);
    }
    Ok((ids, truncated))
}

/// Resolve private scope once per export, rather than joining each message.
async fn message_scope(db: &mongodb::Database, person: &str) -> AppResult<(Document, bool)> {
    let mut private = private_conversation_filter("");
    private.insert("user_id", person);
    let (ids, truncated) = resolve_ids(db, "assistant_conversations", private).await?;
    Ok((
        doc! {"$and": [{"user_id": person}, {"$or": [
            {"role":"user", "author":"owner"},
            {"conversation_id":{"$in":ids}, "author":{"$ne":"guest"}, "origin":{"$in":[Bson::Null,"user"]},
                "platform_origin":Bson::Null, "platform_reply":Bson::Null, "role":{"$in":["user","assistant"]}}
        ]}]},
        truncated,
    ))
}

fn source_pipeline(collection: &str, filter: Document) -> Vec<Document> {
    let mut pipeline = Vec::new();
    pipeline.push(doc! {"$match": filter});
    if collection == "assistant_conversations" {
        // Shared titles can originate from a guest's first message.
        pipeline.push(doc! {"$match": private_conversation_filter("")});
    }

    pipeline
}

fn restrict_row(collection: &str, filter: &Document, row: &mut Document) {
    if collection == "invite_codes"
        && let (Ok(person), Some(Bson::Array(usages))) =
            (filter.get_str("usages.user_id"), row.get_mut("usages"))
    {
        usages.retain(|usage| {
            usage
                .as_document()
                .and_then(|usage| usage.get_str("user_id").ok())
                == Some(person)
        });
    }
}

#[derive(Default)]
struct ExportBudget {
    remaining: usize,
    byte_truncated: bool,
}

impl ExportBudget {
    fn new(max_bytes: usize) -> Self {
        // Leave room for the fixed envelope, section names, and truncation
        // metadata. The final cap pass remains a correctness backstop.
        Self {
            remaining: max_bytes.saturating_sub(64 * 1024),
            ..Default::default()
        }
    }

    fn admit(&mut self, value: &serde_json::Value) -> bool {
        let bytes = serde_json::to_vec(value)
            .map(|value| value.len())
            .unwrap_or(usize::MAX);
        let needed = bytes.saturating_add(1);
        if needed > self.remaining {
            self.byte_truncated = true;
            return false;
        }
        self.remaining -= needed;
        true
    }
}

/// Four total reads per source at most: one equal-share pass and up to three
/// redistributions. Only byte-limited sources compete for unused capacity.
const ALLOCATION_ROUNDS: usize = 4;

#[derive(Default)]
struct ExportPage {
    value: ExportSubsection,
    used: usize,
    byte_truncated: bool,
}

impl ExportPage {
    fn from_read(value: ExportSubsection, limit: usize, budget: ExportBudget) -> Self {
        if value.unavailable {
            // A failed cursor may have consumed part of the budget before its
            // rows were discarded. None of those bytes belong to the result.
            return Self {
                value,
                ..Default::default()
            };
        }
        Self {
            byte_truncated: budget.byte_truncated
                && value.items.len() < SUBSECTION_ROW_CAP as usize,
            used: limit - budget.remaining,
            value,
        }
    }
}

async fn allocate_pages_until<F, Fut>(
    max_bytes: usize,
    count: usize,
    deadline: tokio::time::Instant,
    mut read: F,
) -> AppResult<Vec<ExportSubsection>>
where
    F: FnMut(usize, usize) -> Fut,
    Fut: std::future::Future<Output = AppResult<ExportPage>>,
{
    let total = ExportBudget::new(max_bytes).remaining;
    let mut pages = Vec::with_capacity(count);
    let mut limits = vec![total / count; count];
    for (index, limit) in limits.iter().enumerate() {
        pages.push(read_page_until(deadline, || read(index, *limit)).await?);
    }
    for _ in 1..ALLOCATION_ROUNDS {
        let active = pages.iter().filter(|page| page.byte_truncated).count();
        if active == 0 || tokio::time::Instant::now() >= deadline {
            break;
        }
        let fixed: usize = pages
            .iter()
            .filter(|page| !page.byte_truncated)
            .map(|page| page.used)
            .sum();
        // Include the active sources' old shares in this pool exactly once.
        // Adding all unused bytes to their old limits would double-count the
        // partially unused space at a row boundary and could exceed the cap.
        let limit = total.saturating_sub(fixed) / active;
        let mut progressed = false;
        for index in 0..count {
            if tokio::time::Instant::now() >= deadline {
                break;
            }
            if !pages[index].byte_truncated || limit <= limits[index] {
                continue;
            }
            // Release the old page before reading its replacement. Retained
            // payload plus the new cursor page stays within the total budget.
            pages[index] = ExportPage::default();
            limits[index] = limit;
            pages[index] = read_page_until(deadline, || read(index, limit)).await?;
            progressed = true;
        }
        if !progressed {
            break;
        }
    }
    Ok(pages.into_iter().map(|page| page.value).collect())
}

async fn read_page_until<F, Fut>(deadline: tokio::time::Instant, read: F) -> AppResult<ExportPage>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = AppResult<ExportPage>>,
{
    if tokio::time::Instant::now() < deadline
        && let Ok(page) = tokio::time::timeout_at(deadline, read()).await
    {
        return page;
    }
    Ok(ExportPage {
        value: ExportSubsection {
            unavailable: true,
            ..Default::default()
        },
        ..Default::default()
    })
}

#[cfg(test)]
async fn allocate_pages<F, Fut>(
    max_bytes: usize,
    count: usize,
    read: F,
) -> AppResult<Vec<ExportSubsection>>
where
    F: FnMut(usize, usize) -> Fut,
    Fut: std::future::Future<Output = AppResult<ExportPage>>,
{
    allocate_pages_until(
        max_bytes,
        count,
        tokio::time::Instant::now() + BUILD_BUDGET,
        read,
    )
    .await
}

fn truncated_names(export: &AccountExport) -> BTreeSet<String> {
    export
        .sections
        .iter()
        .flat_map(|(section, subsections)| {
            subsections
                .iter()
                .filter(|(_, value)| value.truncated)
                .map(move |(subsection, _)| format!("{section}.{subsection}"))
        })
        .collect()
}

async fn rows(
    db: &mongodb::Database,
    collection: &str,
    filter: Document,
    fields: &[&str],
    sort: Document,
    budget: &mut ExportBudget,
) -> AppResult<ExportSubsection> {
    let prepared = if collection == "assistant_messages" {
        if let Ok(person) = filter.get_str("user_id") {
            message_scope(db, person).await.map(|(filter, _)| filter)
        } else {
            Ok(filter)
        }
    } else {
        Ok(filter)
    };
    let result = match prepared {
        Ok(filter) => query_rows(db, collection, filter, fields, sort, budget).await,
        Err(error) => Err(error),
    };
    match result {
        Ok(value) => Ok(value),
        Err(_) => {
            // Do not expose database diagnostics or content. Every source has
            // an independent budget, so a failed source cannot starve others.
            tracing::warn!(collection, "Account export subsection unavailable");
            Ok(ExportSubsection {
                unavailable: true,
                ..Default::default()
            })
        }
    }
}

fn unavailable_names(export: &AccountExport) -> Vec<String> {
    export
        .sections
        .iter()
        .flat_map(|(section, values)| {
            values
                .iter()
                .filter(|(_, value)| value.unavailable)
                .map(move |(name, _)| format!("{section}.{name}"))
        })
        .collect()
}

async fn query_rows(
    db: &mongodb::Database,
    collection: &str,
    filter: Document,
    fields: &[&str],
    sort: Document,
    budget: &mut ExportBudget,
) -> AppResult<ExportSubsection> {
    let base = source_pipeline(collection, filter.clone());
    let mut count_pipeline = base.clone();
    // Stop after one sentinel beyond the delivery cap. Counting every match
    // defeats bounded exports when residual predicates require fetching bodies.
    count_pipeline.push(doc! {"$limit": SUBSECTION_ROW_CAP + 1});
    count_pipeline.push(doc! {"$count": "count"});
    let total_count = db
        .collection::<Document>(collection)
        .aggregate(count_pipeline)
        .max_time(QUERY_TIMEOUT)
        .await?
        .try_next()
        .await?
        .and_then(|row| {
            row.get_i64("count")
                .ok()
                .or_else(|| row.get_i32("count").ok().map(i64::from))
        })
        .unwrap_or(0) as u64;
    let mut projection = Document::new();
    for field in fields {
        projection.insert(*field, 1);
    }
    let mut pipeline = base;
    pipeline.push(doc! {"$sort": sort});
    pipeline.push(doc! {"$limit": SUBSECTION_ROW_CAP + 1});
    pipeline.push(doc! {"$project": projection});
    let mut cursor = db
        .collection::<Document>(collection)
        .aggregate(pipeline)
        .batch_size(QUERY_BATCH_SIZE)
        .max_time(QUERY_TIMEOUT)
        .await?;
    let mut items = Vec::new();
    let mut truncated = total_count > SUBSECTION_ROW_CAP as u64;
    while let Some(mut row) = cursor.try_next().await? {
        restrict_row(collection, &filter, &mut row);
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
        total_count_exact: total_count <= SUBSECTION_ROW_CAP as u64,
        truncated,
        unavailable: false,
        scope_truncated: false,
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
    let mut truncated = truncated_names(export);
    let skeleton = AccountExport {
        format: export.format,
        version: export.version,
        generated_at: export.generated_at.clone(),
        account_id: export.account_id.clone(),
        limits: export.limits.clone(),
        excluded: export.excluded.clone(),
        truncated_sections: truncated.iter().cloned().collect(),
        unavailable_sections: export.unavailable_sections.clone(),
        sections: export
            .sections
            .iter()
            .map(|(section, values)| {
                (
                    section.clone(),
                    values
                        .iter()
                        .map(|(name, value)| {
                            (
                                name.clone(),
                                ExportSubsection {
                                    items: Vec::new(),
                                    total_count: value.total_count,
                                    total_count_exact: value.total_count_exact,
                                    truncated: value.truncated,
                                    unavailable: value.unavailable,
                                    scope_truncated: value.scope_truncated,
                                },
                            )
                        })
                        .collect(),
                )
            })
            .collect(),
    };
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

async fn build_until(
    db: &mongodb::Database,
    account_id: &str,
    max_bytes: usize,
    deadline: tokio::time::Instant,
) -> AppResult<AccountExport> {
    let mut sections = empty_sections();
    let account = if tokio::time::Instant::now() < deadline {
        match tokio::time::timeout_at(deadline, async { db.collection::<Document>(crate::models::user::COLLECTION_NAME).find_one(doc! {"_id": account_id}).projection(doc! {"_id": 1, "email": 1, "display_name": 1, "avatar_url": 1, "email_verified": 1, "mfa_enabled": 1, "social_provider": 1, "created_at": 1, "updated_at": 1, "last_login_at": 1, "profile_config": 1}).max_time(QUERY_TIMEOUT).await }).await {
            Ok(result) => Some(result?.ok_or_else(|| AppError::NotFound("User not found".into()))?),
            Err(_) => None,
        }
    } else {
        None
    };
    let mut sources = vec![(
        "users",
        ExplicitExportSpec {
            section: "account",
            subsection: "profile",
            fields: PROFILE_FIELDS,
        },
        Some(doc! {"_id": account_id}),
    )];
    for entry in personal_data_registry::entries() {
        let personal_data_registry::Export::Section(section) = entry.export else {
            continue;
        };
        if entry.collection == crate::models::user::COLLECTION_NAME {
            continue;
        }
        let spec = explicit_export_spec(entry.collection)
            .map_err(|error| AppError::Internal(error.to_string()))?;
        debug_assert_eq!(section, spec.section);
        sources.push((entry.collection, spec, owner_filter(entry, account_id)));
    }
    for view in personal_data_registry::ADDITIONAL_EXPORT_VIEWS {
        sources.push((
            view.collection,
            additional_export_spec(view),
            Some(additional_export_filter(view, account_id)),
        ));
    }
    let mut scope_flags = vec![(false, false); sources.len()];
    let mut parent_scopes = BTreeMap::new();
    for (index, (collection, _, filter)) in sources.iter_mut().enumerate() {
        if let Some(personal_data_registry::Entry {
            export_selector:
                personal_data_registry::ExportSelector::JoinedPerson {
                    parent,
                    field,
                    owner_field,
                },
            ..
        }) = personal_data_registry::find(collection)
        {
            let key = (*parent, *owner_field);
            if let std::collections::btree_map::Entry::Vacant(scope) = parent_scopes.entry(key) {
                let resolved = if tokio::time::Instant::now() < deadline {
                    tokio::time::timeout_at(
                        deadline,
                        joined_parent_ids(db, parent, doc! {*owner_field: account_id}),
                    )
                    .await
                    .ok()
                    .and_then(Result::ok)
                } else {
                    None
                };
                scope.insert(resolved);
            }
            if let Some((ids, truncated)) = &parent_scopes[&key] {
                *filter = Some(doc! {*field: {"$in": ids}});
                scope_flags[index].0 = *truncated;
            } else {
                scope_flags[index].1 = true;
            }
        }
    }
    for (index, (collection, _, filter)) in sources.iter_mut().enumerate() {
        if *collection == "assistant_messages" {
            match tokio::time::timeout_at(deadline, async {
                if tokio::time::Instant::now() >= deadline {
                    return Err(AppError::Internal("Export build budget exhausted".into()));
                }
                message_scope(db, account_id).await
            })
            .await
            {
                Ok(Ok((resolved, truncated))) => {
                    *filter = Some(resolved);
                    scope_flags[index].0 = truncated;
                }
                _ => scope_flags[index].1 = true,
            }
        }
    }
    let pages = allocate_pages_until(max_bytes, sources.len(), deadline, |index, limit| {
        let (collection, spec, filter) = &sources[index];
        let account = &account;
        let (scope_truncated, unavailable) = scope_flags[index];
        async move {
            let mut budget = ExportBudget {
                remaining: limit,
                ..Default::default()
            };
            let mut value = if unavailable {
                ExportSubsection {
                    unavailable: true,
                    ..Default::default()
                }
            } else if let Some(account) = account.as_ref().filter(|_| *collection == "users") {
                let profile = safe_item(account, PROFILE_FIELDS);
                let admitted = budget.admit(&profile);
                ExportSubsection {
                    items: admitted.then_some(profile).into_iter().collect(),
                    total_count: 1,
                    total_count_exact: true,
                    truncated: !admitted,
                    unavailable: false,
                    scope_truncated: false,
                }
            } else if let Some(filter) = filter {
                rows(
                    db,
                    collection,
                    filter.clone(),
                    spec.fields,
                    doc! {"created_at": -1, "_id": -1},
                    &mut budget,
                )
                .await?
            } else {
                ExportSubsection::default()
            };
            value.scope_truncated = scope_truncated;
            value.total_count_exact &= !scope_truncated;
            value.truncated |= scope_truncated;
            Ok(ExportPage::from_read(value, limit, budget))
        }
    })
    .await?;
    for ((_, spec, _), page) in sources.into_iter().zip(pages) {
        sections
            .entry(spec.section.into())
            .or_default()
            .insert(spec.subsection.into(), page);
    }
    let mut export = AccountExport {
        format: FORMAT,
        version: VERSION,
        generated_at: Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        account_id: account_id.to_owned(),
        limits: ExportLimits {
            max_bytes,
            subsection_row_cap: SUBSECTION_ROW_CAP,
            query_timeout_ms: QUERY_TIMEOUT.as_millis() as u64,
            build_budget_ms: BUILD_BUDGET.as_millis() as u64,
            file_payloads: if FILE_PAYLOADS_METADATA_ONLY {
                "metadata_only"
            } else {
                "included"
            },
        },
        truncated_sections: Vec::new(),
        unavailable_sections: Vec::new(),
        excluded: vec![
            "password and authentication hashes",
            "tokens, keys, secrets, ciphertext, signatures and provider credentials",
            "raw uploaded/generated file payloads (metadata only)",
            "transient coordination, lease, deduplication, erasure and retained integrity records",
        ],
        sections,
    };
    export.unavailable_sections = unavailable_names(&export);
    enforce_cap_with_limit(&mut export, max_bytes);
    Ok(export)
}

pub async fn build_admitted(
    db: &mongodb::Database,
    account_id: &str,
    admitted_at: tokio::time::Instant,
) -> AppResult<AccountExport> {
    build_until(db, account_id, MAX_BYTES, admitted_at + BUILD_BUDGET).await
}

#[cfg(test)]
async fn build(db: &mongodb::Database, account_id: &str) -> AppResult<AccountExport> {
    build_admitted(db, account_id, tokio::time::Instant::now()).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::user::UserType;
    use std::str::FromStr;

    #[tokio::test(start_paused = true)]
    async fn total_build_budget_cancels_slow_sources_and_marks_unread_sections_unavailable() {
        let start = tokio::time::Instant::now();
        let calls = std::cell::RefCell::new(Vec::new());
        let pages = allocate_pages_until(MAX_BYTES, 4, start + BUILD_BUDGET, |index, limit| {
            calls.borrow_mut().push(index);
            async move {
                if index == 1 {
                    tokio::time::sleep(BUILD_BUDGET * 2).await;
                }
                Ok(allocation_page(limit, 1, 10, false))
            }
        })
        .await
        .unwrap();
        assert_eq!(tokio::time::Instant::now() - start, BUILD_BUDGET);
        assert_eq!(*calls.borrow(), vec![0, 1]);
        assert!(!pages[0].unavailable);
        assert!(pages[1..].iter().all(|page| page.unavailable));
        let mut export = AccountExport {
            format: FORMAT,
            version: VERSION,
            generated_at: String::new(),
            account_id: String::new(),
            limits: ExportLimits {
                max_bytes: MAX_BYTES,
                subsection_row_cap: SUBSECTION_ROW_CAP,
                query_timeout_ms: 3000,
                build_budget_ms: 60_000,
                file_payloads: "metadata_only",
            },
            sections: BTreeMap::new(),
            truncated_sections: Vec::new(),
            unavailable_sections: Vec::new(),
            excluded: Vec::new(),
        };
        for (index, page) in pages.into_iter().enumerate() {
            export
                .sections
                .entry("test".into())
                .or_default()
                .insert(index.to_string(), page);
        }
        assert_eq!(
            unavailable_names(&export),
            vec!["test.1", "test.2", "test.3"]
        );
    }

    #[tokio::test]
    async fn elapsed_admission_budget_finalizes_all_sources_as_unavailable() {
        let db =
            crate::test_utils::connect_transaction_test_database("export_elapsed_admission").await;
        let start = tokio::time::Instant::now();
        let export = build_admitted(&db, "person", start - BUILD_BUDGET)
            .await
            .unwrap();
        assert!(start.elapsed() < Duration::from_secs(1));
        let count: usize = export.sections.values().map(BTreeMap::len).sum();
        assert!(count > 100);
        assert_eq!(export.unavailable_sections.len(), count);
        assert!(
            export
                .sections
                .values()
                .flat_map(BTreeMap::values)
                .all(|page| page.unavailable)
        );
        assert_eq!(export.limits.build_budget_ms, 60_000);
        db.drop().await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn total_build_budget_cancels_redistribution_and_retains_other_pages() {
        let start = tokio::time::Instant::now();
        let calls = std::cell::RefCell::new(Vec::new());
        let pages = allocate_pages_until(MAX_BYTES, 3, start + BUILD_BUDGET, |index, limit| {
            let reread = calls.borrow().contains(&index);
            calls.borrow_mut().push(index);
            async move {
                if reread {
                    tokio::time::sleep(BUILD_BUDGET * 2).await;
                }
                Ok(allocation_page(
                    limit,
                    if index == 0 { 10_000 } else { 1 },
                    4000,
                    false,
                ))
            }
        })
        .await
        .unwrap();
        assert_eq!(tokio::time::Instant::now() - start, BUILD_BUDGET);
        assert_eq!(*calls.borrow(), vec![0, 1, 2, 0]);
        assert!(pages[0].unavailable);
        assert!(
            pages[1..]
                .iter()
                .all(|page| !page.unavailable && page.items.len() == 1)
        );
    }

    #[test]
    fn shared_attribution_does_not_grant_export_ownership() {
        for collection in ["user_services", "oauth_states"] {
            assert_eq!(
                owner_filter(personal_data_registry::find(collection).unwrap(), "person"),
                Some(doc! {"user_id": "person"})
            );
        }
    }

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
    fn url_sanitizer_reduces_every_url_to_origin() {
        let url = sanitize_url(
            "https://user:URL_USERINFO@api.example.test/v1/PATH_TOKEN?api_key=URL_QUERY#URL_FRAGMENT",
        );
        assert_eq!(url, "https://api.example.test");
        assert!(!url.contains("URL_USERINFO"));
        assert!(!url.contains("URL_QUERY"));
        assert!(!url.contains("URL_FRAGMENT"));
        assert!(!url.contains("PATH_TOKEN"));
        let endpoint =
            sanitize_url("https://user:pass@api.example.test:8443/v1?token=secret#fragment");
        assert_eq!(endpoint, "https://api.example.test:8443");
    }

    #[test]
    fn export_budget_stops_admitting_rows_at_the_byte_cap() {
        let mut budget = ExportBudget {
            remaining: 16,
            ..Default::default()
        };
        assert!(budget.admit(&serde_json::json!("123456")));
        assert!(!budget.admit(&serde_json::json!("this row is too large")));
    }

    // Exercise the production allocator with deterministic source pages so its
    // read count, unavailable handling and row/byte distinction are observable.
    fn allocation_page(
        limit: usize,
        rows: usize,
        row_bytes: usize,
        unavailable: bool,
    ) -> ExportPage {
        let mut budget = ExportBudget {
            remaining: limit,
            ..Default::default()
        };
        let mut value = ExportSubsection {
            total_count: rows as u64,
            unavailable,
            ..Default::default()
        };
        if !unavailable {
            for _ in 0..rows.min(SUBSECTION_ROW_CAP as usize) {
                let item = serde_json::json!({"text": "x".repeat(row_bytes)});
                if !budget.admit(&item) {
                    break;
                }
                value.items.push(item);
            }
            value.truncated = value.items.len() < rows;
        }
        ExportPage::from_read(value, limit, budget)
    }

    #[tokio::test]
    async fn water_filling_reclaims_small_sources_and_splits_large_sources_equally() {
        for large in [1, 2] {
            let count = 20;
            let cap = 1024 * 1024;
            let mut reads = vec![0; count];
            let pages = allocate_pages(cap, count, |index, limit| {
                reads[index] += 1;
                std::future::ready(Ok(if index < large {
                    allocation_page(limit, 2000, 1000, false)
                } else {
                    allocation_page(limit, 1, 20, false)
                }))
            })
            .await
            .unwrap();
            let floor = ExportBudget::new(cap).remaining / count;
            let sizes: Vec<_> = pages
                .iter()
                .map(|page| {
                    page.items
                        .iter()
                        .map(|item| serde_json::to_vec(item).unwrap().len() + 1)
                        .sum::<usize>()
                })
                .collect();
            assert!(sizes[0] > floor * 8, "unused shares were not redistributed");
            assert!(sizes.iter().sum::<usize>() <= ExportBudget::new(cap).remaining);
            if large == 2 {
                assert_eq!(sizes[0], sizes[1]);
            }
            assert!(pages[..large].iter().all(|page| page.truncated));
            assert!(
                pages[large..]
                    .iter()
                    .all(|page| !page.truncated && page.items.len() == 1)
            );
            assert!(reads.iter().all(|count| *count <= ALLOCATION_ROUNDS));
            assert!(reads[large..].iter().all(|count| *count == 1));
        }
    }

    #[tokio::test]
    async fn water_filling_does_not_reread_row_capped_or_unavailable_sources() {
        let mut reads = [0; 4];
        let pages = allocate_pages(MAX_BYTES, 4, |index, limit| {
            reads[index] += 1;
            std::future::ready(Ok(match index {
                0 => allocation_page(limit, SUBSECTION_ROW_CAP as usize + 1, 0, false),
                1 => allocation_page(limit, 0, 0, true),
                2 => allocation_page(limit, 1, 10, false),
                _ => allocation_page(limit, 100, 512 * 1024, false),
            }))
        })
        .await
        .unwrap();
        assert_eq!(reads[..3], [1, 1, 1]);
        assert!(reads[3] > 1 && reads[3] <= ALLOCATION_ROUNDS);
        assert_eq!(pages[0].items.len(), SUBSECTION_ROW_CAP as usize);
        assert!(pages[0].truncated);
        assert!(pages[1].unavailable && !pages[1].truncated && pages[1].items.is_empty());
    }

    #[tokio::test]
    async fn water_filling_releases_failed_reread_bytes_and_never_retries_unavailable() {
        let mut reads = [0; 3];
        let pages = allocate_pages(1024 * 1024, 3, |index, limit| {
            reads[index] += 1;
            let mut page = if index == 0 {
                allocation_page(limit, 1, 10, false)
            } else {
                allocation_page(limit, 100, 50_000, false)
            };
            if index == 1 && reads[index] == 2 {
                let mut spent = ExportBudget {
                    remaining: limit,
                    ..Default::default()
                };
                assert!(spent.admit(&serde_json::json!("partially read")));
                page = ExportPage::from_read(
                    ExportSubsection {
                        unavailable: true,
                        ..Default::default()
                    },
                    limit,
                    spent,
                );
                assert_eq!(page.used, 0);
                assert!(!page.byte_truncated);
            }
            std::future::ready(Ok(page))
        })
        .await
        .unwrap();
        assert_eq!(reads, [1, 2, 3]);
        assert!(pages[1].unavailable && pages[1].items.is_empty());
        assert!(pages[2].items.len() > 15);
    }

    #[tokio::test]
    async fn water_filling_stops_after_four_rounds_when_sources_finish_in_stages() {
        let mut reads = [0; 5];
        let pages = allocate_pages(1024 * 1024, 5, |index, limit| {
            reads[index] += 1;
            // Each round releases a different source's unused capacity.
            let bytes = [0, 210_000, 250_000, 350_000, 900_000][index];
            std::future::ready(Ok(allocation_page(limit, 1, bytes, false)))
        })
        .await
        .unwrap();
        assert_eq!(reads, [1, 2, 3, 4, 4]);
        assert!(pages[4].truncated);
        let used: usize = pages
            .iter()
            .flat_map(|page| &page.items)
            .map(|item| serde_json::to_vec(item).unwrap().len() + 1)
            .sum();
        assert!(used <= ExportBudget::new(1024 * 1024).remaining);
    }

    #[test]
    fn per_collection_allowlists_keep_authored_text_and_only_own_memory() {
        let row = doc! {
            "title": "PERSON_TITLE",
            "persona": "PERSON_PERSONA",
            "memory": [{"text": "PERSON_MEMORY"}],
            "notes": [{"text": "PERSON_NOTE"}],
            "comment": "PERSON_FEEDBACK",
            "label": "PERSON_TRIGGER_LABEL",
            "schedule": {"expression": "PERSON_SCHEDULE", "timezone": "UTC"},
            "delivery": {"instruction": "PERSON_TRIGGER_INSTRUCTION", "url": "https://example.test/hook"},
            "description": "PERSON_DESCRIPTION",
            "credential_label": "PERSON_CONNECTION_LABEL",
            "name": "PERSON_NODE_NAME",
            "url": "https://user:pass@example.test/v1?token=URL_QUERY#URL_FRAGMENT",
            "shared_memory": "OTHER_PERSON_SHARED_MEMORY",
        };
        let value = safe_item(&row, fields_for("assistant_conversations").unwrap());
        assert_eq!(value["title"], "PERSON_TITLE");
        let agent = safe_item(&row, fields_for("assistant_agents").unwrap());
        assert_eq!(agent["persona"], "PERSON_PERSONA");
        assert_eq!(agent["memory"]["text"][0], "PERSON_MEMORY");
        assert!(
            safe_item(&row, fields_for("assistant_agent_sharing").unwrap())
                .get("memory")
                .is_none()
        );
        let notes = safe_item(&row, fields_for("assistant_agent_private_notes").unwrap());
        assert_eq!(notes["notes"]["text"][0], "PERSON_NOTE");
        let feedback = safe_item(&row, fields_for("assistant_message_feedback").unwrap());
        assert_eq!(feedback["comment"], "PERSON_FEEDBACK");
        let trigger = safe_item(&row, fields_for("triggers").unwrap());
        assert_eq!(trigger["label"], "PERSON_TRIGGER_LABEL");
        assert_eq!(trigger["schedule"]["expression"], "PERSON_SCHEDULE");
        assert_eq!(
            trigger["delivery"]["instruction"],
            "PERSON_TRIGGER_INSTRUCTION"
        );
        let endpoint = safe_item(&row, fields_for("user_endpoints").unwrap());
        assert_eq!(endpoint["url"], "https://example.test");
        assert_eq!(endpoint["url_path_omitted"], true);
        let connection = safe_item(&row, fields_for("user_service_connections").unwrap());
        assert_eq!(connection["credential_label"], "PERSON_CONNECTION_LABEL");
        let node = safe_item(&row, fields_for("nodes").unwrap());
        assert_eq!(node["name"], "PERSON_NODE_NAME");
        assert!(
            !serde_json::to_string(&agent)
                .unwrap()
                .contains("OTHER_PERSON_SHARED_MEMORY")
        );
    }

    #[test]
    fn shared_agent_memory_and_other_authors_are_outside_person_export() {
        let row = doc! {
            "memory": [{"text": "TEAM_MEMORY", "owner_id": "other-person"}],
            "text": "OTHER_AUTHOR_MESSAGE",
            "author_user_id": "other-person",
            "user_id": "account",
        };
        let sharing = safe_item(&row, &["user_id", "updated_at"]);
        assert!(serde_json::to_string(&sharing).unwrap().contains("account"));
        assert!(
            !serde_json::to_string(&sharing)
                .unwrap()
                .contains("TEAM_MEMORY")
        );
        let group = safe_item(&row, fields_for("assistant_group_messages").unwrap());
        assert_eq!(group["author_user_id"], "other-person");
        assert_eq!(group["text"], "OTHER_AUTHOR_MESSAGE");
        // Explicit authors must match. Null authors are accepted only for
        // owner inputs in personal groups, never collaboration/org content.
        let filter = owner_filter(
            personal_data_registry::find("assistant_group_messages").unwrap(),
            "account",
        )
        .unwrap();
        assert_eq!(
            filter,
            doc! {"user_id":"account", "$or":[
                {"author_user_id":"account"},
                {"role":"user", "author_user_id":Bson::Null, "collaboration":Bson::Null, "org_group":{"$ne":true}}
            ]}
        );
    }

    #[test]
    fn every_url_allowlist_field_uses_the_common_sanitizer() {
        let row = doc! {
            "avatar_url": "https://u:URL_USERINFO@example.test/avatar/PATH_TOKEN?key=URL_QUERY#URL_FRAGMENT",
            "url": "https://u:URL_USERINFO@example.test/v1/safe-path?key=URL_QUERY#URL_FRAGMENT",
            "openapi_spec_url": "https://u:URL_USERINFO@example.test/openapi/PATH_TOKEN?key=URL_QUERY#URL_FRAGMENT",
            "target_url": "https://u:URL_USERINFO@example.test/target/PATH_TOKEN?key=URL_QUERY#URL_FRAGMENT",
            "project_url": "https://u:URL_USERINFO@example.test/project/PATH_TOKEN?key=URL_QUERY#URL_FRAGMENT",
            "page_url": "https://u:URL_USERINFO@example.test/page/PATH_TOKEN?key=URL_QUERY#URL_FRAGMENT",
            "chatgpt_url": "https://u:URL_USERINFO@example.test/chat/PATH_TOKEN?key=URL_QUERY#URL_FRAGMENT",
            "connection_webhook_url": "https://u:URL_USERINFO@example.test/connect/PATH_TOKEN?key=URL_QUERY#URL_FRAGMENT",
            "revocation_webhook_url": "https://u:URL_USERINFO@example.test/revoke/PATH_TOKEN?key=URL_QUERY#URL_FRAGMENT",
            "redirect_uris": ["https://u:URL_USERINFO@example.test/cb/PATH_TOKEN?key=URL_QUERY#URL_FRAGMENT"],
            "delivery": {"url": "https://u:URL_USERINFO@example.test/hook/PATH_TOKEN?key=URL_QUERY#URL_FRAGMENT"},
            "unexpected_link": "https://u:UNEXPECTED_USERINFO@example.test/unexpected/PATH_TOKEN?key=UNEXPECTED_QUERY#UNEXPECTED_FRAGMENT",
        };
        let value = safe_item(
            &row,
            &[
                "avatar_url",
                "url",
                "openapi_spec_url",
                "target_url",
                "project_url",
                "page_url",
                "chatgpt_url",
                "connection_webhook_url",
                "revocation_webhook_url",
                "redirect_uris",
                "delivery.url",
                "unexpected_link",
            ],
        );
        let bytes = serde_json::to_vec(&value).unwrap();
        for sentinel in [
            "URL_USERINFO",
            "URL_QUERY",
            "URL_FRAGMENT",
            "PATH_TOKEN",
            "UNEXPECTED_USERINFO",
            "UNEXPECTED_QUERY",
            "UNEXPECTED_FRAGMENT",
        ] {
            assert!(
                !bytes
                    .windows(sentinel.len())
                    .any(|window| window == sentinel.as_bytes())
            );
        }
        assert_eq!(value["url"], "https://example.test");
        assert_eq!(value["url_path_omitted"], true);
        assert_eq!(value["delivery"]["url"], "https://example.test");
        assert_eq!(value["delivery"]["url_path_omitted"], true);
        assert_eq!(value["unexpected_link"], "https://example.test");
        assert_eq!(value["unexpected_link_path_omitted"], true);
        for field in [
            "avatar_url",
            "openapi_spec_url",
            "target_url",
            "project_url",
            "page_url",
            "chatgpt_url",
            "connection_webhook_url",
            "revocation_webhook_url",
        ] {
            assert_eq!(value[field], "https://example.test");
            assert_eq!(
                value.get(format!("{field}_path_omitted")).unwrap(),
                &serde_json::Value::Bool(true)
            );
        }
    }

    #[test]
    fn every_declared_url_field_drops_distinct_secret_sentinels() {
        let fields = [
            ("avatar_url", PROFILE_FIELDS),
            ("chatgpt_url", fields_for("oracle_sessions").unwrap()),
            ("target_url", fields_for("oracle_tasks").unwrap()),
            ("project_url", fields_for("oracle_tasks").unwrap()),
            ("page_url", fields_for("oracle_workers").unwrap()),
            ("delivery.url", fields_for("triggers").unwrap()),
            ("url", fields_for("user_endpoints").unwrap()),
            ("openapi_spec_url", fields_for("user_endpoints").unwrap()),
            ("redirect_uris", fields_for("oauth_clients").unwrap()),
            (
                "connection_webhook_url",
                fields_for("oauth_clients").unwrap(),
            ),
            (
                "revocation_webhook_url",
                fields_for("oauth_clients").unwrap(),
            ),
        ];
        for (field, allowlist) in fields {
            assert!(
                allowlist.contains(&field),
                "URL field {field} is not allowlisted"
            );
            let value = "https://export-user:URL_USERINFO@example.test/token/PATH_TOKEN?token=URL_QUERY#URL_FRAGMENT";
            let mut row = Document::new();
            if field == "redirect_uris" {
                row.insert(field, Bson::Array(vec![Bson::String(value.into())]));
            } else if let Some((parent, child)) = field.split_once('.') {
                row.insert(parent, doc! {child: value});
            } else {
                row.insert(field, value);
            }
            let json = safe_item(&row, allowlist);
            let bytes = serde_json::to_vec(&json).unwrap();
            for sentinel in ["URL_USERINFO", "URL_QUERY", "URL_FRAGMENT", "PATH_TOKEN"] {
                assert!(
                    !bytes
                        .windows(sentinel.len())
                        .any(|window| window == sentinel.as_bytes()),
                    "{field} leaked {sentinel}"
                );
            }
            let marker = if let Some((parent, child)) = field.split_once('.') {
                format!("{parent}.{child}_path_omitted")
            } else {
                format!("{field}_path_omitted")
            };
            let mut marker_value = &json;
            for part in marker.split('.') {
                marker_value = &marker_value[part];
            }
            assert_eq!(marker_value, true, "{field} omission marker missing");
        }
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
    fn profile_export_includes_the_saved_quick_access_list() {
        let row = doc! {
            "email": "person@example.test",
            "profile_config": {
                "quick_access": {"items": [
                    {"kind": "destination", "id": "machines"},
                    {"kind": "agent", "id": "6f9619ff-8b86-4011-b42d-00c04fc964ff"},
                    {"kind": "crew", "id": "nyxg-0123456789abcdef0123456789abcdef"},
                ]},
                "analytics": {"enabled": true, "revision": 3_i32},
            },
        };
        let value = safe_item(&row, PROFILE_FIELDS);
        assert_eq!(
            value["profile_config"]["quick_access"]["items"],
            serde_json::json!([
                {"kind": "destination", "id": "machines"},
                {"kind": "agent", "id": "6f9619ff-8b86-4011-b42d-00c04fc964ff"},
                {"kind": "crew", "id": "nyxg-0123456789abcdef0123456789abcdef"},
            ])
        );
        // Sibling settings outside the allowlist stay out.
        assert!(
            value["profile_config"]["analytics"]
                .get("revision")
                .is_none()
        );
        assert_eq!(
            fields_for("users").unwrap(),
            PROFILE_FIELDS,
            "the users registry export and the profile section list the same fields"
        );
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
                    total_count_exact: true,
                    truncated: false,
                    unavailable: false,
                    scope_truncated: false,
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
                build_budget_ms: BUILD_BUDGET.as_millis() as u64,
                file_payloads: "metadata_only",
            },
            truncated_sections: Vec::new(),
            unavailable_sections: Vec::new(),
            excluded: Vec::new(),
            sections,
        };
        enforce_cap_with_limit(&mut export, 500);
        assert!(export.sections["assistant"]["messages"].truncated);
        assert_eq!(export.truncated_sections, vec!["assistant.messages"]);
        assert!(serde_json::to_vec(&export).unwrap().len() <= 500);
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
                    },
                    "section mismatch for {}",
                    entry.collection
                );
                assert!(
                    !spec.fields.is_empty(),
                    "empty explicit field allowlist for {}",
                    entry.collection
                );
                assert!(
                    seen.insert((spec.section, spec.subsection)),
                    "duplicate subsection spec"
                );
                assert!(
                    spec.fields.iter().all(|field| !forbidden_key(field)),
                    "forbidden field in explicit spec for {}",
                    entry.collection
                );
            }
        }
        for view in personal_data_registry::ADDITIONAL_EXPORT_VIEWS {
            let source = personal_data_registry::find(view.collection).expect(view.collection);
            let spec = additional_export_spec(view);
            match view.selector {
                personal_data_registry::ExportViewSelector::AuditMetadata => {
                    assert_eq!(
                        source.erasure,
                        personal_data_registry::Erasure::Retained(
                            personal_data_registry::RetainedCategory::Audit
                        )
                    );
                    assert_eq!(spec.fields, &["event_type", "created_at"]);
                }
                personal_data_registry::ExportViewSelector::AuthoredGroupMessagesOutsideOwnedGroups => {
                    assert_eq!(
                        source.export,
                        personal_data_registry::Export::Section(spec.section)
                    );
                }
            }
            assert!(
                seen.insert((spec.section, spec.subsection)),
                "duplicate additional view subsection"
            );
            assert!(!spec.fields.is_empty(), "empty additional view allowlist");
            assert!(
                spec.fields.iter().all(|field| !forbidden_key(field)),
                "forbidden additional view field"
            );
        }
    }

    #[test]
    fn unknown_export_collection_returns_typed_error() {
        let result = explicit_export_spec("collection_added_after_registry");
        assert!(matches!(
            result,
            Err(ExportSpecError(collection)) if collection == "collection_added_after_registry"
        ));
    }

    #[tokio::test]
    async fn authored_group_message_export_isolates_authors_and_preserves_owner_export() {
        let db =
            crate::test_utils::connect_transaction_test_database("authored_group_export").await;
        let alice = uuid::Uuid::new_v4().to_string();
        let bob = uuid::Uuid::new_v4().to_string();
        let carol = uuid::Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_many(
                [&alice, &bob, &carol]
                    .into_iter()
                    .map(|id| crate::test_utils::test_user(id, UserType::Person)),
            )
            .await
            .unwrap();
        let message = |id: &str, owner: &str, author: &str, text: &str, millis: i64| {
            doc! {
                "_id": id, "user_id": owner, "author_user_id": author,
                "group_id": format!("group-{owner}"), "role": "user", "text": text,
                "created_at": bson::DateTime::from_millis(millis),
                "author_display_name": "PRIVATE_DISPLAY_NAME", "activities": [{"text": "PRIVATE_ACTIVITY"}],
            }
        };
        db.collection::<Document>("assistant_group_messages")
            .insert_one(message("bob-message", &bob, &bob, "BOB_MESSAGE", 1000))
            .await
            .unwrap();
        let bob_before = build(&db, &bob).await.unwrap();
        let mut notice = message("notice", &bob, &bob, "GROUP_NOTICE", 5000);
        notice.remove("author_user_id");
        notice.insert("role", "notice");
        db.collection::<Document>("assistant_group_messages")
            .insert_many([
                message("alice-older", &bob, &alice, "ALICE_OLDER", 2000),
                message("alice-newer", &bob, &alice, "ALICE_NEWER", 4000),
                message("carol-message", &bob, &carol, "CAROL_MESSAGE", 3000),
                message("alice-owned", &alice, &alice, "ALICE_OWN_GROUP", 6000),
                notice,
            ])
            .await
            .unwrap();
        let alice_export = build(&db, &alice).await.unwrap();
        let authored = &alice_export.sections["assistant"]["authored_group_messages"];
        assert_eq!(authored.total_count, 2);
        assert!(!authored.truncated);
        assert_eq!(
            authored
                .items
                .iter()
                .map(|item| item["text"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["ALICE_NEWER", "ALICE_OLDER"]
        );
        for (item, millis) in authored.items.iter().zip([4000, 2000]) {
            assert_eq!(
                item.as_object()
                    .unwrap()
                    .keys()
                    .map(String::as_str)
                    .collect::<BTreeSet<_>>(),
                BTreeSet::from(["group_id", "role", "text", "created_at"])
            );
            assert_eq!(item["group_id"], format!("group-{bob}"));
            assert_eq!(item["role"], "user");
            assert_eq!(
                chrono::DateTime::parse_from_rfc3339(item["created_at"].as_str().unwrap())
                    .unwrap()
                    .timestamp_millis(),
                millis
            );
        }
        let owned = &alice_export.sections["assistant"]["assistant_group_messages"];
        assert_eq!(owned.total_count, 1);
        assert_eq!(owned.items[0]["text"], "ALICE_OWN_GROUP");
        let bytes = serde_json::to_string(&alice_export).unwrap();
        for excluded in [
            "BOB_MESSAGE",
            "CAROL_MESSAGE",
            "GROUP_NOTICE",
            "PRIVATE_DISPLAY_NAME",
            "PRIVATE_ACTIVITY",
        ] {
            assert!(
                !bytes.contains(excluded),
                "other-member or private metadata escaped: {excluded}"
            );
        }
        let bob_after = build(&db, &bob).await.unwrap();
        assert_eq!(
            serde_json::to_value(&bob_before.sections).unwrap(),
            serde_json::to_value(&bob_after.sections).unwrap()
        );
        assert_eq!(
            bob_after.sections["assistant"]["assistant_group_messages"].items[0]["text"],
            "BOB_MESSAGE"
        );
        assert_eq!(
            bob_after.sections["assistant"]["authored_group_messages"].total_count,
            0
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn authored_group_message_view_respects_the_shared_export_budget() {
        let db =
            crate::test_utils::connect_transaction_test_database("authored_group_budget").await;
        let person = uuid::Uuid::new_v4().to_string();
        db.collection::<Document>("assistant_group_messages").insert_many((0..3).map(|index| doc! {
            "_id": format!("message-{index}"), "user_id": "other-owner", "author_user_id": &person,
            "group_id": "shared-group", "role": "user", "text": format!("{index}{}", "x".repeat(600)),
            "created_at": bson::DateTime::from_millis(index),
        })).await.unwrap();
        let view = &personal_data_registry::ADDITIONAL_EXPORT_VIEWS[0];
        let spec = additional_export_spec(view);
        let mut budget = ExportBudget {
            remaining: 2000,
            ..Default::default()
        };
        assert!(budget.admit(&serde_json::json!({"earlier_subsection": "x".repeat(1000)})));
        let value = rows(
            &db,
            view.collection,
            additional_export_filter(view, &person),
            spec.fields,
            doc! {"created_at": -1},
            &mut budget,
        )
        .await
        .unwrap();
        assert_eq!(value.total_count, 3);
        assert!(value.truncated);
        assert_eq!(value.items.len(), 1);
        assert!(value.items[0]["text"].as_str().unwrap().starts_with('2'));
        assert!(!budget.admit(&serde_json::json!({"later_subsection": "x".repeat(600)})));
        db.drop().await.unwrap();
    }

    fn put_owner(document: &mut Document, path: &str, owner: &str) {
        fn put(value: &mut Bson, segments: &[&str], owner: &str) {
            if segments.is_empty() {
                *value = Bson::String(owner.into());
            } else if let Bson::Array(values) = value {
                if values.is_empty() {
                    values.push(Bson::Document(Document::new()));
                }
                for value in values {
                    put(value, segments, owner);
                }
            } else {
                if !matches!(value, Bson::Document(_)) {
                    *value = Bson::Document(Document::new());
                }
                let doc = value.as_document_mut().unwrap();
                let child = doc.entry(segments[0].to_owned()).or_insert(Bson::Null);
                put(child, &segments[1..], owner);
            }
        }
        let mut row = Bson::Document(std::mem::take(document));
        put(&mut row, &path.split('.').collect::<Vec<_>>(), owner);
        *document = row.as_document().unwrap().clone();
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
            let mut target = super::model_tests::model_fixture(entry.collection);
            target.insert("_id", &target_id);
            for field in ["password_hash", "token_value", "ciphertext"] {
                target.insert(field, "EXPORT_SECRET_SENTINEL");
            }
            target.insert(
                "payload_encrypted",
                Bson::Binary(bson::Binary {
                    subtype: bson::spec::BinarySubtype::Generic,
                    bytes: vec![0xde, 0xad, 0xbe, 0xef],
                }),
            );
            let mut other = target.clone();
            other.insert("_id", &bystander_id);
            for field in fields {
                if *field == "_id" {
                    continue;
                }
                put_owner(&mut target, field, &owner);
                put_owner(&mut other, field, &bystander);
            }
            use personal_data_registry::ExportSelector;
            let selected_fields: &[&str] = match entry.export_selector {
                ExportSelector::Person(field) => &[field],
                ExportSelector::AnyPerson(fields) => fields,
                ExportSelector::LegacyServiceAccount => &["owner_user_id"],
                ExportSelector::AuthoredOwnedGroup => &["user_id", "author_user_id"],
                ExportSelector::None | ExportSelector::JoinedPerson { .. } => &[],
            };
            for field in selected_fields {
                put_owner(&mut target, field, &owner);
                put_owner(&mut other, field, &bystander);
            }
            db.collection::<Document>(entry.collection)
                .insert_many([target, other])
                .await
                .unwrap();
        }
        // A person's history label on someone else's resource does not confer
        // export ownership, even when the resource has exportable metadata.
        for collection in ["user_services", "oauth_states"] {
            let mut shared = doc! {
                "_id": format!("shared-attribution-{collection}"), "user_id": &bystander,
                "name": "EXPORT_SHARED_RESOURCE_SENTINEL", "created_at": bson::DateTime::now(),
            };
            for spec in personal_data_registry::ATTRIBUTIONS
                .iter()
                .filter(|spec| spec.collection == collection)
            {
                put_owner(&mut shared, spec.person_field, &owner);
            }
            db.collection::<Document>(collection)
                .insert_one(shared)
                .await
                .unwrap();
        }
        let export = build(&db, &owner).await.unwrap();
        let bytes = serde_json::to_vec(&export).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("EXPORT_SECRET_SENTINEL"));
        assert!(!String::from_utf8_lossy(&bytes).contains("bystander-"));
        assert!(!String::from_utf8_lossy(&bytes).contains("shared-attribution-"));
        assert!(!String::from_utf8_lossy(&bytes).contains("EXPORT_SHARED_RESOURCE_SENTINEL"));
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
            if matches!(
                entry.owner,
                personal_data_registry::OwnerSelector::NotPersonal
            ) {
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

#[cfg(test)]
#[path = "account_export_model_tests.rs"]
mod model_tests;

#[cfg(test)]
#[path = "account_export_query_tests.rs"]
mod query_tests;
