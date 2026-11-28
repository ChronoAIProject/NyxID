# Account data export

`POST /api/v1/users/me/export` returns a JSON attachment for the authenticated
first-party human account. The human-only route and
`require_first_party_human` check reject OAuth-client, delegated, API-key,
service-account and relay credentials. The response is named
`nyxid-account-export-YYYY-MM-DD.json` and uses
`format: "nyxid.account-export"`, `version: 1`, an RFC3339 UTC
`generated_at`, the opaque `account_id`, `limits`, `truncated_sections`,
`excluded`, and nested section/subsection objects.

Each `sections.<section>.<subsection>` value is one explicitly filtered view of
a source collection:

```json
{
  "items": [],
  "total_count": 42,
  "total_count_exact": true,
  "truncated": false,
  "unavailable": false,
  "scope_truncated": false
}
```

Rows are newest first. A subsection exports at most 10,000 rows. Its count query
stops at 10,001 matching rows, across every kind of record. `total_count` remains
a numeric count: when `total_count_exact` is true it is the full matching total;
when false it is a lower bound (normally 10,001), and more rows may exist.
A bounded parent scope also makes the count a lower bound, even below 10,001.
An unavailable source has an unknown total (`total_count: 0`,
`total_count_exact: false`), rather than evidence that it is empty.
If more rows exist, `truncated` is true and
its fully qualified name (`section.subsection`) appears in
`truncated_sections`. The complete serialized document is capped at 32 MiB;
64 KiB is reserved for the envelope and metadata. The remaining bytes initially
provide an equal share to every declared subsection (including the profile,
audit view and additional authored-message view). Unused space is then divided
equally among subsections still limited by bytes. Only those sources are read
again, replacing their earlier pages in the same indexed order; sources that are
complete, unavailable or already at the 10,000-row limit are not read again.
There are at most four passes per source (one initial pass and up to three
redistributions), or eight aggregation queries per source, plus cursor batches.
This bounded max-min allocation preserves each source's initial share while
letting large sources use space that smaller ones do not need. Old pages are
released before their replacements are read, so the combined retained payload
stays within the byte budget. If the final envelope still exceeds the cap, the oldest rows
are removed incrementally from the largest subsections until the document fits.
Every subsection with `truncated: true`, whether limited by rows, its byte share
or final envelope enforcement, appears in `truncated_sections` and the export
audit metadata. Queries have a three-second
MongoDB `maxTimeMS` bound. Each replica permits two concurrent exports. The rate
limit is three exports per person per fixed hourly window; `Retry-After` gives
the seconds until the window resets. A busy or rate-limited request returns
`429` with `Retry-After`; a busy response requests a one-second retry.

The stable section set is `account`, `services`, `security`, `assistant`,
`channels`, `account_activity`, `machines`, and `billing`. Empty sections are
present so clients can discover the schema. The account section also contains
`profile` and `org_memberships` subsections. The profile contains only the
visible account email, display name, avatar URL, email-verification state, MFA
state, social-provider name, profile preferences, and created/updated/last-login
timestamps. Memberships contain organization/member IDs, role, and lifecycle
timestamps.

The remaining subsections are explicit registry mappings. They expose only
user-visible metadata or content appropriate to that collection: service
connections and service metadata; session lifecycle metadata; assistant
private conversation titles, the person's explicitly owner-authored user inputs, assistant replies to web/mobile-origin turns in private conversations without per-message platform routing, and attachment metadata; channel message metadata
and routing metadata; action, notification, machine-job and other account
activity metadata; consented crash and error report metadata (versions, categories and timestamps, without stack frames, native images or delivery state); audit event type/time metadata for events authored by the
person; and billing wallet summary, received grants and usage rollups. Every
source collection has a named field allowlist, owner predicate, subsection name
and newest-first sort in `account_export_service.rs`; there is no generic
collection fallback. Uploaded and generated files are metadata only (name,
MIME type, byte size, creation and expiry timestamps).


### Export ownership

Export selectors are explicit per collection and independent of deletion selectors.
An invite-code record includes only the requesting person's usage elements; it
never includes other redeemers, their emails or the administrator's note.
Shared-agent consent records belong to the acting person, so owning an agent
does not expose a recipient's private conversation. Relationship metadata omits
other people's account identifiers.

Service accounts follow `ServiceAccount::effective_owner_user_id`: an explicit
`owner_user_id` wins, and only legacy rows without an owner fall back to
`created_by`. Creating an org-owned or subsequently transferred service account
does not grant export access. OAuth clients have no organization-owner field;
`oauth_client_service::get_client_for_creator` and `list_clients_for_creator`
use `created_by` as their actual access boundary, so creator-owned client
metadata is included. Organization invites belong to the org; their issuer's
attribution does not grant access. The redeemer can export their own redemption
metadata, without the issuer's identity or the invitation token.

### Complete subsection map

Every declared registry export view has one subsection; a source collection may have multiple explicitly scoped views. The following table is the version-1 field contract; fields containing dates use RFC3339 strings, Decimal128 values use decimal strings, and binary values are omitted. Per-collection allowlists include the visible authored fields for that source: conversation `title`; agent `name`, `description`, `display_name`, `persona`, `voice_instructions`, and the requesting person's own `memory.text`; private-note `notes.text`; feedback `comment`; trigger labels, schedules, and assistant instructions; endpoint labels and URLs; connection labels and metadata; node names; channel-bot labels; OAuth client names, redirect URIs and scopes; and service-account names and descriptions. Shared/team memory, system-injected instructions, and group messages whose `author_user_id` is another person are excluded.

<!-- export-fields:start -->
| Section | Subsection | Allowlisted fields |
| --- | --- | --- |
| `account` | `agent_key_login_requests` | `approved_user_id`, `status`, `created_at`, `expires_at` |
| `account` | `agent_service_bindings` | `user_id`, `created_at`, `updated_at` |
| `account` | `approval_grants` | `user_id`, `service_id`, `expires_at` |
| `account` | `approval_requests` | `user_id`, `service_id`, `service_slug`, `status`, `created_at`, `expires_at` |
| `account` | `assistant_notification_push_budgets` | `reservations.at` |
| `account` | `assistant_notification_push_sources` | `user_id`, `sent_at` |
| `account` | `auth_device_codes` | `approved_user_id`, `status`, `created_at`, `expires_at` |
| `account` | `auth_device_codes_v2` | `approved_user_id`, `status`, `created_at`, `expires_at` |
| `account` | `authorization_codes` | `user_id`, `created_at`, `expires_at` |
| `account` | `cli_pairings` | `user_id`, `kind`, `status`, `created_at`, `completed_at`, `expires_at` |
| `account` | `connect_links` | `user_id`, `service_id`, `service_slug`, `status`, `created_at`, `completed_at`, `expires_at` |
| `account` | `consents` | `user_id`, `expires_at` |
| `account` | `device_onboard_credentials` | `owner_user_id`, `created_at`, `expires_at` |
| `account` | `execution_permission_deliveries` | `user_id`, `conversation_id`, `state`, `created_at`, `expires_at` |
| `account` | `generated_artifact_content_series` | `user_id`, `conversation_id`, `revision`, `purge_after` |
| `account` | `lark_base_write_photos` | `user_id`, `created_at`, `expires_at` |
| `account` | `lark_base_writes` | `user_id`, `state`, `created_at`, `updated_at`, `expires_at` |
| `account` | `mcp_session_slots` | `user_id`, `expires_at` |
| `account` | `mfa_challenges` | `user_id`, `created_at`, `expires_at` |
| `account` | `node_registration_tokens` | `user_id`, `name`, `created_at`, `expires_at` |
| `account` | `oracle_pools` | `user_id`, `slug`, `name`, `created_at`, `updated_at`, `deleted_at`, `is_active` |
| `account` | `oracle_workers` | `enrollment.owner_user_id`, `worker_label`, `generation`, `last_seen_at`, `current_task_id`, `script_version`, `page_url`, `first_seen_at`, `provisioned_at`, `platform`, `capabilities`, `desired_state`, `logged_in`, `chrome_alive`, `last_error`, `cooldown_until` |
| `account` | `org_invites` | `role`, `created_at`, `expires_at` |
| `account` | `org_memberships` | `org_user_id`, `member_user_id`, `role`, `created_at` |
| `account` | `permission_key_policies` | `user_id`, `created_at`, `updated_at` |
| `account` | `platform_bot_installations` | `owner_user_id`, `state`, `created_at`, `updated_at` |
| `account` | `platform_personal_bindings` | `user_id`, `conversation_id`, `state`, `created_at`, `updated_at` |
| `account` | `platform_public_probe_consents` | `expires_at` |
| `account` | `platform_setups` | `owner_user_id`, `state`, `created_at`, `updated_at`, `expires_at` |
| `account` | `profile` | `email`, `display_name`, `avatar_url`, `email_verified`, `mfa_enabled`, `social_provider`, `created_at`, `updated_at`, `last_login_at`, `profile_config.security.skip_mfa_for_approved_sign_ins`, `profile_config.analytics.enabled`, `profile_config.services_view`, `profile_config.service_views`, `profile_config.quick_access`, `profile_config.onboarding.ai_services_completed_at` |
| `account` | `saved_logins` | `user_id`, `created_at`, `updated_at` |
| `account` | `trigger_deliveries` | `user_id`, `status`, `created_at`, `updated_at`, `expires_at` |
| `account` | `trigger_owner_limits` | `version` |
| `account` | `trigger_runs` | `user_id`, `expires_at` |
| `account` | `trigger_work` | `trigger_id`, `user_id`, `kind`, `at` |
| `account` | `triggers` | `user_id`, `label`, `status`, `source`, `schedule.kind`, `schedule.expression`, `schedule.timezone`, `schedule.amount`, `schedule.unit`, `schedule.anchor`, `schedule.at`, `schedule.start`, `schedule.end`, `schedule.max_runs`, `schedule.grace_seconds`, `schedule_state`, `overlap`, `delivery.type`, `delivery.url`, `delivery.conversation_id`, `delivery.confirmation_policy`, `delivery.agent_id`, `delivery.thread_policy`, `delivery.instruction`, `delivery.deliver_to`, `created_at`, `updated_at` |
| `account_activity` | `assistant_action_receipts` | `user_id`, `action`, `status`, `created_at`, `completed_at` |
| `account_activity` | `assistant_cma_session_origins` | `user_id`, `conversation_id`, `service_id`, `created_at` |
| `account_activity` | `assistant_group_agent_access` | `owner_id`, `updated_at` |
| `account_activity` | `assistant_group_execution_slots` | `conversation_id` |
| `account_activity` | `assistant_learning_publication_targets` | `owner_id`, `state`, `created_at`, `updated_at` |
| `account_activity` | `assistant_voice_sessions` | `user_id`, `conversation_id`, `protocol`, `dictation`, `measured_ms`, `state`, `created_at`, `deadline`, `last_user_at`, `closed_at`, `observed_seconds`, `playback_ms`, `final_usage_confirmed`, `billing_finalized` |
| `account_activity` | `assistant_voice_windows` | `user_id` |
| `account_activity` | `assistant_wire_logs` | `user_id`, `conversation_id`, `created_at`, `expires_at` |
| `account_activity` | `audit_log` | `event_type`, `created_at` |
| `account_activity` | `client_error_reports` | `received_at`, `expires_at`, `environment`, `severity`, `report.platform`, `report.kind`, `report.exception_class`, `report.native_signal`, `report.app_version`, `report.app_build`, `report.os_version`, `report.runtime` |
| `account_activity` | `feature_flag_overrides` | `created_at`, `updated_at` |
| `account_activity` | `invite_codes` | `usages.user_id`, `usages.used_at`, `created_at`, `updated_at` |
| `account_activity` | `invocation_diagnostics` | `actor_user_id`, `state`, `created_at`, `completed_at`, `expires_at` |
| `account_activity` | `mcp_session_notifications` | `created_at`, `expires_at` |
| `account_activity` | `openclaw_channel_mappings` | `nyxid_user_id`, `channel`, `channel_user_id`, `created_at`, `updated_at` |
| `account_activity` | `org_channel_link_challenges` | `user_id`, `org_id`, `state`, `created_at`, `expires_at` |
| `account_activity` | `platform_delivery_tests` | `state`, `expires_at` |
| `account_activity` | `platform_onboarding_binding_requests` | `approved_user_id`, `state`, `created_at`, `expires_at` |
| `account_activity` | `platform_onboarding_bindings` | `user_id`, `created_at`, `updated_at`, `expires_at`, `is_active` |
| `account_activity` | `platform_personal_decision_spacing` | `last_decision_at` |
| `account_activity` | `platform_personal_link_challenges` | `state`, `created_at`, `expires_at` |
| `account_activity` | `platform_personal_link_intents` | `state`, `created_at`, `expires_at` |
| `account_activity` | `platform_personal_setup_selections` | `user_id` |
| `account_activity` | `platform_setup_notices` | `kind`, `expires_at` |
| `account_activity` | `platform_setup_steps` | `created_at` |
| `account_activity` | `service_change_events` | `service_id`, `actor.app_id` |
| `account_activity` | `telegram_bot_claims` | `owner_user_id`, `actor_user_id`, `status`, `created_at`, `expires_at` |
| `account_activity` | `telegram_bot_requests` | `owner_user_id`, `actor_user_id`, `status`, `created_at`, `expires_at` |
| `assistant` | `assistant_acknowledgements` | `user_id`, `conversation_id`, `service_id`, `service_slug`, `kind`, `status`, `created_at`, `expires_at` |
| `assistant` | `assistant_agent_consents` | `actor_user_id`, `conversation_id`, `created_at`, `updated_at`, `expires_at` |
| `assistant` | `assistant_agent_learning` | `owner_id`, `created_at`, `updated_at` |
| `assistant` | `assistant_agent_learning_members` | `member_user_id`, `created_at`, `updated_at` |
| `assistant` | `assistant_agent_learning_proposals` | `owner_id`, `status`, `created_at`, `updated_at` |
| `assistant` | `assistant_agent_learning_runs` | `owner_id`, `status`, `created_at`, `updated_at` |
| `assistant` | `assistant_agent_learning_skill_roots` | `owner_id`, `created_at` |
| `assistant` | `assistant_agent_private_notes` | `actor_user_id`, `agent_id`, `revision`, `notes.text`, `notes.created_at`, `notes.updated_at`, `updated_at` |
| `assistant` | `assistant_agent_shares` | `role`, `created_at` |
| `assistant` | `assistant_agent_sharing` | `owner_id`, `updated_at` |
| `assistant` | `assistant_agents` | `user_id`, `name`, `display_name`, `description`, `specialty`, `persona`, `voice_instructions`, `memory.text`, `created_at`, `updated_at` |
| `assistant` | `assistant_artifact_deliveries` | `user_id`, `conversation_id`, `expires_at` |
| `assistant` | `assistant_async_operations` | `user_id`, `conversation_id`, `service_id`, `state`, `created_at`, `expires_at` |
| `assistant` | `assistant_async_quotas` | `user_id`, `generation` |
| `assistant` | `assistant_attachment_expiry` | `user_id`, `conversation_id`, `created_at`, `expired_at` |
| `assistant` | `assistant_attachments` | `user_id`, `content_type`, `size`, `created_at` |
| `assistant` | `assistant_authored_draft_claims` | `claim.conversation_id` |
| `assistant` | `assistant_checkpoints` | `user_id`, `status`, `updated_at` |
| `assistant` | `assistant_cma_agents` | `user_id`, `created_at`, `updated_at` |
| `assistant` | `assistant_cma_creations` | `user_id`, `created_at`, `updated_at` |
| `assistant` | `assistant_cma_sandboxes` | `user_id`, `created_at`, `updated_at` |
| `assistant` | `assistant_conversations` | `user_id`, `title`, `model`, `created_at`, `updated_at`, `message_count` |
| `assistant` | `assistant_direct_handoffs` | `user_id`, `state`, `expires_at` |
| `assistant` | `assistant_group_agent_proposals` | `expires_at` |
| `assistant` | `assistant_group_invites` | `created_at`, `expires_at` |
| `assistant` | `assistant_group_mention_notifications` | `user_id`, `created_at` |
| `assistant` | `assistant_group_messages` | `user_id`, `group_id`, `author_user_id`, `role`, `text`, `created_at` |
| `assistant` | `assistant_group_read_states` | `user_id` |
| `assistant` | `assistant_group_requests` | `actor_user_id`, `created_at` |
| `assistant` | `assistant_groups` | `user_id`, `name`, `created_at`, `updated_at` |
| `assistant` | `assistant_invocation_turns` | `user_id`, `conversation_id`, `state`, `created_at` |
| `assistant` | `assistant_invocations` | `user_id`, `conversation_id`, `created_at` |
| `assistant` | `assistant_message_feedback` | `user_id`, `message_id`, `vote`, `comment`, `created_at`, `updated_at` |
| `assistant` | `assistant_messages` | `user_id`, `conversation_id`, `role`, `text`, `created_at` |
| `assistant` | `assistant_notifications` | `user_id`, `kind`, `created_at`, `updated_at`, `expires_at` |
| `assistant` | `assistant_onboarding` | `intro_version`, `outcome`, `topics`, `work_style`, `created_at`, `updated_at` |
| `assistant` | `assistant_resources` | `user_id`, `conversation_id`, `created_at`, `expires_at` |
| `assistant` | `assistant_service_execution` | `user_id`, `conversation_id` |
| `assistant` | `assistant_settings` | `access_mode`, `skip_destructive_confirmation`, `timezone`, `voice.model`, `voice.language`, `voice.voice`, `voice.input_mode`, `notifications.push.automation_result`, `notifications.push.automation_problem`, `notifications.push.needs_decision`, `notifications.push.agent_update`, `notifications.push.chat_update`, `notifications.push.group_mention`, `updated_at` |
| `assistant` | `assistant_skill_draft_attempts` | `user_id`, `conversation_id`, `created_at` |
| `assistant` | `assistant_tool_results` | `actor_id`, `conversation_id`, `expires_at` |
| `assistant` | `assistant_voice_announcements` | `user_id`, `conversation_id`, `state`, `text`, `created_at`, `expires_at` |
| `assistant` | `assistant_voice_requests` | `user_id`, `conversation_id`, `state`, `created_at`, `expires_at` |
| `assistant` | `async_service_operation_artifacts` | `user_id`, `filename`, `content_type`, `size`, `created_at`, `expires_at` |
| `assistant` | `authored_group_messages` | `group_id`, `role`, `text`, `created_at` |
| `assistant` | `image_publications` | `user_id`, `conversation_id`, `company`, `state`, `stage`, `post`, `verification`, `created_at`, `updated_at` |
| `assistant` | `nyxbot_event_bindings` | `user_id`, `status`, `created_at`, `updated_at` |
| `assistant` | `nyxbot_events` | `user_id`, `conversation_id`, `status`, `created_at`, `expires_at` |
| `assistant` | `nyxbot_gateway_responses` | `user_id`, `conversation_id`, `created_at`, `expires_at` |
| `assistant` | `nyxbot_image_deliveries` | `user_id`, `conversation_id`, `state`, `created_at` |
| `assistant` | `nyxbot_image_deliveries_v2` | `user_id`, `conversation_id`, `state`, `created_at` |
| `assistant` | `nyxbot_threads` | `user_id`, `conversation_id`, `kind`, `created_at`, `updated_at` |
| `assistant` | `nyxbot_watches` | `user_id`, `conversation_id`, `kind`, `status`, `created_at`, `expires_at` |
| `assistant` | `oracle_sessions` | `owner_user_id`, `origin`, `tag`, `turn_count`, `last_task_id`, `chatgpt_url`, `owner_worker_label`, `closed_at`, `created_at`, `updated_at` |
| `assistant` | `oracle_tasks` | `submitter_user_id`, `kind`, `target_url`, `prompt`, `model_label`, `project_url`, `tag`, `pdf_name`, `attachment_name`, `status`, `conversation_id`, `required_worker_label`, `phase`, `phase_detail`, `response`, `response_chars`, `chatgpt_url`, `failure_reason`, `observed_model_switcher`, `observed_model_effort`, `created_at`, `updated_at`, `completed_at`, `expires_at` |
| `billing` | `credit_grants` | `recipient_user_id`, `target_kind`, `amount`, `remaining`, `reserved`, `scope`, `expires_at`, `reason`, `status`, `created_at`, `updated_at`, `consumed_at`, `expired_at`, `revoked_at` |
| `billing` | `credit_schedule_recipients` | `schedule_id`, `recipient_user_id`, `anchor_at`, `next_due_at`, `is_eligible`, `created_at`, `updated_at` |
| `billing` | `usage_meter` | `billing_owner_id`, `service_id`, `service_slug`, `metric`, `quantity`, `status`, `created_at` |
| `billing` | `usage_workspaces` | `config`, `revision`, `updated_at` |
| `billing` | `wallet` | `owner_id`, `plan_kind`, `balance_credits`, `reserved_credits`, `pending_lago_debits`, `pending_topup_expiry_credits`, `has_payment_instrument`, `overdraft_cap_credits`, `suspended`, `collection_state`, `balance_synced_at`, `created_at`, `updated_at` |
| `channels` | `channel_activity_notifications` | `user_id`, `conversation_id`, `direction`, `content_type`, `platform`, `created_at` |
| `channels` | `channel_adapter_allocations` | `user_id`, `created_at` |
| `channels` | `channel_adapter_bindings` | `user_id`, `created_at`, `updated_at` |
| `channels` | `channel_adapter_events` | `user_id`, `status`, `created_at` |
| `channels` | `channel_adapter_resource_claims` | `user_id`, `created_at` |
| `channels` | `channel_adapter_sessions` | `user_id`, `conversation_id`, `state`, `created_at`, `expires_at` |
| `channels` | `channel_bots` | `user_id`, `label`, `platform`, `status`, `is_active`, `created_at`, `updated_at` |
| `channels` | `channel_connect_links` | `user_id`, `status`, `created_at`, `expires_at` |
| `channels` | `channel_conversations` | `user_id`, `created_at`, `updated_at` |
| `channels` | `channel_delivery_receipts` | `user_id`, `expires_at` |
| `channels` | `channel_email_batches` | `user_id`, `expires_at` |
| `channels` | `channel_email_receipts` | `user_id`, `created_at` |
| `channels` | `channel_email_sends` | `user_id`, `status`, `created_at`, `updated_at` |
| `channels` | `channel_email_subscriptions` | `user_id`, `created_at` |
| `channels` | `channel_guest_setup` | `owner_id`, `revision`, `pending_activation`, `policy_revision`, `updated_at` |
| `channels` | `channel_messages` | `user_id`, `conversation_id`, `direction`, `platform`, `content_type`, `callback_status`, `created_at` |
| `channels` | `channel_reply_deliveries` | `user_id`, `created_at` |
| `channels` | `channel_text_deliveries` | `user_id`, `created_at` |
| `channels` | `channel_whatsapp_native` | `owner_id`, `updated_at` |
| `channels` | `nyxbot_channels` | `user_id`, `status`, `created_at`, `updated_at` |
| `channels` | `org_channel_lanes` | `actor_id`, `conversation_id`, `created_at` |
| `channels` | `org_channel_messages` | `channel_id`, `conversation_id`, `role`, `text`, `created_at` |
| `channels` | `org_channel_sender_links` | `user_id`, `channel_id`, `org_id`, `active`, `created_at` |
| `machines` | `machine_confirmations` | `user_id`, `node_id`, `state`, `created_at`, `expires_at` |
| `machines` | `machine_free_slots` | `machine_id`, `state`, `created_at`, `updated_at` |
| `machines` | `machine_jobs` | `user_id`, `node_id`, `state`, `created_at`, `expires_at` |
| `machines` | `machine_orphan_inventory` | `owner_id`, `machine_id`, `state`, `reason`, `created_at`, `last_seen_at` |
| `machines` | `nodes` | `user_id`, `name`, `status`, `is_active`, `created_at`, `updated_at` |
| `security` | `device_codes` | `approved_by_user_id`, `status`, `created_at`, `expires_at` |
| `security` | `login_approvals` | `user_id`, `expires_at` |
| `security` | `login_codes` | `user_id`, `status`, `created_at`, `expires_at` |
| `security` | `mfa_factors` | `user_id`, `is_active`, `created_at`, `updated_at` |
| `security` | `notification_channels` | `user_id`, `created_at`, `updated_at` |
| `security` | `refresh_tokens` | `user_id`, `created_at`, `expires_at` |
| `security` | `sessions` | `user_id`, `purpose`, `ip_address`, `user_agent`, `revoked`, `created_at`, `expires_at`, `last_active_at` |
| `services` | `api_key_credentials` | `user_id`, `kind`, `is_active`, `created_at`, `expires_at` |
| `services` | `api_keys` | `user_id`, `name`, `is_active`, `created_at`, `updated_at`, `expires_at` |
| `services` | `catalog_delegation_grants` | `user_id`, `created_at`, `expires_at` |
| `services` | `oauth_broker_bindings` | `user_id`, `created_at` |
| `services` | `oauth_clients` | `created_by`, `client_name`, `redirect_uris`, `allowed_scopes`, `grant_types`, `client_type`, `is_active`, `delegation_scopes`, `default_service_catalog_slugs`, `broker_capability_enabled`, `connection_webhook_url`, `revocation_webhook_url`, `connection_webhook_enabled`, `created_at`, `updated_at` |
| `services` | `oauth_consent_requests` | `user_id`, `expires_at` |
| `services` | `oauth_delegation_authorities` | `user_id`, `created_at`, `expires_at` |
| `services` | `oauth_states` | `user_id`, `created_at`, `expires_at` |
| `services` | `service_account_key_read_grants` | `owner_id`, `expires_at` |
| `services` | `service_accounts` | `owner_user_id`, `name`, `description`, `role_ids`, `allowed_scopes`, `is_active`, `purpose`, `rate_limit_override`, `created_at`, `updated_at`, `last_authenticated_at` |
| `services` | `service_approval_configs` | `user_id`, `service_id`, `created_at`, `updated_at` |
| `services` | `service_pool_member_health` | `owner_id`, `updated_at`, `expires_at` |
| `services` | `service_pools` | `user_id`, `slug`, `name`, `is_active`, `created_at`, `updated_at` |
| `services` | `service_preferences` | `version`, `ordered`, `updated_at` |
| `services` | `user_api_keys` | `user_id`, `status`, `created_at`, `updated_at`, `expires_at` |
| `services` | `user_endpoints` | `user_id`, `label`, `url`, `catalog_service_id`, `openapi_spec_url`, `recommended_skills`, `created_at`, `updated_at` |
| `services` | `user_provider_credentials` | `user_id`, `created_at`, `updated_at` |
| `services` | `user_provider_tokens` | `user_id`, `status`, `created_at`, `updated_at`, `expires_at` |
| `services` | `user_service_connections` | `user_id`, `service_id`, `credential_type`, `credential_label`, `metadata`, `is_active`, `created_at`, `updated_at` |
| `services` | `user_services` | `user_id`, `slug`, `endpoint_id`, `catalog_service_id`, `service_type`, `is_active`, `created_at`, `updated_at` |
<!-- export-fields:end -->

The two group-message views read `assistant_group_messages` with disjoint
filters. `assistant_group_messages` includes only messages both owned and
authored by the requesting person. `authored_group_messages` includes only that
person's authored messages in groups they do not own, using
`{author_user_id: account_id, user_id: {$ne: account_id}}`. It does not include
other members' messages or the group's transcript. Both views sort newest first,
have independent 10,000-row caps, and share the export's 32 MiB byte budget.

The export omits password and authentication hashes, session/token hashes,
OAuth codes and states, raw access or refresh tokens, API-key prefixes and
hints, push and webhook tokens, private keys, KMS/JWK identifiers, wrapped
keys, DEKs, nonces, signatures, HMACs, ciphertext, binary payloads, encrypted
file contents, transient lease/coordination/deduplication records, erasure
bookkeeping, and retained financial or integrity content. MCP session IDs and their storage hashes are excluded, along with the session's
`absolute_expires_at` and `slot_recovery_revision`. Notification and slot projections
use the fields listed above across both formats. The shared `security_rollouts`
issuance fence revision is excluded as fleet metadata. Decimal128 values are emitted as decimal strings;
BSON documents and arrays are converted recursively, dates are RFC3339 strings,
and BSON binary values are omitted. Every URL-valued allowlist field uses one
sanitizer: every value is reduced to `scheme://host[:port]`, removing userinfo,
path, query, and fragment. Whenever a URL is reduced, the export emits a sibling
`<field>_path_omitted: true` marker (for nested fields, the marker sits beside
the nested URL; arrays use the URL field's marker). The path is omitted because
providers commonly put access tokens or webhook secrets in path segments. Full
endpoint, webhook, trigger-destination, redirect-URI, and profile avatar
addresses remain visible in the web and mobile controls where those values are
managed; operational Oracle browser/worker addresses are not claimed to have a
full-address account view. Instructions and memory export only the requesting
person's authored text; shared/team memory, system-injected instructions, and
messages authored by other people are omitted. Owner-keyed shared conversations can contain guest content: only explicitly owner-authored user inputs are exported there. Assistant replies and legacy inputs without authorship are exported only in private conversations, excluding collaboration, platform, channel, group, guest and invocation-controlled threads. Shared titles are excluded because they may derive from a guest message. The successful response emits a chained
metadata-only `user.data_exported` audit event containing format/version,
subsection counts, truncation names and serialized byte size; it never records
export content.


## Orphan cleanup operations

The operator orphan sweep starts a persisted leased job and returns HTTP 202; poll by job ID for its report. It scans bounded keyset pages and checks liveness in batches. Run a dry run, apply its digest, and repeat the dry run until it reports no candidates; children of parents removed during an apply appear on the next pass. See `API.md` for the start/poll contract and transactional mutation fences.

A source query failure or timeout marks that kind of record `unavailable: true`
and lists its full subsection name in `unavailable_sections`. Its items and count
are empty/zero because no complete result is available; other sources continue.
The audit records the same unavailable list without database diagnostics. Account
lookup, encoding and mandatory audit failures fail the request; a server failure
refunds only its original fixed-window slot. Slots are reserved atomically before
work starts, so concurrent requests cannot bypass the hourly limit.

Joined views resolve at most 10,000 parent IDs using the parent's indexed owner
predicate, then match children by the indexed parent key. They never look up a
parent for every child. If parent resolution reaches its cap, `scope_truncated`
and `truncated` are true; `total_count` is a lower bound covering only the
resolved parent scope. Source-query failures still mark the view unavailable.

Assistant-message reads resolve up to 10,000 private conversation ids once per export, then use indexed message predicates with batches of 100. Explicit owner-authored inputs do not need a conversation join. If private scope is capped, `scope_truncated` is true.

Reply classification uses each stored message’s origin, platform origin and platform reply fields (`assistant_nyxagent::begin_owner_turn_in_session` and settled-turn message writes), not the conversation’s mutable `reply_channel`. Replies to channel-origin turns, platform-routed replies and replies in shared/guest conversations are excluded. Changing a delivery preference cannot change which previously stored replies qualify.

Storage-fixture coverage uses production serde models wherever the writer has a
storage model. `channel_activity_notifications` uses `ChannelMessage` selected by
`channel_activity_service::collection`; `org_channel_messages` serializes
`GroupMessage` plus the three raw routing selectors added by
`org_channel_service::transcript::publish`. These are not independent message
schemas. The remaining typed test envelopes below describe genuine raw BSON
writers: the feature stores `Document` directly and has no whole-row serde model.
They intentionally omit unknown fields during round-trip validation.

| Collection | Raw writer and reason for a test envelope |
| --- | --- |
| `assistant_async_quotas` | `async_service_operation::admit_in_session`: mutable quota counter, no storage model. |
| `assistant_attachment_expiry` | `assistant_upload_retention::delete_root`: expiry tombstone copied from scoped attachment metadata. |
| `assistant_authored_draft_claims` | `assistant_skill_authoring_claims::reserve/save_claim`: owner envelope around the production `AuthoredDraftClaim` model. |
| `assistant_learning_migrations` | `assistant_agent_learning_review::backfill_package_format`: global migration marker. |
| `assistant_notification_push_budgets` | `assistant_notification_service::claim_push`: reservation timestamp array. |
| `assistant_notification_push_sources` | `assistant_notification_service::claim_push`: deduplication reservation. |
| `assistant_service_execution` | `assistant_service_execution::check_inner/attach_org_offer`: acknowledgement-to-execution binding envelope. |
| `encrypted_record_migration_findings` | `encrypted_record_migration::advance`: keyed diagnostic document, excluded from export. |
| `encrypted_record_migration_runs` | `encrypted_record_migration::start`: migration coordination document, excluded from export. |
| `machine_orphan_inventory` | `remote_machine_events::record_investigation`: diagnostic resource investigation document. |
| `platform_guide_selections` | `platform_setup_service::guide_select_current`: binding/generation selection counter. |
| `platform_native_replies` | `platform_binding_service::admit_native_reply_in_session`: provider reply deduplication document. |
| `platform_personal_decision_spacing` | `platform_link_intent_service::admit_decision`: per-person timestamp counter. |
| `trigger_owner_limits` | `trigger_service::create`: atomic owner version counter. |
| `trigger_work` | `trigger_schedule::insert_run/set_schedule_job`: queued scheduled occurrence document. |
| `org_channel_lanes` | `org_channel_service::transcript::lane`: actor/routing selectors and generated conversation id. |
| `org_channel_link_challenges` | `org_channel_service::links::issue`: one-time link challenge document. |

Delivery slots are refunded if the request is cancelled during building or the
response body is dropped before its final slice is handed off, on server failures,
or when more than half
of all declared subsections are unavailable. A separate durable fixed-hourly
limit allows at most 12 admitted attempts per person, including refunded attempts;
three delivered exports remain the normal hourly limit. Busy requests consume
neither budget. The two concurrency permits remain held until the final slice is
handed to the transport, the body is dropped, or its deadline expires, bounding
builders and buffered deliveries together. A complete hand-off remains charged
even when HTTP/1 stops polling at Content-Length without a separate EOF poll.
Refund cleanup is tracked by graceful shutdown and affects only the original reservation window.

Joined views reuse existing child indexes for MCP notifications and setup notices. Explicit indexes on delivery-test installation ids and setup-step setup ids support the other two joined reads; their rollout commands and awaited startup cost are in `security/PERSONAL_ERASURE_INVENTORY.md`.

`account_export_service::BUILD_BUDGET` allows 60 seconds total from admission
for source reads, parent resolution and allocation rereads, rather than 60 seconds
per source. At expiry the in-flight read is cancelled, and both unread subsections
and a subsection whose redistribution reread is in flight are marked `unavailable`
and listed in `unavailable_sections`. The earlier page of a subsection being reread
was released before the reread to stay within the byte budget. Completed pages not
being reread are kept, and no further redistribution reads start. These unavailable
subsections count toward the existing mostly-unavailable delivery-slot refund. Finalizing the bounded
JSON envelope follows without more database source reads. `limits.build_budget_ms`
reports this budget.

The response body has a total 120-second delivery deadline starting when it is
created. The backend emits at most 64 KiB per slice and sets `Content-Length` to
the exact serialized byte size. The API stack does not compress responses; the
frontend nginx export location disables gzip and response buffering to preserve
that length and propagate an upstream abort. **Do not compress
`POST /api/v1/users/me/export`** at any ingress, reverse proxy or CDN; preserve the
length and disable response buffering (see `DEPLOYMENT.md`). At the deadline, the backend drops
its remaining bytes independently of reader polling, releases the replica
concurrency permit, and errors the body so transport aborts rather than ending
with a clean EOF. A slice already handed to the transport can remain until that
transport drops it. Expiry is a non-delivery: the delivery slot is refunded,
while the separate hourly attempt remains charged.

Web and mobile reject transport/read errors and byte counts that differ from a
present `Content-Length` only when `Content-Encoding` is absent or `identity`.
Fetch exposes decoded bytes; a nonidentity encoding makes that length unusable,
so clients use their JSON/structural fallback. When an unencoded length matches,
web checks the first and last non-whitespace bytes for `{` / `}` through at most
4 KiB blob slices and saves the original blob directly, without a full body string
or parsed object graph. This is a completeness/structure check, not full syntax
validation. With no usable length, web fully parses the JSON and checks the format
before downloading or showing success. Mobile never reads the temporary file back or parses the full
body: when the unencoded length is present, the exact byte count proves completeness. When
it is absent or an encoding is present, mobile requires a clean transport completion and checks that the
first and last non-whitespace bytes are `{` and `}`. It tracks only those two bytes
while writing, using constant validation memory; this is a structural check, not
a full JSON syntax check. On failure, mobile removes the temporary file and never
opens the share sheet.

Mobile also bounds stalled native requests and readers independently of their
AbortSignal support. `ACCOUNT_EXPORT_HEADERS_TIMEOUT_MS` bounds the header wait
to 75 seconds: the server's 60-second build budget plus 15 seconds for admission
and network overhead. `ACCOUNT_EXPORT_IDLE_TIMEOUT_MS` is 30 seconds without new
bytes, armed only after headers arrive and renewed only by nonempty byte chunks.
`ACCOUNT_EXPORT_DEADLINE_MS` is 195 seconds from starting the export: the 75-second
header bound plus the server's 120-second delivery deadline, including validation;
progress cannot extend it. All three watchdogs abort the
request and race pending async work so even an unsettled native `read()` fails.
The temporary file is closed and removed before waiting for native cancellation;
`ACCOUNT_EXPORT_CANCEL_TIMEOUT_MS` bounds that wait to one second, and a lock
release error from a still-pending read is ignored. The existing localized export
error toast appears after cleanup, with no share sheet. All timers are cleared on
success or failure. Network watchdogs stop before the user-controlled share sheet.
