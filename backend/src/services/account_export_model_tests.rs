//! Serialized storage-model fixtures and explicit writer-shape exceptions.
use super::*;
use serde::de::DeserializeOwned;
use std::sync::OnceLock;

fn roundtrip<T: DeserializeOwned + Serialize>(row: &Document) -> Result<Document, String> {
    let model: T = bson::from_document(row.clone()).map_err(|error| error.to_string())?;
    bson::to_document(&model).map_err(|error| error.to_string())
}

// These coordination writers intentionally have no public storage model.
// Keep one typed fixture for each documented writer shape, independently of
// the export projection; unknown input fields do not survive serialization.
#[derive(serde::Deserialize, Serialize)]
struct PushReservation {
    at: bson::DateTime,
}

/// async_service_operation::admit_in_session uses an untyped quota document.
#[derive(serde::Deserialize, Serialize)]
struct AsyncQuota {
    user_id: String,
    generation: i64,
}

/// assistant_upload_retention::delete_root copies scoped metadata into an untyped tombstone.
#[derive(serde::Deserialize, Serialize)]
struct AttachmentExpiry {
    user_id: String,
    conversation_id: String,
    created_at: bson::DateTime,
    expired_at: bson::DateTime,
}

/// assistant_skill_authoring_claims::reserve/save_claim persist an untyped
/// owner envelope around the typed claim; DraftOrigin reads only the inner claim.
#[derive(serde::Deserialize, Serialize)]
struct DraftClaimEnvelope {
    owner_id: String,
    claim: crate::models::assistant_agent_learning::AuthoredDraftClaim,
}

/// assistant_agent_learning_review::backfill_package_format writes an untyped migration marker.
#[derive(serde::Deserialize, Serialize)]
struct LearningMigration {
    completed_at: bson::DateTime,
}

/// assistant_notification_service::claim_push writes untyped reservation timestamps.
#[derive(serde::Deserialize, Serialize)]
struct PushBudget {
    #[serde(rename = "_id")]
    id: String,
    reservations: Vec<PushReservation>,
}

/// assistant_notification_service::claim_push writes untyped source reservations.
#[derive(serde::Deserialize, Serialize)]
struct PushSource {
    user_id: String,
    sent_at: bson::DateTime,
}

/// assistant_service_execution::check_inner/attach_org_offer writes an untyped execution record.
#[derive(serde::Deserialize, Serialize)]
struct ServiceExecution {
    user_id: String,
    conversation_id: String,
}

/// encrypted_record_migration::advance writes untyped storage diagnostic metadata.
#[derive(serde::Deserialize, Serialize)]
struct MigrationFinding {
    collection: String,
    category: String,
    observed_at: bson::DateTime,
}

/// encrypted_record_migration::start writes an untyped global migration record.
#[derive(serde::Deserialize, Serialize)]
struct MigrationRun {
    phase: String,
    scanned: i64,
    rewritten: i64,
    started_at: bson::DateTime,
}

/// remote_machine_events::record_investigation writes untyped resource investigation metadata.
#[derive(serde::Deserialize, Serialize)]
struct MachineOrphan {
    owner_id: String,
    machine_id: String,
    state: String,
    reason: String,
    created_at: bson::DateTime,
    last_seen_at: bson::DateTime,
}

/// platform_setup_service::guide_select_current writes an untyped binding-scoped selection.
#[derive(serde::Deserialize, Serialize)]
struct GuideSelection {
    binding_id: String,
    generation: i64,
    setup_id: String,
    purge_after: bson::DateTime,
}

/// platform_binding_service::admit_native_reply_in_session writes untyped provider reply deduplication metadata.
#[derive(serde::Deserialize, Serialize)]
struct NativeReply {
    source_id: String,
    kind: String,
    purge_after: bson::DateTime,
}

/// platform_link_intent_service writes an untyped last-decision timestamp keyed by the person.
#[derive(serde::Deserialize, Serialize)]
struct DecisionSpacing {
    #[serde(rename = "_id")]
    id: String,
    last_decision_at: bson::DateTime,
}

/// trigger_service::create writes an untyped owner version counter.
#[derive(serde::Deserialize, Serialize)]
struct TriggerOwnerLimit {
    #[serde(rename = "_id")]
    id: String,
    version: i64,
}

/// trigger_schedule::insert_run writes untyped scheduled work metadata.
#[derive(serde::Deserialize, Serialize)]
struct TriggerWork {
    trigger_id: String,
    user_id: String,
    kind: String,
    at: bson::DateTime,
}

/// org_channel_service::transcript::lane writes untyped actor-scoped lane metadata.
#[derive(serde::Deserialize, Serialize)]
struct OrgLane {
    actor_id: String,
    conversation_id: String,
    created_at: bson::DateTime,
}

/// org_channel_service::links::issue writes untyped link challenges.
#[derive(serde::Deserialize, Serialize)]
struct OrgChallenge {
    user_id: String,
    org_id: String,
    state: String,
    created_at: bson::DateTime,
    expires_at: bson::DateTime,
}

/// org_channel_service::transcript::publish extends GroupMessage with actor and conversation selectors.
#[derive(serde::Deserialize, Serialize)]
struct OrgMessage {
    #[serde(flatten)]
    message: crate::models::assistant_group::GroupMessage,
    channel_id: String,
    conversation_id: String,
    actor_id: String,
}

fn stored_fixture(collection: &str, row: &Document) -> Result<Document, String> {
    match collection {
        "client_error_reports" => {
            roundtrip::<crate::models::client_error_report::ClientErrorReport>(row)
        }
        "agent_key_login_requests" => {
            roundtrip::<crate::models::agent_key_login_request::AgentKeyLoginRequest>(row)
        }
        "agent_service_bindings" => {
            roundtrip::<crate::models::agent_service_binding::AgentServiceBinding>(row)
        }
        "api_key_credentials" => {
            roundtrip::<crate::models::api_key_credential::ApiKeyCredential>(row)
        }
        "api_keys" => roundtrip::<crate::models::api_key::ApiKey>(row),
        "approval_grants" => roundtrip::<crate::models::approval_grant::ApprovalGrant>(row),
        "approval_requests" => roundtrip::<crate::models::approval_request::ApprovalRequest>(row),
        "assistant_acknowledgements" => {
            roundtrip::<crate::models::assistant_acknowledgement::AssistantAcknowledgement>(row)
        }
        "assistant_action_receipts" => {
            roundtrip::<crate::models::assistant_action_receipt::AssistantActionReceipt>(row)
        }
        "assistant_agent_consents" => {
            roundtrip::<crate::models::assistant_sharing::AgentConsent>(row)
        }
        "assistant_agent_learning" => {
            roundtrip::<crate::models::assistant_agent_learning::AssistantAgentLearning>(row)
        }
        "assistant_agent_learning_members" => {
            roundtrip::<crate::models::assistant_agent_learning::AssistantAgentLearningMember>(row)
        }
        "assistant_agent_learning_proposals" => roundtrip::<
            crate::models::assistant_agent_learning::AssistantAgentLearningProposal,
        >(row),
        "assistant_agent_learning_runs" => {
            roundtrip::<crate::models::assistant_agent_learning::AssistantAgentLearningRun>(row)
        }
        "assistant_agent_learning_skill_roots" => roundtrip::<
            crate::models::assistant_agent_learning::AssistantAgentLearningSkillRoot,
        >(row),
        "assistant_agent_private_notes" => {
            roundtrip::<crate::models::assistant_sharing::PrivateNotes>(row)
        }
        "assistant_agent_shares" => roundtrip::<crate::models::assistant_sharing::AgentShare>(row),
        "assistant_agent_sharing" => {
            roundtrip::<crate::models::assistant_sharing::AgentSharing>(row)
        }
        "assistant_agents" => roundtrip::<crate::models::assistant_agent::AssistantAgent>(row),
        "assistant_artifact_deliveries" => {
            roundtrip::<crate::models::generated_artifact::Delivery>(row)
        }
        "assistant_async_operations" => {
            roundtrip::<crate::models::async_service_operation::AsyncServiceOperation>(row)
        }
        "assistant_async_quotas" => roundtrip::<AsyncQuota>(row),
        "assistant_attachment_expiry" => roundtrip::<AttachmentExpiry>(row),
        "assistant_attachments" => {
            roundtrip::<crate::models::assistant_attachment::AssistantAttachment>(row)
        }
        "assistant_authored_draft_claims" => roundtrip::<DraftClaimEnvelope>(row),
        "assistant_checkpoints" => {
            roundtrip::<crate::models::assistant_checkpoint::AssistantCheckpoint>(row)
        }
        "assistant_cma_agents" => {
            roundtrip::<crate::models::assistant_cma_agent::AssistantCmaAgent>(row)
        }
        "assistant_cma_creations" => {
            roundtrip::<crate::models::assistant_cma_creation::AssistantCmaCreation>(row)
        }
        "assistant_cma_sandboxes" => {
            roundtrip::<crate::models::assistant_cma_sandbox::AssistantCmaSandbox>(row)
        }
        "assistant_cma_session_origins" => {
            roundtrip::<crate::models::cma_session_origin::CmaSessionOrigin>(row)
        }
        "assistant_conversations" => {
            roundtrip::<crate::models::assistant_conversation::AssistantConversation>(row)
        }
        "assistant_direct_handoffs" => {
            roundtrip::<crate::models::assistant_direct_handoff::DirectHandoff>(row)
        }
        "assistant_group_agent_access" => {
            roundtrip::<crate::models::assistant_group_collaboration::AgentAccess>(row)
        }
        "assistant_group_agent_proposals" => {
            roundtrip::<crate::models::assistant_group_collaboration::AgentProposal>(row)
        }
        "assistant_group_execution_slots" => {
            roundtrip::<crate::models::assistant_group_collaboration::ExecutionSlot>(row)
        }
        "assistant_group_invites" => {
            roundtrip::<crate::models::assistant_group_collaboration::Invite>(row)
        }
        "assistant_group_mention_notifications" => {
            roundtrip::<crate::models::assistant_group_collaboration::MentionNotification>(row)
        }
        "assistant_group_messages" => {
            roundtrip::<crate::models::assistant_group::GroupMessage>(row)
        }
        "assistant_group_read_states" => {
            roundtrip::<crate::models::assistant_group_collaboration::ReadState>(row)
        }
        "assistant_group_requests" => {
            roundtrip::<crate::models::assistant_group::GroupRequest>(row)
        }
        "assistant_groups" => roundtrip::<crate::models::assistant_group::AssistantGroup>(row),
        "assistant_invocation_turns" => {
            roundtrip::<crate::models::assistant_invocation::InvocationTurn>(row)
        }
        "assistant_invocations" => {
            roundtrip::<crate::models::assistant_invocation::AssistantInvocation>(row)
        }
        "assistant_learning_migrations" => roundtrip::<LearningMigration>(row),
        "assistant_learning_publication_targets" => {
            roundtrip::<crate::models::assistant_agent_learning::LearningPublicationTarget>(row)
        }
        "assistant_message_feedback" => {
            roundtrip::<crate::models::assistant_message_feedback::AssistantMessageFeedback>(row)
        }
        "assistant_messages" => {
            roundtrip::<crate::models::assistant_message::AssistantMessage>(row)
        }
        "assistant_notification_push_budgets" => roundtrip::<PushBudget>(row),
        "assistant_notification_push_sources" => roundtrip::<PushSource>(row),
        "assistant_notifications" => {
            roundtrip::<crate::models::assistant_notification::AssistantNotification>(row)
        }
        "assistant_onboarding" => {
            roundtrip::<crate::models::assistant_onboarding::AssistantOnboarding>(row)
        }
        "assistant_resources" => {
            roundtrip::<crate::models::assistant_resource::AssistantResource>(row)
        }
        "assistant_service_execution" => roundtrip::<ServiceExecution>(row),
        "assistant_settings" => {
            roundtrip::<crate::models::assistant_settings::AssistantSettings>(row)
        }
        "assistant_skill_draft_attempts" => roundtrip::<
            crate::models::assistant_skill_draft_attempt::AssistantSkillDraftAttempt,
        >(row),
        "assistant_tool_results" => {
            roundtrip::<crate::models::assistant_tool_result::AssistantToolResult>(row)
        }
        "assistant_voice_announcements" => {
            roundtrip::<crate::models::assistant_voice_announcement::Announcement>(row)
        }
        "assistant_voice_requests" => {
            roundtrip::<crate::models::assistant_voice::VoiceRequest>(row)
        }
        "assistant_voice_windows" => roundtrip::<crate::models::assistant_voice::VoiceWindow>(row),
        "assistant_wire_logs" => {
            roundtrip::<crate::models::assistant_wire_log::AssistantWireLog>(row)
        }
        "async_service_operation_artifacts" => {
            roundtrip::<crate::models::generated_artifact::PendingArtifact>(row)
        }
        "auth_device_codes" => roundtrip::<crate::models::auth_device_code::AuthDeviceCode>(row),
        "auth_device_codes_v2" => roundtrip::<crate::models::auth_device_code::AuthDeviceCode>(row),
        "authorization_codes" => {
            roundtrip::<crate::models::authorization_code::AuthorizationCode>(row)
        }
        "billing_wallet" => roundtrip::<crate::models::billing_wallet::BillingWallet>(row),
        "catalog_delegation_grants" => {
            roundtrip::<crate::models::catalog_delegation_grant::CatalogDelegationGrant>(row)
        }
        "channel_activity_notifications" => {
            roundtrip::<crate::models::channel_message::ChannelMessage>(row)
        }
        "channel_adapter_accounts" => {
            roundtrip::<crate::models::channel_adapter::ProviderAccount>(row)
        }
        "channel_adapter_allocations" => {
            roundtrip::<crate::models::channel_adapter::ResourceAllocation>(row)
        }
        "channel_adapter_bindings" => roundtrip::<crate::models::channel_adapter::Binding>(row),
        "channel_adapter_events" => roundtrip::<crate::models::channel_adapter::EventReceipt>(row),
        "channel_adapter_resource_claims" => {
            roundtrip::<crate::models::channel_adapter::ResourceClaim>(row)
        }
        "channel_adapter_sessions" => roundtrip::<crate::models::channel_adapter::CallSession>(row),
        "channel_bots" => roundtrip::<crate::models::channel_bot::ChannelBot>(row),
        "channel_connect_links" => {
            roundtrip::<crate::models::channel_connect_link::ChannelConnectLink>(row)
        }
        "channel_conversations" => {
            roundtrip::<crate::models::channel_conversation::ChannelConversation>(row)
        }
        "channel_delivery_receipts" => {
            roundtrip::<crate::models::channel_delivery::DeliveryReceipt>(row)
        }
        "channel_email_batches" => roundtrip::<crate::models::channel_email::EmailBatch>(row),
        "channel_email_receipts" => roundtrip::<crate::models::channel_email::EmailReceipt>(row),
        "channel_email_sends" => roundtrip::<crate::models::channel_email::EmailSend>(row),
        "channel_email_subscriptions" => {
            roundtrip::<crate::models::channel_email::EmailSubscription>(row)
        }
        "channel_guest_setup" => roundtrip::<crate::models::channel_guest_setup::GuestSetup>(row),
        "channel_messages" => roundtrip::<crate::models::channel_message::ChannelMessage>(row),
        "channel_reply_deliveries" => {
            roundtrip::<crate::models::channel_reply_delivery::ChannelReplyDelivery>(row)
        }
        "channel_text_deliveries" => {
            roundtrip::<crate::models::channel_text_delivery::ChannelTextDelivery>(row)
        }
        "channel_whatsapp_native" => {
            roundtrip::<crate::models::channel_whatsapp_native::NativeWhatsapp>(row)
        }
        "cli_pairings" => roundtrip::<crate::models::cli_pairing::CliPairing>(row),
        "connect_links" => roundtrip::<crate::models::connect_link::ConnectLink>(row),
        "consents" => roundtrip::<crate::models::consent::Consent>(row),
        "credit_grants" => roundtrip::<crate::models::credit_grant::CreditGrant>(row),
        "credit_schedule_recipients" => {
            roundtrip::<crate::models::credit_schedule_recipient::CreditScheduleRecipient>(row)
        }
        "device_codes" => roundtrip::<crate::models::device_code::DeviceCode>(row),
        "device_onboard_credentials" => {
            roundtrip::<crate::models::device_onboard_credential::DeviceOnboardCredential>(row)
        }
        "encrypted_record_migration_findings" => roundtrip::<MigrationFinding>(row),
        "encrypted_record_migration_runs" => roundtrip::<MigrationRun>(row),
        "execution_permission_deliveries" => {
            roundtrip::<crate::models::execution_permission_channel::Delivery>(row)
        }
        "feature_flag_overrides" => {
            roundtrip::<crate::models::feature_flag_override::FeatureFlagOverride>(row)
        }
        "image_publications" => roundtrip::<crate::models::image_publication::ImagePublication>(row),
        "invite_codes" => roundtrip::<crate::models::invite_code::InviteCode>(row),
        "invocation_diagnostics" => {
            roundtrip::<crate::models::invocation_diagnostic::InvocationDiagnostic>(row)
        }
        "lark_base_write_photos" => {
            roundtrip::<crate::models::lark_base_write_photo::LarkBaseWritePhoto>(row)
        }
        "lark_base_writes" => roundtrip::<crate::models::lark_base_write::LarkBaseWrite>(row),
        "login_approvals" => roundtrip::<crate::models::login_approval::LoginApproval>(row),
        "login_codes" => roundtrip::<crate::models::login_code::LoginCode>(row),
        "machine_confirmations" => {
            roundtrip::<crate::models::machine_external_execution::ExternalConfirmation>(row)
        }
        "machine_free_slots" => roundtrip::<crate::models::machine_free_slot::MachineFreeSlot>(row),
        "machine_jobs" => roundtrip::<crate::models::machine_job::MachineJob>(row),
        "machine_orphan_inventory" => roundtrip::<MachineOrphan>(row),
        "mcp_session_notifications" => {
            roundtrip::<crate::models::mcp_notification::McpSessionNotification>(row)
        }
        "mcp_session_slots" => roundtrip::<crate::models::mcp_session::McpSessionSlot>(row),
        "mfa_challenges" => roundtrip::<crate::models::session::Session>(row),
        "mfa_factors" => roundtrip::<crate::models::mfa_factor::MfaFactor>(row),
        "node_registration_tokens" => {
            roundtrip::<crate::models::node_registration_token::NodeRegistrationToken>(row)
        }
        "nodes" => roundtrip::<crate::models::node::Node>(row),
        "notification_channels" => {
            roundtrip::<crate::models::notification_channel::NotificationChannel>(row)
        }
        "nyxbot_channels" => roundtrip::<crate::models::nyxbot_channel::NyxbotChannel>(row),
        "nyxbot_event_bindings" => {
            roundtrip::<crate::models::nyxbot_event_binding::NyxbotEventBinding>(row)
        }
        "nyxbot_events" => roundtrip::<crate::models::nyxbot_channel::NyxbotEvent>(row),
        "nyxbot_gateway_responses" => {
            roundtrip::<crate::models::nyxbot_gateway_response::GatewayResponse>(row)
        }
        "nyxbot_image_deliveries" => {
            roundtrip::<crate::models::nyxbot_image_delivery::NyxbotImageDelivery>(row)
        }
        "nyxbot_image_deliveries_v2" => {
            roundtrip::<crate::models::nyxbot_image_delivery::NyxbotImageDelivery>(row)
        }
        "nyxbot_threads" => roundtrip::<crate::models::nyxbot_channel::NyxbotThread>(row),
        "nyxbot_watches" => roundtrip::<crate::models::nyxbot_channel::NyxbotWatch>(row),
        "oauth_broker_bindings" => {
            roundtrip::<crate::models::oauth_broker_binding::OauthBrokerBinding>(row)
        }
        "oauth_clients" => roundtrip::<crate::models::oauth_client::OauthClient>(row),
        "oauth_consent_requests" => {
            roundtrip::<crate::models::oauth_consent_request::OauthConsentRequest>(row)
        }
        "oauth_delegation_authorities" => {
            roundtrip::<crate::models::oauth_delegation_authority::OauthDelegationAuthority>(row)
        }
        "oauth_states" => roundtrip::<crate::models::oauth_state::OAuthState>(row),
        "openclaw_channel_mappings" => {
            roundtrip::<crate::services::openclaw_channel_service::OpenClawChannelMapping>(row)
        }
        "oracle_pools" => roundtrip::<crate::models::oracle_pool::OraclePool>(row),
        "org_invites" => roundtrip::<crate::models::org_invite::OrgInvite>(row),
        "org_memberships" => roundtrip::<crate::models::org_membership::OrgMembership>(row),
        "permission_key_policies" => {
            roundtrip::<crate::models::permission_policy::PermissionPolicy>(row)
        }
        "platform_bot_cleanup" => {
            roundtrip::<crate::models::platform_bot_config::PlatformBotCleanup>(row)
        }
        "platform_bot_configs" => {
            roundtrip::<crate::models::platform_bot_config::PlatformBotConfig>(row)
        }
        "platform_bot_installations" => {
            roundtrip::<crate::models::platform_installation::PlatformInstallation>(row)
        }
        "platform_delivery_tests" => {
            roundtrip::<crate::models::platform_installation::DeliveryTest>(row)
        }
        "platform_discord_control_installation_history" => roundtrip::<
            crate::models::platform_entry_discord_installation::DiscordControlInstallation,
        >(row),
        "platform_discord_control_installations" => roundtrip::<
            crate::models::platform_entry_discord_installation::DiscordControlInstallation,
        >(row),
        "platform_guide_selections" => roundtrip::<GuideSelection>(row),
        "platform_native_replies" => roundtrip::<NativeReply>(row),
        "platform_onboarding_binding_requests" => {
            roundtrip::<crate::models::platform_binding::PlatformBindingRequest>(row)
        }
        "platform_onboarding_bindings" => {
            roundtrip::<crate::models::platform_binding::PlatformBinding>(row)
        }
        "platform_personal_decision_spacing" => roundtrip::<DecisionSpacing>(row),
        "platform_personal_setup_selections" => {
            roundtrip::<crate::models::platform_manager_management::PersonalSetupSelection>(row)
        }
        "platform_personal_stop_effects" => roundtrip::<
            crate::models::platform_bot_runtime_controls::PlatformPersonalStopEffect,
        >(row),
        "platform_public_deployment_activations" => {
            roundtrip::<crate::models::platform_public_contract::PublicDeploymentActivation>(row)
        }
        "platform_public_deployment_proofs" => {
            roundtrip::<crate::models::platform_public_contract::PublicDeploymentRecord>(row)
        }
        "platform_public_probe_consents" => {
            roundtrip::<crate::models::platform_public_contract::ProbeConsent>(row)
        }
        "platform_public_readiness_probes" => {
            roundtrip::<crate::models::platform_public_contract::PublicProbe>(row)
        }
        "platform_setup_notices" => {
            roundtrip::<crate::models::platform_installation::SetupNotice>(row)
        }
        "platform_setup_steps" => {
            roundtrip::<crate::models::platform_installation::SetupStepJob>(row)
        }
        "platform_setups" => roundtrip::<crate::models::platform_setup::PlatformSetup>(row),
        "refresh_tokens" => roundtrip::<crate::models::refresh_token::RefreshToken>(row),
        "saved_logins" => roundtrip::<crate::models::saved_login::SavedLogin>(row),
        "service_account_key_read_grants" => roundtrip::<
            crate::models::service_account_key_read_grant::ServiceAccountKeyReadGrant,
        >(row),
        "service_accounts" => roundtrip::<crate::models::service_account::ServiceAccount>(row),
        "service_approval_configs" => {
            roundtrip::<crate::models::service_approval_config::ServiceApprovalConfig>(row)
        }
        "service_change_events" => {
            roundtrip::<crate::models::service_change_event::ServiceChangeEvent>(row)
        }
        "service_pool_member_health" => {
            roundtrip::<crate::models::service_pool_member_health::ServicePoolMemberHealth>(row)
        }
        "service_pools" => roundtrip::<crate::models::service_pool::ServicePool>(row),
        "service_preferences" => {
            roundtrip::<crate::models::service_preference::ServicePreference>(row)
        }
        "sessions" => roundtrip::<crate::models::session::Session>(row),
        "telegram_bot_claims" => {
            roundtrip::<crate::models::telegram_bot_claim::TelegramBotClaim>(row)
        }
        "telegram_bot_requests" => {
            roundtrip::<crate::models::telegram_bot_request::TelegramBotRequest>(row)
        }
        "telegram_managed_bot_events" => {
            roundtrip::<crate::models::telegram_bot_request::ManagedBotEvents>(row)
        }
        "trigger_deliveries" => {
            roundtrip::<crate::models::trigger_delivery::TriggerDeliveryRecord>(row)
        }
        "trigger_owner_limits" => roundtrip::<TriggerOwnerLimit>(row),
        "trigger_runs" => roundtrip::<crate::models::trigger_run::TriggerRun>(row),
        "trigger_work" => roundtrip::<TriggerWork>(row),
        "triggers" => roundtrip::<crate::models::trigger::Trigger>(row),
        "usage_meter" => roundtrip::<crate::models::usage_meter::UsageMeterRow>(row),
        "usage_workspaces" => roundtrip::<crate::models::usage_workspace::UsageWorkspace>(row),
        "user_api_keys" => roundtrip::<crate::models::user_api_key::UserApiKey>(row),
        "user_endpoints" => roundtrip::<crate::models::user_endpoint::UserEndpoint>(row),
        "user_provider_credentials" => {
            roundtrip::<crate::models::user_provider_credentials::UserProviderCredentials>(row)
        }
        "user_provider_tokens" => {
            roundtrip::<crate::models::user_provider_token::UserProviderToken>(row)
        }
        "user_service_connections" => {
            roundtrip::<crate::models::user_service_connection::UserServiceConnection>(row)
        }
        "user_services" => roundtrip::<crate::models::user_service::UserService>(row),
        "assistant_voice_sessions" => {
            roundtrip::<crate::models::assistant_voice_session::VoiceSession>(row)
        }
        "generated_artifact_content_series" => {
            roundtrip::<crate::models::generated_artifact::ContentSeries>(row)
        }
        "oracle_login_snapshots" => {
            roundtrip::<crate::models::oracle_login_snapshot::OracleLoginSnapshot>(row)
        }
        "oracle_sessions" => roundtrip::<crate::models::oracle_session::OracleSession>(row),
        "oracle_tasks" => roundtrip::<crate::models::oracle_task::OracleTask>(row),
        "oracle_worker_commands" => {
            roundtrip::<crate::models::oracle_worker_command::OracleWorkerCommand>(row)
        }
        "oracle_workers" => roundtrip::<crate::models::oracle_worker::OracleWorker>(row),
        "org_channel_lanes" => roundtrip::<OrgLane>(row),
        "org_channel_link_challenges" => roundtrip::<OrgChallenge>(row),
        "org_channel_messages" => roundtrip::<OrgMessage>(row),
        "org_channel_sender_links" => roundtrip::<crate::models::org_channel::SenderLink>(row),
        "platform_personal_bindings" => {
            roundtrip::<crate::models::platform_personal_binding::PlatformPersonalBinding>(row)
        }
        "platform_personal_link_challenges" => {
            roundtrip::<crate::models::platform_link_intent::PlatformLinkChallenge>(row)
        }
        "platform_personal_link_intents" => {
            roundtrip::<crate::models::platform_link_intent::PlatformLinkIntent>(row)
        }
        "users" => roundtrip::<crate::models::user::User>(row),
        "audit_log" => roundtrip::<crate::models::audit_log::AuditLog>(row),
        _ => Err(format!("no storage-model fixture for {collection}")),
    }
}

fn inputs() -> &'static BTreeMap<String, Vec<Document>> {
    static INPUTS: OnceLock<BTreeMap<String, Vec<Document>>> = OnceLock::new();
    INPUTS.get_or_init(|| {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("test_data/account-export-models.json");
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    })
}

pub(super) fn model_fixture(collection: &str) -> Document {
    stored_fixture(collection, &inputs()[collection][0])
        .unwrap_or_else(|error| panic!("{collection}: {error}"))
}

#[test]
fn every_export_path_exists_in_serialized_storage_models() {
    let mut problems = Vec::new();
    let mut specs = personal_data_registry::entries()
        .iter()
        .filter(|entry| matches!(entry.export, personal_data_registry::Export::Section(_)))
        .map(|entry| (entry.collection, fields_for(entry.collection).unwrap()))
        .collect::<Vec<_>>();
    for view in personal_data_registry::ADDITIONAL_EXPORT_VIEWS {
        specs.push((view.collection, additional_export_spec(view).fields));
    }
    // Access predicates must refer to persisted fields too; an invented owner
    // key silently yields an empty section even with a correct projection.
    for entry in personal_data_registry::entries() {
        use personal_data_registry::ExportSelector as S;
        let fields: Vec<&str> = match entry.export_selector {
            S::Person(field) => vec![field],
            S::AnyPerson(fields) => fields.to_vec(),
            S::LegacyServiceAccount => vec!["owner_user_id", "created_by"],
            S::AuthoredOwnedGroup => vec!["user_id", "author_user_id"],
            S::JoinedPerson { field, .. } => vec![field],
            S::None => Vec::new(),
        };
        if !fields.is_empty() {
            specs.push((entry.collection, Box::leak(fields.into_boxed_slice())));
        }
    }
    for (collection, fields) in specs {
        let Some(rows) = inputs().get(collection) else {
            problems.push(format!("{collection}: missing independent model fixture"));
            continue;
        };
        let serialized = rows
            .iter()
            .filter_map(|row| match stored_fixture(collection, row) {
                Ok(row) => Some(row),
                Err(error) => {
                    problems.push(format!("{collection}: {error}"));
                    None
                }
            })
            .collect::<Vec<_>>();
        for field in fields {
            if !serialized.iter().any(|row| get_path(row, field).is_some()) {
                problems.push(format!(
                    "{collection}.{field}: absent from serialized model"
                ));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[tokio::test]
async fn recipient_schedule_export_is_person_scoped_and_anchors_are_financially_retained() {
    use crate::models::credit_schedule_recipient::{
        COLLECTION_NAME, CreditScheduleRecipient, CreditScheduleRecipientScan, SCAN_COLLECTION_NAME,
    };
    use personal_data_registry::{Erasure, ExclusionReason, Export, RetainedCategory};

    let db = crate::test_utils::connect_transaction_test_database("export_recipient_cycles").await;
    let owner = uuid::Uuid::new_v4().to_string();
    db.collection::<crate::models::user::User>("users")
        .insert_one(crate::test_utils::test_user(
            &owner,
            crate::models::user::UserType::Person,
        ))
        .await
        .unwrap();
    let mut own: CreditScheduleRecipient =
        bson::from_document(model_fixture(COLLECTION_NAME)).unwrap();
    own.recipient_user_id.clone_from(&owner);
    own.schedule_id = "own-cycle".into();
    let mut other = own.clone();
    other.id = "other-cycle".into();
    other.recipient_user_id = "other-person".into();
    other.schedule_id = "other-schedule".into();
    db.collection::<CreditScheduleRecipient>(COLLECTION_NAME)
        .insert_many([own.clone(), other])
        .await
        .unwrap();
    db.collection::<CreditScheduleRecipientScan>(SCAN_COLLECTION_NAME)
        .insert_one(CreditScheduleRecipientScan {
            id: "own-cycle".into(),
            cursor_user_id: Some(owner.clone()),
        })
        .await
        .unwrap();

    let export = build(&db, &owner).await.unwrap();
    let cycles = &export.sections["billing"][COLLECTION_NAME];
    assert_eq!(cycles.total_count, 1);
    assert_eq!(cycles.items.len(), 1);
    assert_eq!(cycles.items[0]["schedule_id"], "own-cycle");
    assert_eq!(cycles.items[0]["recipient_user_id"], owner);
    assert_eq!(cycles.items[0]["anchor_at"], own.anchor_at.to_rfc3339());
    assert!(cycles.items[0].get("eligibility_version").is_none());
    assert!(
        export
            .sections
            .values()
            .all(|section| !section.contains_key(SCAN_COLLECTION_NAME))
    );
    assert_eq!(
        personal_data_registry::find(COLLECTION_NAME)
            .unwrap()
            .erasure,
        Erasure::Retained(RetainedCategory::FinancialBilling)
    );
    assert_eq!(
        personal_data_registry::find(SCAN_COLLECTION_NAME)
            .unwrap()
            .export,
        Export::Excluded(ExclusionReason::TransientCoordination)
    );
    // Removing the anchor would change future deterministic grant identities.
    assert!(
        crate::services::personal_erasure_service::local::inventory(&owner)
            .iter()
            .all(|(collection, _)| *collection != COLLECTION_NAME)
    );
}

#[tokio::test]
async fn model_rows_export_real_message_feedback_session_service_and_channel_fields() {
    use crate::models::{
        assistant_message::AssistantMessage, assistant_message_feedback::AssistantMessageFeedback,
        channel_message::ChannelMessage, session::Session, user_service::UserService,
    };
    let db = crate::test_utils::connect_transaction_test_database("export_storage_models").await;
    let person = uuid::Uuid::new_v4().to_string();
    db.collection::<crate::models::user::User>("users")
        .insert_one(crate::test_utils::test_user(
            &person,
            crate::models::user::UserType::Person,
        ))
        .await
        .unwrap();

    let mut message: AssistantMessage =
        bson::from_document(model_fixture("assistant_messages")).unwrap();
    message.user_id = person.clone();
    message.role = "user".into();
    message.text = "My authored message".into();
    db.collection::<AssistantMessage>("assistant_messages")
        .insert_one(message)
        .await
        .unwrap();
    let mut feedback: AssistantMessageFeedback =
        bson::from_document(model_fixture("assistant_message_feedback")).unwrap();
    feedback.user_id = person.clone();
    feedback.vote = "up".into();
    feedback.comment = Some("My feedback".into());
    db.collection::<AssistantMessageFeedback>("assistant_message_feedback")
        .insert_one(feedback)
        .await
        .unwrap();
    let mut session: Session = bson::from_document(model_fixture("sessions")).unwrap();
    session.user_id = person.clone();
    session.token_hash = "MODEL_SECRET_SENTINEL".into();
    db.collection::<Session>("sessions")
        .insert_one(session)
        .await
        .unwrap();
    let mut service: UserService = bson::from_document(model_fixture("user_services")).unwrap();
    service.user_id = person.clone();
    service.slug = "my-service".into();
    service.catalog_service_id = Some("catalog-service".into());
    db.collection::<UserService>("user_services")
        .insert_one(service)
        .await
        .unwrap();
    let mut channel: ChannelMessage =
        bson::from_document(model_fixture("channel_messages")).unwrap();
    channel.user_id = person.clone();
    channel.callback_status = Some("delivered".into());
    channel.thread_id = Some("interaction:MODEL_SECRET_SENTINEL".into());
    db.collection::<ChannelMessage>("channel_messages")
        .insert_one(channel)
        .await
        .unwrap();

    let export = build(&db, &person).await.unwrap();
    for collection in [
        "assistant_messages",
        "assistant_message_feedback",
        "sessions",
        "user_services",
        "channel_messages",
    ] {
        let spec = explicit_export_spec(collection).unwrap();
        let subsection = &export.sections[spec.section][spec.subsection];
        assert_eq!(subsection.total_count, 1, "{collection}");
        assert_eq!(subsection.items.len(), 1, "{collection}");
        for field in spec.fields {
            let path = format!("/{}", field.replace('.', "/"));
            let value = subsection.items[0]
                .pointer(&path)
                .unwrap_or_else(|| panic!("{collection}.{field}"));
            assert!(!value.is_null(), "{collection}.{field}");
            assert!(
                !value.as_str().is_some_and(str::is_empty),
                "{collection}.{field}"
            );
        }
    }
    assert_eq!(
        export.sections["assistant"]["assistant_messages"].items[0]["text"],
        "My authored message"
    );
    assert_eq!(
        export.sections["assistant"]["assistant_message_feedback"].items[0]["vote"],
        "up"
    );
    assert!(
        !serde_json::to_string(&export)
            .unwrap()
            .contains("MODEL_SECRET_SENTINEL")
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn export_selectors_isolate_three_people_and_shared_relationships() {
    use crate::models::{
        assistant_sharing::{AgentConsent, AgentShare, ShareSubject},
        invite_code::{InviteCode, InviteCodeUsage},
        oauth_client::OauthClient,
        org_invite::OrgInvite,
        org_membership::OrgMembership,
        service_account::ServiceAccount,
        user::UserType,
    };
    let db =
        crate::test_utils::connect_transaction_test_database("export_principal_selectors").await;
    let people = (0..3)
        .map(|_| uuid::Uuid::new_v4().to_string())
        .collect::<Vec<_>>();
    let org = uuid::Uuid::new_v4().to_string();
    let users = people
        .iter()
        .map(|id| crate::test_utils::test_user(id, UserType::Person))
        .chain(std::iter::once(crate::test_utils::test_user(
            &org,
            UserType::Org,
        )))
        .collect::<Vec<_>>();
    db.collection::<crate::models::user::User>("users")
        .insert_many(users)
        .await
        .unwrap();
    for (index, person) in people.iter().enumerate() {
        if index < 2 {
            let mut membership: OrgMembership =
                bson::from_document(model_fixture("org_memberships")).unwrap();
            membership.id = uuid::Uuid::new_v4().to_string();
            membership.member_user_id = person.clone();
            membership.org_user_id = org.clone();
            db.collection::<OrgMembership>("org_memberships")
                .insert_one(membership)
                .await
                .unwrap();
        }
        let mut client: OauthClient = bson::from_document(model_fixture("oauth_clients")).unwrap();
        client.id = uuid::Uuid::new_v4().to_string();
        client.created_by = Some(person.clone());
        client.client_name = format!("own-client-{index}");
        db.collection::<OauthClient>("oauth_clients")
            .insert_one(client)
            .await
            .unwrap();
        let mut consent: AgentConsent =
            bson::from_document(model_fixture("assistant_agent_consents")).unwrap();
        consent.id = uuid::Uuid::new_v4().to_string();
        consent.actor_user_id = person.clone();
        consent.owner_id = people[0].clone();
        consent.conversation_id = format!("private-conversation-{index}");
        db.collection::<AgentConsent>("assistant_agent_consents")
            .insert_one(consent)
            .await
            .unwrap();
        if index > 0 {
            let mut share: AgentShare =
                bson::from_document(model_fixture("assistant_agent_shares")).unwrap();
            share.id = uuid::Uuid::new_v4().to_string();
            share.owner_id = people[0].clone();
            share.subject_kind = ShareSubject::Person;
            share.subject_id = person.clone();
            db.collection::<AgentShare>("assistant_agent_shares")
                .insert_one(share)
                .await
                .unwrap();
        }
    }
    let mut code: InviteCode = bson::from_document(model_fixture("invite_codes")).unwrap();
    code.created_by = people[0].clone();
    code.usages = people
        .iter()
        .enumerate()
        .map(|(index, user_id)| InviteCodeUsage {
            user_id: user_id.clone(),
            used_at: Utc::now(),
            email: Some(format!("person-{index}@example.test")),
        })
        .collect();
    code.used_count = 3;
    db.collection::<InviteCode>("invite_codes")
        .insert_one(code)
        .await
        .unwrap();
    for (owner, name) in [
        (Some(org.clone()), "org-created-account"),
        (Some(people[1].clone()), "transferred-account"),
        (None, "legacy-own-account"),
    ] {
        let mut account: ServiceAccount =
            bson::from_document(model_fixture("service_accounts")).unwrap();
        account.id = uuid::Uuid::new_v4().to_string();
        account.owner_user_id = owner;
        account.created_by = people[0].clone();
        account.name = name.into();
        db.collection::<ServiceAccount>("service_accounts")
            .insert_one(account)
            .await
            .unwrap();
    }
    let mut invite: OrgInvite = bson::from_document(model_fixture("org_invites")).unwrap();
    invite.org_user_id = org.clone();
    invite.created_by = people[0].clone();
    invite.redeemed_by = Some(people[1].clone());
    invite.redeemed_at = Some(Utc::now());
    db.collection::<OrgInvite>("org_invites")
        .insert_one(invite)
        .await
        .unwrap();
    for (index, person) in people.iter().enumerate() {
        let export = build(&db, person).await.unwrap();
        let encoded = serde_json::to_string(&export).unwrap();
        for other in people.iter().filter(|other| *other != person) {
            assert!(
                !encoded.contains(other),
                "person {index} sees another person's identifier"
            );
        }
        for other in (0..3).filter(|other| *other != index) {
            assert!(!encoded.contains(&format!("private-conversation-{other}")));
            assert!(!encoded.contains(&format!("own-client-{other}")));
        }
        let usages =
            &export.sections["account_activity"]["invite_codes"].items[0]["usages"]["user_id"];
        assert_eq!(usages, &serde_json::json!([person]));
        let invite = &export.sections["account_activity"]["invite_codes"].items[0];
        assert!(invite.get("used_count").is_none());
        assert!(invite.get("max_uses").is_none());
        assert_eq!(
            export.sections["assistant"]["assistant_agent_consents"].total_count,
            1
        );
        assert!(!encoded.contains("org-created-account"));
        assert_eq!(encoded.contains("legacy-own-account"), index == 0);
        assert_eq!(encoded.contains("transferred-account"), index == 1);
        assert_eq!(
            export.sections["account"]["org_invites"].total_count,
            u64::from(index == 1)
        );
    }
    db.drop().await.unwrap();
}

#[test]
fn every_primary_export_has_an_explicit_access_selector() {
    use personal_data_registry::{Export, ExportSelector};
    for entry in personal_data_registry::entries() {
        assert_eq!(
            matches!(entry.export, Export::Section(_)),
            entry.export_selector != ExportSelector::None,
            "{}",
            entry.collection
        );
    }
}

#[tokio::test]
async fn large_message_source_does_not_starve_later_session_metadata() {
    use crate::models::{assistant_message::AssistantMessage, session::Session, user::UserType};
    let db = crate::test_utils::connect_transaction_test_database("export_fair_sources").await;
    let person = uuid::Uuid::new_v4().to_string();
    db.collection::<crate::models::user::User>("users")
        .insert_one(crate::test_utils::test_user(&person, UserType::Person))
        .await
        .unwrap();
    let mut message: AssistantMessage =
        bson::from_document(model_fixture("assistant_messages")).unwrap();
    message.user_id = person.clone();
    message.role = "user".into();
    message.text = "x".repeat(512 * 1024);
    db.collection::<AssistantMessage>("assistant_messages")
        .insert_one(message)
        .await
        .unwrap();
    let mut session: Session = bson::from_document(model_fixture("sessions")).unwrap();
    session.user_id = person.clone();
    db.collection::<Session>("sessions")
        .insert_one(session)
        .await
        .unwrap();
    let export = build(&db, &person).await.unwrap();
    let message_spec = explicit_export_spec("assistant_messages").unwrap();
    let session_spec = explicit_export_spec("sessions").unwrap();
    assert!(!export.sections[message_spec.section][message_spec.subsection].truncated);
    assert_eq!(
        export.sections[message_spec.section][message_spec.subsection]
            .items
            .len(),
        1
    );
    assert!(!export.truncated_sections.contains(&format!(
        "{}.{}",
        message_spec.section, message_spec.subsection
    )));
    assert_eq!(
        export.sections[session_spec.section][session_spec.subsection]
            .items
            .len(),
        1
    );
    assert!(serde_json::to_vec(&export).unwrap().len() <= MAX_BYTES);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn row_cap_marks_the_source_and_is_listed_without_final_byte_trimming() {
    use crate::models::assistant_message_feedback::AssistantMessageFeedback;
    let db = crate::test_utils::connect_transaction_test_database("export_row_cap").await;
    let person = uuid::Uuid::new_v4().to_string();
    let mut feedback: AssistantMessageFeedback =
        bson::from_document(model_fixture("assistant_message_feedback")).unwrap();
    feedback.user_id = person.clone();
    for start in (0..SUBSECTION_ROW_CAP + 1).step_by(256) {
        db.collection::<AssistantMessageFeedback>("assistant_message_feedback")
            .insert_many(
                (start..(start + 256).min(SUBSECTION_ROW_CAP + 1))
                    .map(|index| {
                        let mut row = feedback.clone();
                        row.id = format!("feedback-{index}");
                        row
                    })
                    .collect::<Vec<_>>(),
            )
            .await
            .unwrap();
    }
    let mut budget = ExportBudget {
        remaining: MAX_BYTES,
        ..Default::default()
    };
    let value = rows(
        &db,
        "assistant_message_feedback",
        doc! {"user_id": &person},
        fields_for("assistant_message_feedback").unwrap(),
        doc! {"created_at": -1},
        &mut budget,
    )
    .await
    .unwrap();
    assert_eq!(value.total_count, SUBSECTION_ROW_CAP as u64 + 1);
    assert_eq!(value.items.len(), SUBSECTION_ROW_CAP as usize);
    assert!(value.truncated);
    let mut export = AccountExport {
        format: FORMAT,
        version: VERSION,
        generated_at: Utc::now().to_rfc3339(),
        account_id: person,
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
        sections: BTreeMap::from([(
            "assistant".into(),
            BTreeMap::from([("assistant_message_feedback".into(), value)]),
        )]),
    };
    enforce_cap_with_limit(&mut export, MAX_BYTES);
    assert_eq!(
        export.truncated_sections,
        ["assistant.assistant_message_feedback"]
    );
    assert_eq!(
        export.sections["assistant"]["assistant_message_feedback"]
            .items
            .len(),
        SUBSECTION_ROW_CAP as usize
    );
    db.drop().await.unwrap();
}

#[test]
fn export_documentation_matches_every_declared_view_and_field() {
    let mut rows = std::collections::BTreeSet::new();
    for entry in personal_data_registry::entries() {
        if !matches!(entry.export, personal_data_registry::Export::Section(_)) {
            continue;
        }
        let spec = explicit_export_spec(entry.collection).unwrap();
        rows.insert((spec.section, spec.subsection, spec.fields));
    }
    for view in personal_data_registry::ADDITIONAL_EXPORT_VIEWS {
        let spec = additional_export_spec(view);
        rows.insert((spec.section, spec.subsection, spec.fields));
    }
    let mut table =
        String::from("| Section | Subsection | Allowlisted fields |\n| --- | --- | --- |\n");
    for (section, subsection, fields) in rows {
        table.push_str(&format!(
            "| `{section}` | `{subsection}` | {} |\n",
            fields
                .iter()
                .map(|field| format!("`{field}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/ACCOUNT_DATA_EXPORT.md");
    let mut docs = std::fs::read_to_string(&path).unwrap();
    let start =
        docs.find("<!-- export-fields:start -->").unwrap() + "<!-- export-fields:start -->".len();
    let end = docs.find("<!-- export-fields:end -->").unwrap();
    if std::env::var_os("UPDATE_ACCOUNT_EXPORT_DOC").is_some() {
        docs.replace_range(start..end, &format!("\n{table}"));
        std::fs::write(&path, &docs).unwrap();
    }
    let end = docs.find("<!-- export-fields:end -->").unwrap();
    assert_eq!(
        docs[start..end].trim(),
        table.trim(),
        "regenerate the export field table with UPDATE_ACCOUNT_EXPORT_DOC=1"
    );
}

#[tokio::test]
async fn shared_guest_content_and_replies_are_excluded_but_own_inputs_and_private_history_remain() {
    use crate::models::{
        assistant_conversation::{AssistantConversation, TurnOrigin},
        assistant_message::{AssistantMessage, MessageAuthor},
    };
    let db = crate::test_utils::connect_transaction_test_database("export_authorship").await;
    let person = uuid::Uuid::new_v4().to_string();
    db.collection::<crate::models::user::User>("users")
        .insert_one(crate::test_utils::test_user(
            &person,
            crate::models::user::UserType::Person,
        ))
        .await
        .unwrap();
    let mut private: AssistantConversation =
        bson::from_document(model_fixture("assistant_conversations")).unwrap();
    private.id = "private".into();
    private.user_id = person.clone();
    private.collaboration = None;
    private.platform_thread = None;
    private.shared_agent_thread = false;
    private.channel = None;
    private.reply_channel = None;
    private.deliver_also.clear();
    private.group_id = None;
    private.guest_turn = false;
    private.invocation_controlled = false;
    private.title = "Private title".into();
    let mut shared = private.clone();
    shared.id = "shared".into();
    shared.shared_agent_thread = true;
    shared.title = "GUEST_TITLE_SENTINEL".into();
    db.collection::<AssistantConversation>("assistant_conversations")
        .insert_many([private, shared])
        .await
        .unwrap();
    let cases = [
        (
            "shared",
            "user",
            Some(MessageAuthor::Guest),
            "GUEST_INPUT_SENTINEL",
            false,
        ),
        ("shared", "assistant", None, "GUEST_REPLY_SENTINEL", false),
        ("shared", "user", None, "AMBIGUOUS_INPUT_SENTINEL", false),
        (
            "shared",
            "user",
            Some(MessageAuthor::Owner),
            "MY_SHARED_INPUT",
            true,
        ),
        ("private", "user", None, "MY_LEGACY_INPUT", true),
        ("private", "assistant", None, "MY_PRIVATE_REPLY", true),
        ("private", "assistant", None, "CHANNEL_ORIGIN_REPLY", false),
    ];
    for (index, (conversation, role, author, text, _)) in cases.iter().enumerate() {
        let mut message: AssistantMessage =
            bson::from_document(model_fixture("assistant_messages")).unwrap();
        message.id = format!("message-{index}");
        message.conversation_id = (*conversation).into();
        message.user_id = person.clone();
        message.role = (*role).into();
        message.author = *author;
        message.text = (*text).into();
        message.origin = Some(if *text == "CHANNEL_ORIGIN_REPLY" {
            TurnOrigin::Channel
        } else {
            TurnOrigin::User
        });
        message.platform_origin = None;
        message.platform_reply = None;
        db.collection::<AssistantMessage>("assistant_messages")
            .insert_one(message)
            .await
            .unwrap();
    }
    let export = build(&db, &person).await.unwrap();
    let encoded = serde_json::to_string(&export).unwrap();
    for (_, _, _, text, included) in cases {
        assert_eq!(encoded.contains(text), included, "{text}");
    }
    // Current delivery preferences cannot retroactively reclassify stored replies.
    db.collection::<Document>("assistant_conversations")
        .update_one(
            doc! {"_id":"private"},
            doc! {"$set":{"reply_channel":{"platform":"telegram","conversation_id":"destination"}}},
        )
        .await
        .unwrap();
    let changed = build(&db, &person).await.unwrap();
    assert_eq!(
        export.sections["assistant"]["assistant_messages"].items,
        changed.sections["assistant"]["assistant_messages"].items
    );
    assert!(!encoded.contains("GUEST_TITLE_SENTINEL"));
    assert!(encoded.contains("Private title"));
    assert_eq!(
        export.sections["assistant"]["assistant_messages"].total_count,
        3
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn personal_group_owner_inputs_from_the_real_writer_are_exported() {
    use crate::models::assistant_group::{AssistantGroup, GroupMessage};
    let db = crate::test_utils::connect_transaction_test_database("export_group_writer").await;
    let person = uuid::Uuid::new_v4().to_string();
    db.collection::<crate::models::user::User>("users")
        .insert_one(crate::test_utils::test_user(
            &person,
            crate::models::user::UserType::Person,
        ))
        .await
        .unwrap();
    let mut group: AssistantGroup = bson::from_document(model_fixture("assistant_groups")).unwrap();
    group.id = format!("nyxg-{}", uuid::Uuid::new_v4().simple());
    group.user_id = person.clone();
    group.collaboration = None;
    db.collection::<AssistantGroup>("assistant_groups")
        .insert_one(&group)
        .await
        .unwrap();
    let message = crate::services::assistant_group_service::append_with_recipients(
        &db,
        &person,
        &group.id,
        "user",
        None,
        "MY_PERSONAL_GROUP_INPUT",
        &[],
        &[],
    )
    .await
    .unwrap();
    assert!(message.author_user_id.is_none());
    assert!(message.collaboration.is_none());
    assert!(!message.org_group);
    let mut other = message.clone();
    other.id = uuid::Uuid::new_v4().to_string();
    other.org_group = true;
    other.text = "OTHER_ORG_MEMBER_INPUT".into();
    db.collection::<GroupMessage>("assistant_group_messages")
        .insert_one(other)
        .await
        .unwrap();
    let export = build(&db, &person).await.unwrap();
    let text = serde_json::to_string(&export).unwrap();
    assert!(text.contains("MY_PERSONAL_GROUP_INPUT"));
    assert!(!text.contains("OTHER_ORG_MEMBER_INPUT"));
    db.drop().await.unwrap();
}

#[test]
fn crash_reports_export_metadata_without_frames_or_delivery_state() {
    let row = model_fixture("client_error_reports");
    let item = safe_item(&row, fields_for("client_error_reports").unwrap());
    assert_eq!(item["report"]["platform"], "ios");
    assert_eq!(item["report"]["app_version"], "1.2.3");
    assert_eq!(item["report"]["runtime"]["name"], "hermes");
    assert_eq!(item["report"]["native_signal"], "SIGSEGV");
    let serialized = serde_json::to_string(&item).unwrap();
    for forbidden in [
        "js_frames",
        "native_frames",
        "native_images",
        "fingerprint",
        "consent_revision",
        "artifact_id",
        "destination_id",
        "frame-sentinel",
        "delivery-sentinel",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "unexpected diagnostic field: {forbidden}"
        );
    }
    assert_eq!(item.as_object().unwrap().len(), 5);
    assert_eq!(item["report"].as_object().unwrap().len(), 8);
}
