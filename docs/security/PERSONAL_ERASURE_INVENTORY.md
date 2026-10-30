# Personal-account erasure inventory and acceptance contract

This runbook describes personal-account erasure acceptance, durable cleanup and
provider revocation. Acceptance fences account authority; completion waits for
the recorded local and remote obligations. Queuing vendor erasure does not
establish completion, and a scan for `user_id` alone cannot establish ownership.
The [coverage limits below](#coverage-limits-and-follow-up-tests) distinguish
the implemented cleanup from the remaining completeness and recovery tests.

## Acceptance before account fencing

Only `UserType::Person` uses personal erasure. Organization users must use the
existing organization lifecycle. Admin removal does not grant owner-confirmed
Machine workspace destruction. The explicit
`machine-data-provider-revocation-v2` policy boundary and required Machine
lifecycle evidence remain in force.

Preparation checks credential dependencies without closing account authority.
The acceptance transaction checks and fences lifecycle prerequisites before
committing the intent and account fence. A rejected request preserves
authentication and account usability; it dispatches no vendor erasure and
revokes no unrelated credentials. Temporary email lifecycle claims are released
after the acceptance attempt or expire after process loss.

| Prerequisite | Required behavior and race fence |
| --- | --- |
| Organization recovery | The person cannot be the sole active admin of a live org. Require promotion/transfer or deliberate org deletion. Membership removal/demotion and personal-erasure acceptance must contend on the same org fence; two simultaneous admin deletions cannot each rely on the other. Count live, nondeleting people, not only unrevoked membership rows. |
| Calls, SMS and email | Settlement and active-effect guards run before acceptance. Aurinko lifecycle coordination and call owner-admission fences preserve a rejected account's usability. Billing and delivery evidence remain subject to their existing lifecycle. |
| Oracle pools | Reuse pool lifecycle admission, active-task/command guards, tombstone, credential invalidation, mandatory chained audit and bounded durable cleanup. No forced abandonment. Settle expired commands using the existing bounded contract. |
| Managed Machines | Preserve owner policy, lifecycle version, revocation outbox and remote cleanup evidence. An admin may fence execution without authorizing indefinite workspace destruction. |
| Remote provider grants | Group the person's own live keys/tokens by provider and OAuth app lineage, using the same owner and lineage rules as ordinary disconnect. Other owners' local rows are never targeted. A grant-wide provider call can invalidate another owner's imported copy of that upstream grant, as ordinary disconnect can. |
| BYO OAuth clients | Refuse acceptance while another owner's live provider token or pending OAuth state references the person's OAuth client credentials. Those owners must reconnect or remove the dependency. A token copy held by another owner never blocks acceptance. |

Preparation is bounded and keys only on the person's
`credential_dependency_revision`; it does not close account authority. While
it runs, self/admin deletion returns HTTP 409, `12801 erasure_preparing`.
Web and CLI back off and retry that specific response. Unrelated owners'
credential imports do not invalidate the snapshot. A ready snapshot is checked
again in the acceptance transaction. Scope plans are generation-bound metadata
children in `personal_erasure_preparations`, keyed by source collection and ID;
preparation never stamps live credential rows. Bounded cleanup removes old
generations before a new scan, and local erasure removes all preparation data.
Planning pages and superseded-row cleanup commit atomically with their
generation and lease check; stale workers cannot alter successor plans.
For a grant group, preservation selects its newest refresh-token member, else
its newest access-token member, before creating the shared effect. This keeps
one bounded source transaction while preventing a stale access-only sibling
from supplying the group's revocation capability. Only `Delivered` confirms a
`Grant` effect; `NotFound` alone requires `grant_scope_unconfirmed`. `Token`
effects accept `Delivered` or `NotFound`. Bulk domain retries rearm only effects
explicitly classified `single_attempt: false`; single-attempt retries use the
per-effect reconciliation disclosure.

## Durable phases and completion

The accepted intent has a deterministic per-person identity, explicit policy
basis and majority-durable account fence. The accepted transition fences user,
session, MFA, approval, Machine and ordinary OAuth authority. Login and writer
paths check the same lifecycle barrier before creating successor authority.
The intent survives removal of the user document and authorizes only the
enumerated cleanup work; possession of its ID grants no API access.

Each bounded worker pass claims a lease with a fresh fencing token and expiry,
records its phase/cursor and outcome, and commits progress with that token.
Expired leases are reclaimable. Source ownership/version CAS prevents deleting
a resource transferred after discovery. Uncertain remote effects retain their
encrypted retry capability and exact destination/app lineage before local purge.
Retry errors use finite diagnostic categories, never provider response bodies.

Jobs expose `accepted`, `pending`, `operator_action` and `complete` states.
Domain checkpoints record local cleanup, required remote revocations, vendor
asynchronous deletion and policy-retained exceptions. API acceptance, a
scheduled task, HTTP 2xx or an empty source query is not evidence of remote
completion. An operator can see pending/action-needed evidence through an
authenticated, authorized surface. No unauthenticated status link is a bearer
grant. Retained immutable evidence is described as retained, not erased.

Disconnects from the keys UI, provider connections and assistant actions, as
well as personal erasure, retain durable provider-revocation obligations.
Provider source removal and its encrypted revocation intent commit atomically.
A group produces one `Grant` effect when effective revocation has
`revokes_grant` and at least one member is non-legacy; otherwise it produces
one `Token` effect per credential. Grant-wide-only providers use `Grant`.
The deletion confirmation discloses that supported provider authorizations,
including the person's other connections sharing a grant, will be revoked;
unsupported or unconfirmed outcomes remain visible in deletion status.

Grant effects, and token effects whose provider is not `verified_token_only`,
get at most one automatic attempt within five minutes of
`max(created_at, retry_armed_at)`. Immediately before dispatch, a live key/token
of the same owner, provider and OAuth app lineage created after that origin
prevents dispatch (`grant_scope_successor_present`). An elapsed window becomes
`grant_scope_window_elapsed`; an uncertain/failed attempt or lease reclaim
after a prior claim becomes `grant_scope_unconfirmed`. All require operator
action. Only explicit `retry` or `retry_with_current_provider_config`, bound to
`expected_retry_generation` and an idempotency key, records `retry_armed_at`
and re-arms one attempt. The successor check runs again. **Any authorization
granted since at the provider for this OAuth app will also be revoked**;
re-arming is an explicit operator decision disclosed in the response, admin UI
and CLI prompt.

Only token effects on `verified_token_only` providers retain bounded automatic
retries with backoff. That classification requires explicit `TokenOnly` with
`revokes_grant: false`, or GitHub request scope; default RFC 7009 does not
qualify. Dedup and receipt matching are per owner, token SHA-256 and captured
revocation-authority digest. Obligation identity stays in
`personal_erasure_effects`; delivery receipts live separately in
`personal_erasure_revocation_receipts` (exact `user_id` ownership). The erasure
registry classifies receipts as Delete at final source cleanup, in bounded
pages, after every provider obligation completes. Receipts never participate
in effect status queries and remain available throughout provider draining.
Receipt creation commits with a write to the live effect lease in one bounded
transaction, so stale dispatch cannot recreate receipts after final cleanup.
A receipt lease expires with its parent effect lease and cannot extend dispatch
authority.

An effect becomes `complete` only with a confirmed provider receipt or an
audited operator resolution with an evidence reference.
`record_external_revocation_confirmed` is available for every `operator_action`
reason, including unreadable preserved capabilities.
`record_remote_revocation_unavailable` also resolves unsupported
revocation, unavailable capabilities, and exhausted or permanent failures;
this records an exception and does not claim provider confirmation. Pending or
in-flight effects cannot be resolved. Only `retry_with_current_provider_config`
decrypts capability material during reconciliation. Both resolutions and
automatic completion remove `capability_encrypted` in that same update.
Unresolved `operator_action` keeps the encrypted capability for an explicit
retry or resolution. Personal erasure
cannot report complete while any effect is pending, unresolved, or retains
capability ciphertext. The DELETE acceptance response includes a bounded page
of effect outcomes. `GET /admin/users/{user_id}/erasure` paginates all outcomes,
including completed live-user disconnects when no erasure job exists. Receipt
metadata is stored separately from the obligation list. Final source cleanup
removes the owner's receipts in bounded pages and deletes the user document
after the provider obligations and local inventory have settled. The durable
job remains available after the user document is gone; overall `complete` also
requires the remaining domain checkpoints, including vendor erasure, to finish.

## Ownership and lifecycle rules

Cleanup uses explicit owner and child-join predicates with bounded domain
adapters. The table describes their ownership and retention rules. The
model-reference appendix is discovery input, not a complete collection registry
or authority for blanket deletion.

| Group | Ownership / action | Retained or shared state |
| --- | --- | --- |
| Account authority | Person `_id`; session, challenge, refresh, approval, code, pairing, MCP and OAuth authority rows join the person or an owned key. Revoke/fence first, purge only after no remaining consumer needs their evidence. | Immutable security audit stays intact. OAuth client/grant relations can belong to other people. |
| Unified services / API keys | `user_services`, `user_endpoints`, `user_api_keys`, key credentials and agent service bindings use real polymorphic ownership and child joins. Reuse disconnect/revocation lifecycle and preserve encrypted retry material before purge. | `credential_user_id` is not necessarily resource ownership. Do not remove org-owned connections. All of the person's same-grant connections are already within personal erasure scope. |
| Provider credentials | Person-owned `user_provider_credentials` and provider tokens plus states bound to actual credential/target owner. Revoke token or grant according to supported provider semantics, with durable outcomes. | `provider_configs` and `platform_credentials` are shared configuration; creator/updater attribution is not ownership. |
| Nodes | Person-owned `nodes`, bindings, registration tokens and pending deliveries. Reuse node lifecycle/remote-Machine guards and close active WS/proxy authority across replicas before record removal. | Org-owned bindings may route through a personal physical node; remove the personal node capability without deleting org services. Recipient E2E ciphertext has its own expiry/owner contract. |
| Machine / saved login | Owner-confirmed Machine lifecycle, saved logins and setup/desktop/update metadata. Preserve signed remote deletion basis, version fences and outbox until cleanup resolves. | Jobs, operations, billing and lifecycle evidence remain as required by existing retention and remote revocation. Admin removal does not silently erase workspaces. |
| Oracle | Pool `user_id` is owner; worker `owner_user_id` can be a person contributing to an org pool. Revoke that worker while preserving the pool. Reuse tombstone/cleanup contract for personally owned pools. | Snapshot/profile creator, task submitter and command creator are attribution. Existing pool-bound encrypted formats, terminal history/TTL and tombstone owner history remain valid. |
| Channel bots / connections | Bot polymorphic owner plus conversation/message joins. Use webhook deregistration and durable connection-backed retry semantics before credential purge. Release a person's NyxBot link/route through existing owner APIs. | Preserve org bots and their other consumers. Sender/recipient/creator fields alone do not confer ownership. Delivery, usage and settlement evidence need their existing lifecycle. |
| Assistant / NyxBot | Person-owned conversations, short-lived `assistant_resources` source locators, attachments (including upload variants), messages, agents, groups, learning bodies, checkpoints, settings, watches and encrypted deliveries. Local cleanup includes `assistant_invocations`, `assistant_invocation_turns`, `lark_base_writes`, `lark_base_write_photos` and `channel_text_deliveries` by exact `user_id`. Channel media selectors/outcomes and ingestion receipts are embedded in owner-owned `nyxbot_events`, `assistant_messages` and `assistant_attachments`; all upload roots and chunks share the attachment collection. Media admission, upload commits and Base ledger claims contend on the durable owner-erasure fence. Base ledger cleanup removes active and terminal local metadata; it does not delete records or uploaded files already sent to an external Lark/Feishu Base. Join child records to authoritative owner, then delete in bounded batches. | Shared authored/published learning, group members and messages need their actual owner and publication contract. A creator reference alone must not destroy shared content. `assistant_uploads` is not a collection. |
| Platform surfaces | Reuse personal-owner retirement, installation/binding/setup/public-session/turn cleanup and credential-owner lifecycle. | Sponsor, payer, initiating actor and manager ingress receipts can represent someone else's shared service or retained effect. Preserve shared bot funding/configuration and mandatory cleanup. Shared `platform_help_libraries`, `platform_help_revisions` and `platform_help_assignments` belong to the platform publication lifecycle; `published_by` is attribution, not personal ownership, so erasing a publisher preserves reviewed content used by other bots. |
| Services / clients / service accounts | Resolve canonical owner; use explicit legacy fallback only where its model contract permits. Deactivate authored OAuth clients through their lifecycle and retain consent-revocation receipts; follow service-account child tokens and grants. | Creator on platform catalog, provider and feature settings is attribution. Shared services are transferred or retired by their manager contract, not deleted by creator search. |
| Organization membership | Remove/revoke the person's memberships and pending registrations/invites with last-admin fencing. | Preserve org user, wallets, resources, role scopes and other memberships. Never dissolve an org implicitly. |
| Billing / grants / effects | Settle outstanding operations and preserve immutable financial/effect records. Apply existing grant/wallet/account lifecycle to personally owned balances. | Billing ledger, audit chain, retained settlement/checkpoint/operation and transfer evidence remain under their existing retention rules. Personal erasure does not scrub, TTL-delete or re-sign them. |
| Telemetry | Central consent/account fence, durable dispatch settlement and current provider-generation drain precede vendor deletion/no-data attestation. Fan out historical destinations and keep unresolved configuration actionable. | Capture acceptance does not prove provider processing is drained. Completion requires supported authenticated deletion/status evidence or a current-generation operator attestation with the same transport/drain fences. |

## Coverage limits and follow-up tests

The full collection registry and its completeness test remain follow-up work in
[#1930](https://github.com/ChronoAIProject/NyxID/issues/1930). The registry must
cover model constants, service-layer literal collections and embedded variants,
with each collection's exact owner or join predicate, bounded index, retained
evidence and prerequisites. The current adapters and bookkeeping registry do
not establish that every collection has been classified.

The full-collection interrupt/resume test also remains follow-up work in #1930.
Focused tests cover acceptance, domain recovery and provider-effect completion;
they do not prove interruption and recovery across every collection in one
erasure. These gaps limit the completeness claim, even though the implemented
job reports completion only after its recorded obligations settle.

## Model-reference discovery appendix

The model-reference scan below is retained as discovery input. It is not
exhaustive, and its field matches include false ownership candidates such as
creator attribution and cursor IDs. A reference alone authorizes neither
deletion nor indefinite retention; its owner and lifecycle require source
tracing and an explicit classification.

<!-- Model discovery appendix follows; predicates are deliberately not generated. -->

```text
backend/src/models/agent_key_login_request.rs: COLLECTION_NAME=agent_key_login_requests | approved_user_id, denied_by_user_id
backend/src/models/agent_service_binding.rs: COLLECTION_NAME=agent_service_bindings | user_id
backend/src/models/api_key.rs: COLLECTION_NAME=api_keys | assistant_agent_owner_id, user_id
backend/src/models/api_key_credential.rs: COLLECTION_NAME=api_key_credentials | user_id
backend/src/models/approval_grant.rs: COLLECTION_NAME=approval_grants | user_id
backend/src/models/approval_request.rs: COLLECTION_NAME=approval_requests | actor_user_id, user_id
backend/src/models/assistant_acknowledgement.rs: COLLECTION_NAME=assistant_acknowledgements | user_id
backend/src/models/assistant_action_receipt.rs: COLLECTION_NAME=assistant_action_receipts | user_id
backend/src/models/assistant_agent.rs: COLLECTION_NAME=assistant_agents | created_by, user_id
backend/src/models/assistant_agent_credential.rs: COLLECTION_NAME=assistant_agent_credentials | user_id
backend/src/models/assistant_agent_learning.rs: CONFIG_COLLECTION_NAME=assistant_agent_learning, MEMBERS_COLLECTION_NAME=assistant_agent_learning_members, RUNS_COLLECTION_NAME=assistant_agent_learning_runs, PROPOSALS_COLLECTION_NAME=assistant_agent_learning_proposals, REJECTIONS_COLLECTION_NAME=assistant_agent_learning_rejections, ROOTS_COLLECTION_NAME=assistant_agent_learning_skill_roots, PUBLICATION_TARGETS_COLLECTION_NAME=assistant_learning_publication_targets, MIGRATIONS_COLLECTION_NAME=assistant_learning_migrations, AUTHORED_CLAIMS_COLLECTION_NAME=assistant_authored_draft_claims | actor_id, member_user_id, owner_id
backend/src/models/assistant_attachment.rs: COLLECTION_NAME=assistant_attachments | user_id
backend/src/models/assistant_checkpoint.rs: COLLECTION_NAME=assistant_checkpoints | user_id
backend/src/models/assistant_conversation.rs: COLLECTION_NAME=assistant_conversations | agent_owner_id, user_id
backend/src/models/assistant_group.rs: COLLECTION_NAME=assistant_groups, MESSAGES_COLLECTION_NAME=assistant_group_messages, REQUESTS_COLLECTION_NAME=assistant_group_requests | actor_user_id, author_user_id, created_by, created_by_user_id, user_id
backend/src/models/assistant_invocation.rs: COLLECTION_NAME=assistant_invocations, TURNS=assistant_invocation_turns | user_id
backend/src/models/assistant_message.rs: COLLECTION_NAME=assistant_messages | user_id
backend/src/models/assistant_message_feedback.rs: COLLECTION_NAME=assistant_message_feedback | user_id
backend/src/models/assistant_settings.rs: COLLECTION_NAME=assistant_settings | user_id
backend/src/models/assistant_skill_draft_attempt.rs: COLLECTION_NAME=assistant_skill_draft_attempts | user_id
backend/src/models/assistant_voice.rs: REQUESTS=assistant_voice_requests | user_id
backend/src/models/assistant_voice_session.rs: COLLECTION_NAME=assistant_voice_sessions | user_id
backend/src/models/async_service_operation.rs: COLLECTION_NAME=assistant_async_operations, QUOTAS_COLLECTION_NAME=assistant_async_quotas | user_id
backend/src/models/audit_log.rs: COLLECTION_NAME=audit_log | user_id
backend/src/models/auth_device_code.rs: COLLECTION_NAME=auth_device_codes, V2_COLLECTION_NAME=auth_device_codes_v2, RESERVATION_COLLECTION_NAME=auth_device_code_reservations | approved_user_id, denied_by_user_id
backend/src/models/authorization_code.rs: COLLECTION_NAME=authorization_codes | external_user_id, user_id
backend/src/models/billing_lago_carry.rs: COLLECTION_NAME=billing_lago_carry | owner_id
backend/src/models/billing_ledger.rs: COLLECTION_NAME=billing_ledger | owner_id
backend/src/models/billing_topup_session.rs: COLLECTION_NAME=billing_topup_sessions | owner_id
backend/src/models/billing_wallet.rs: COLLECTION_NAME=billing_wallet | owner_id
backend/src/models/catalog_delegation_grant.rs: COLLECTION_NAME=catalog_delegation_grants | user_id
backend/src/models/channel_adapter.rs: BINDINGS=channel_adapter_bindings, SESSIONS=channel_adapter_sessions, EVENTS=channel_adapter_events, CLAIMS=channel_adapter_resource_claims | user_id
backend/src/models/channel_bot.rs: COLLECTION_NAME=channel_bots | user_id
backend/src/models/channel_connect_link.rs: COLLECTION_NAME=channel_connect_links | created_by_user_id, user_id
backend/src/models/channel_conversation.rs: COLLECTION_NAME=channel_conversations | user_id
backend/src/models/channel_delivery.rs: COLLECTION_NAME=channel_delivery_receipts | recipient_id, user_id
backend/src/models/channel_message.rs: COLLECTION_NAME=channel_messages | user_id
backend/src/models/channel_reply_delivery.rs: COLLECTION_NAME=channel_reply_deliveries | user_id
backend/src/models/channel_text_delivery.rs: COLLECTION_NAME=channel_text_deliveries | user_id
backend/src/models/cli_pairing.rs: COLLECTION_NAME=cli_pairings | user_id
backend/src/models/connect_link.rs: COLLECTION_NAME=connect_links | user_id
backend/src/models/consent.rs: COLLECTION_NAME=consents | user_id
backend/src/models/credit_grant.rs: COLLECTION_NAME=credit_grants | recipient_user_id
backend/src/models/credit_schedule.rs: COLLECTION_NAME=credit_schedules | created_by
backend/src/models/credit_schedule_period.rs: COLLECTION_NAME=credit_schedule_periods | cursor_user_id
backend/src/models/device_code.rs: COLLECTION_NAME=device_codes | approved_by_user_id
backend/src/models/device_onboard_credential.rs: COLLECTION_NAME=device_onboard_credentials | owner_user_id
backend/src/models/downstream_service.rs: COLLECTION_NAME=downstream_services | created_by, identity_include_email, identity_include_user_id, owner_user_id
backend/src/models/durable_operation_execution.rs: COLLECTION_NAME=durable_operation_executions | user_id
backend/src/models/durable_operation_grant.rs: COLLECTION_NAME=durable_operation_grants | created_by, user_id
backend/src/models/feature_flag_definition.rs: COLLECTION_NAME=feature_flag_definitions | created_by
backend/src/models/feature_flag_override.rs: COLLECTION_NAME=feature_flag_overrides | org_user_id
backend/src/models/invite_code.rs: COLLECTION_NAME=invite_codes | created_by, email, user_id
backend/src/models/lark_base_write.rs: COLLECTION_NAME=lark_base_writes | user_id
backend/src/models/lark_base_write_photo.rs: COLLECTION_NAME=lark_base_write_photos | user_id
backend/src/models/login_approval.rs: COLLECTION_NAME=login_approvals | user_id
backend/src/models/login_code.rs: COLLECTION_NAME=login_codes | user_id
backend/src/models/machine_desktop.rs: COLLECTION_NAME=machine_desktops | user_id
backend/src/models/machine_external_grant.rs: COLLECTION_NAME=machine_external_grants | user_id
backend/src/models/machine_free_slot.rs: COLLECTION_NAME=machine_free_slots | owner_id
backend/src/models/machine_job.rs: COLLECTION_NAME=machine_jobs | user_id
backend/src/models/machine_oauth_consent_request.rs: COLLECTION_NAME=machine_oauth_consent_requests | user_id
backend/src/models/machine_runtime_billing.rs: RATE_COLLECTION=machine_runtime_rates, QUOTE_COLLECTION=machine_runtime_quotes, RUN_COLLECTION=machine_runtime_runs, INTERVAL_COLLECTION=machine_runtime_intervals, HEAD_COLLECTION=machine_runtime_heads | owner_id
backend/src/models/machine_setup.rs: COLLECTION_NAME=machine_setups | owner_id, user_id
backend/src/models/machine_update.rs: ATTEMPTS_COLLECTION_NAME=machine_update_attempts, COLLECTION_NAME=machine_updates | user_id
backend/src/models/managed_execution.rs: COLLECTION_NAME=managed_executions | actor_user_id, billing_owner_id
backend/src/models/mcp_session.rs: MCP_SESSION_COLLECTION=mcp_sessions, MCP_SESSION_SLOT_COLLECTION=mcp_session_slots | user_id
backend/src/models/mfa_factor.rs: COLLECTION_NAME=mfa_factors | user_id
backend/src/models/node.rs: COLLECTION_NAME=nodes | user_id
backend/src/models/node_pending_credential.rs: COLLECTION_NAME=node_pending_credentials | created_by_user_id, owner_user_id
backend/src/models/node_registration_token.rs: COLLECTION_NAME=node_registration_tokens | user_id
backend/src/models/node_service_binding.rs: COLLECTION_NAME=node_service_bindings | user_id
backend/src/models/notification_channel.rs: COLLECTION_NAME=notification_channels | user_id
backend/src/models/nyxbot_channel.rs: COLLECTION_NAME=nyxbot_channels, THREADS_COLLECTION_NAME=nyxbot_threads, EVENTS_COLLECTION_NAME=nyxbot_events, WATCHES_COLLECTION_NAME=nyxbot_watches | bot_owner_id, user_id
backend/src/models/nyxbot_image_delivery.rs: COLLECTION_NAME=nyxbot_image_deliveries_v2, LEGACY_COLLECTION_NAME=nyxbot_image_deliveries | user_id
backend/src/models/oauth_broker_binding.rs: COLLECTION_NAME=oauth_broker_bindings | user_id
backend/src/models/oauth_client.rs: COLLECTION_NAME=oauth_clients | created_by
backend/src/models/oauth_consent_request.rs: COLLECTION_NAME=oauth_consent_requests | user_id
backend/src/models/oauth_state.rs: COLLECTION_NAME=oauth_states | credential_user_id, target_user_id, user_id
backend/src/models/oracle_login_snapshot.rs: COLLECTION_NAME=oracle_login_snapshots | created_by_user_id
backend/src/models/oracle_pool.rs: COLLECTION_NAME=oracle_pools | user_id
backend/src/models/oracle_session.rs: COLLECTION_NAME=oracle_sessions | owner_user_id
backend/src/models/oracle_task.rs: COLLECTION_NAME=oracle_tasks | submitter_user_id
backend/src/models/oracle_worker.rs: COLLECTION_NAME=oracle_workers | owner_user_id
backend/src/models/oracle_worker_command.rs: COLLECTION_NAME=oracle_worker_commands | created_by_user_id
backend/src/models/org_invite.rs: COLLECTION_NAME=org_invites | created_by, org_user_id
backend/src/models/org_membership.rs: COLLECTION_NAME=org_memberships | member_user_id, org_user_id
backend/src/models/org_role_scope.rs: COLLECTION_NAME=org_role_scopes | org_user_id
backend/src/models/ownership_transfer.rs: COLLECTION_NAME=ownership_transfers | actor_user_id, new_owner_user_id, previous_owner_user_id
backend/src/models/permission_policy.rs: COLLECTION_NAME=permission_key_policies | user_id
backend/src/models/platform_binding.rs: COLLECTION_NAME=platform_onboarding_bindings, REQUESTS=platform_onboarding_binding_requests | approved_user_id, user_id
backend/src/models/platform_bot_config.rs: COLLECTION_NAME=platform_bot_configs, CLEANUP_COLLECTION=platform_bot_cleanup | funding_owner_user_id, owner_user_id
backend/src/models/platform_bot_runtime_controls.rs: COLLECTION_NAME=platform_bot_runtime_controls | sponsor_owner_user_id
backend/src/models/platform_entry_gateway.rs: COLLECTION_NAME=platform_entry_gateway_transports | credential_owner_id
backend/src/models/platform_installation.rs: COLLECTION_NAME=platform_bot_installations | owner_user_id
backend/src/models/platform_link_intent.rs: COLLECTION_NAME=platform_personal_link_intents | claimant_user_id, reviewed_user_id, user_id
backend/src/models/platform_manager_ingress.rs: COLLECTION_NAME=platform_manager_ingress_receipts | payer_user_id
backend/src/models/platform_personal_binding.rs: COLLECTION_NAME=platform_personal_bindings | user_id
backend/src/models/platform_public_session.rs: COLLECTION_NAME=platform_public_sessions | sponsor_owner_user_id
backend/src/models/platform_public_turn.rs: COLLECTION_NAME=platform_public_turns | sponsor_owner_user_id
backend/src/models/platform_setup.rs: COLLECTION_NAME=platform_setups | initiating_actor_id, owner_user_id
backend/src/models/provider_config.rs: COLLECTION_NAME=provider_configs | created_by
backend/src/models/refresh_token.rs: COLLECTION_NAME=refresh_tokens | user_id
backend/src/models/remote_machine.rs: COLLECTION_NAME=managed_machines | owner_id
backend/src/models/remote_machine_event.rs: COLLECTION_NAME=machine_lifecycle_events | owner_id
backend/src/models/remote_machine_operation.rs: COLLECTION_NAME=machine_operations | owner_id
backend/src/models/remote_machine_outbox.rs: COLLECTION_NAME=machine_effect_outbox | owner_id
backend/src/models/saved_login.rs: COLLECTION_NAME=saved_logins | user_id
backend/src/models/service_account.rs: COLLECTION_NAME=service_accounts | created_by, owner_user_id
backend/src/models/service_account_key_read_grant.rs: COLLECTION_NAME=service_account_key_read_grants | owner_id
backend/src/models/service_approval_config.rs: COLLECTION_NAME=service_approval_configs | user_id
backend/src/models/service_change_event.rs: COLLECTION_NAME=service_change_events, HEADS_COLLECTION_NAME=service_history_heads | owner_id
backend/src/models/service_pool.rs: COLLECTION_NAME=service_pools | user_id
backend/src/models/service_pool_member_health.rs: COLLECTION_NAME=service_pool_member_health, OBSERVATION_COLLECTION_NAME=service_pool_health_observations | owner_id
backend/src/models/service_preference.rs: COLLECTION_NAME=service_preferences | user_id
backend/src/models/session.rs: COLLECTION_NAME=sessions | user_id
backend/src/models/telegram_bot_claim.rs: COLLECTION_NAME=telegram_bot_claims | actor_user_id, owner_user_id, telegram_user_id
backend/src/models/telegram_bot_request.rs: COLLECTION_NAME=telegram_bot_requests | actor_user_id, created_by, owner_user_id, telegram_user_id
backend/src/models/telemetry_erasure_job.rs: COLLECTION_NAME=telemetry_erasure_jobs | user_id
backend/src/models/trigger.rs: COLLECTION_NAME=triggers | user_id
backend/src/models/trigger_delivery.rs: COLLECTION_NAME=trigger_deliveries | user_id
backend/src/models/trigger_run.rs: COLLECTION_NAME=trigger_runs | user_id
backend/src/models/usage_allowance.rs: COLLECTION_NAME=usage_allowances | created_by
backend/src/models/usage_allowance_period.rs: COLLECTION_NAME=usage_allowance_periods | owner_user_id
backend/src/models/usage_meter.rs: COLLECTION_NAME=usage_meter, POOL_RECOVERY_COLLECTION_NAME=billing_pool_recovery | actor_user_id, billing_owner_id, member_id
backend/src/models/usage_workspace.rs: COLLECTION_NAME=usage_workspaces | user_id
backend/src/models/user.rs: COLLECTION_NAME=users | email
backend/src/models/user_api_key.rs: COLLECTION_NAME=user_api_keys | user_id
backend/src/models/user_endpoint.rs: COLLECTION_NAME=user_endpoints | user_id
backend/src/models/user_provider_credentials.rs: COLLECTION_NAME=user_provider_credentials | user_id
backend/src/models/user_provider_token.rs: COLLECTION_NAME=user_provider_tokens | credential_user_id, user_id
backend/src/models/user_service.rs: COLLECTION_NAME=user_services | created_by, identity_include_email, identity_include_user_id, user_id
backend/src/models/user_service_connection.rs: COLLECTION_NAME=user_service_connections | user_id
```
