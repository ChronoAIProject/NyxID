# Personal-account erasure inventory and acceptance contract

This runbook describes personal-account erasure acceptance, durable cleanup and
provider revocation. Acceptance fences account authority; completion waits for
the recorded local and remote obligations. Queuing vendor erasure does not
establish completion, and a scan for `user_id` alone cannot establish ownership.
The registry and inventory drift tests described below keep the cleanup map
reviewable as collections change.

Personal erasure prunes the erased person from each organization specialist’s user and maintainer lists. Each affected capability revision advances once. Organization specialists remain with the organization.

## Owner-index rollout

These startup additions never drop, rename, or rebuild an existing index. They
reuse any existing index whose first key is the erasure owner predicate and
add only the audited gaps below. The names are stable, so a rolling deploy
can run old and new replicas against the same database. These indexes support
bounded owner scans for account erasure and the pre-job orphan sweep; they do
not change retention or ownership decisions. The manifest adds 17 indexes, comprising
14 stream indexes, the group-message author index used for attribution
redaction, and the two specialist access-list indexes. The reply delivery index on `{user_id:1, inbound_message_id:1}` already covers owner erasure.

Each row below is a real missing prefix on a collection that grows with
events, messages, requests, or deliveries. Without it, the owner query would
scan the whole collection before the page limit could be applied. Small or
resource-bounded collections, short-lived rows, and joined children bounded by
their parent are explicitly classified in the registry tests and receive no
new index. `channel_send_claims` is also covered by its composite `_id` prefix.

`db::create_connection` awaits the base `db::ensure_indexes` call, and `main`
awaits `personal_erasure_service::ensure_startup_indexes`. That awaits
`ensure_indexes` → `ensure_owner_index_gaps` → `create_index(...).await`
before either listener binds. Therefore every row below is an
awaited startup build: on a large existing collection MongoDB may hold startup
until the one-time index build completes. This follows the existing startup
precedent for large collections, where `ensure_indexes` awaits feature-owned
index creation in the `assistant_wire_logs`, `refresh_tokens`,
`trigger_deliveries`, `mcp_session_notifications`, `invocation_diagnostics`,
and `service_pool_health_observations` blocks of `db::ensure_indexes`. No deferred
or background build mechanism is introduced. A concrete additive precedent
on a large event collection is the awaited `usage_rollup_actor_window` index on
`usage_meter` in `billing::usage_rollup::ensure_indexes`, reached through
`db::ensure_indexes`. The same path awaits v6 covering indexes
for hourly/daily billing summaries in `billing::usage_rollup::ensure_indexes`
before serving. This change follows those synchronous additions. A first build
scans the existing data and consumes database CPU/I/O and index storage while
other replicas continue serving; compatible existing prefixes skip creation.
No production build duration is assumed here.

For large deployments, pre-build the following indexes before deploying. Run
these against the application database using the deployment's normal MongoDB
administration access. Startup awaits builds before listening; identical
pre-built indexes make the creation calls no-ops. A local measurement on one
million rows took approximately 1.0 seconds per index; production time depends
on collection size, hardware and concurrent load. The MCP notification join
uses the existing `session_id` prefix index; no `user_id` index is needed because
that collection has no such field. An already deployed extra index is left
untouched.

```javascript
db.assistant_agents.createIndex({"org_maintainer_user_ids": 1}, {name: "personal_erasure_owner_assistant_agents_org_maintainer_user_ids"});
db.assistant_agents.createIndex({"org_user_ids": 1}, {name: "personal_erasure_owner_assistant_agents_org_user_ids"});
db.assistant_group_mention_notifications.createIndex({"user_id": 1}, {name: "personal_erasure_owner_assistant_group_mention_notifications_user_id"});
db.assistant_group_messages.createIndex({"author_user_id": 1}, {name: "personal_erasure_owner_assistant_group_messages_author_user_id"});
db.assistant_group_messages.createIndex({"user_id": 1}, {name: "personal_erasure_owner_assistant_group_messages_user_id"});
db.assistant_group_requests.createIndex({"user_id": 1}, {name: "personal_erasure_owner_assistant_group_requests_user_id"});
db.channel_adapter_events.createIndex({"user_id": 1}, {name: "personal_erasure_owner_channel_adapter_events_user_id"});
db.channel_delivery_receipts.createIndex({"user_id": 1}, {name: "personal_erasure_owner_channel_delivery_receipts_user_id"});
db.channel_email_batches.createIndex({"user_id": 1}, {name: "personal_erasure_owner_channel_email_batches_user_id"});
db.channel_email_receipts.createIndex({"user_id": 1}, {name: "personal_erasure_owner_channel_email_receipts_user_id"});
db.channel_email_sends.createIndex({"user_id": 1}, {name: "personal_erasure_owner_channel_email_sends_user_id"});
db.nyxbot_events.createIndex({"user_id": 1}, {name: "personal_erasure_owner_nyxbot_events_user_id"});
db.refresh_tokens.createIndex({"user_id": 1}, {name: "personal_erasure_owner_refresh_tokens_user_id"});
db.service_approval_configs.createIndex({"approver_user_ids": 1}, {name: "personal_erasure_owner_service_approval_configs_approver_user_ids"});
db.service_change_events.createIndex({"actor.person_id": 1}, {name: "personal_erasure_owner_service_change_events_actor_person_id"});
db.service_pool_health_observations.createIndex({"pool_id": 1}, {name: "personal_erasure_owner_service_pool_health_observations_pool_id"});
db.trigger_deliveries.createIndex({"user_id": 1}, {name: "personal_erasure_owner_trigger_deliveries_user_id"});
db.trigger_work.createIndex({"user_id": 1}, {name: "personal_erasure_owner_trigger_work_user_id"});
```

The checked-in origin/main startup fixture in the registry tests is generated
from `db::ensure_indexes` plus origin/main's security-readiness and personal
erasure index calls. The upgrade tests run that fixture first, run the branch
sequence twice, and then replay the fixture to prove mixed-fleet compatibility.
Paths in the tables below are relative to `backend/src`.

`node_credential_desired` is removed by the node drain and by
`personal_erasure_registry::final_metadata` using `owner_id`; organization-owned
rows retain only an opaque actor ID.

| Collection | Key | New index name | Why this gap is kept | Startup/build behavior |
| --- | --- | --- | --- | --- |
| `assistant_group_messages` | `user_id` | `personal_erasure_owner_assistant_group_messages_user_id` | Per-message group inserts (`collaboration_group_service::requests::append`, `collaboration_group_service::runtime::publish_reply`), with no message TTL; local cleanup needs `user_id`. | Awaited before listeners; a large build can delay startup, following `billing::usage_rollup::ensure_indexes`. |
| `assistant_group_messages` | `author_user_id` | `personal_erasure_owner_assistant_group_messages_author_user_id` | Participant posts and addressed requests snapshot the human author's display name (`collaboration_group_service::participant_posts::try_post`, `collaboration_group_service::requests::enqueue_with_uploads_ready`); no message TTL bounds this stream. Shared-message redaction and its final-source/orphan checks otherwise scan the whole collection. | Awaited before listeners; a large build can delay startup, following `billing::usage_rollup::ensure_indexes`. |
| `assistant_group_requests` | `user_id` | `personal_erasure_owner_assistant_group_requests_user_id` | Per-addressed-group-message inserts (`collaboration_group_service::requests::enqueue_with_uploads_ready`); no TTL or completed-history cap, and local cleanup needs `user_id`. | Awaited before listeners; a large build can delay startup, following `billing::usage_rollup::ensure_indexes`. |
| `nyxbot_events` | `user_id` | `personal_erasure_owner_nyxbot_events_user_id` | Per-inbound-event inserts (`handlers::nyxbot::custom_responses`), retained for 24 hours (seven days with media, `handlers::nyxbot::responses_with_producers_impl`) at message volume; local cleanup needs `user_id`. | Awaited before listeners; a large build can delay startup, following `billing::usage_rollup::ensure_indexes`. |
| `trigger_deliveries` | `user_id` | `personal_erasure_owner_trigger_deliveries_user_id` | Per-delivery inserts (`trigger_service::admit_webhook_delivery`); configured TTL defaults to 72 hours, leaving a high-volume stream for local cleanup. | Awaited before listeners; a large build can delay startup, following `db::ensure_indexes`, `trigger_deliveries` block. |
| `trigger_work` | `user_id` | `personal_erasure_owner_trigger_work_user_id` | Per-run and per-notice work inserts (`trigger_schedule::insert_run` and `trigger_schedule::record_terminal_in_session`), plus schedule rows; no TTL bounds stalled work before owner cleanup. | Awaited before listeners; a large build can delay startup, following `billing::usage_rollup::ensure_indexes`. |
| `channel_adapter_events` | `user_id` | `personal_erasure_owner_channel_adapter_events_user_id` | Per-call-turn/SMS event inserts (`call_channels::session::insert_sms_send` and `call_channels::session::admit`); event receipts have no TTL, and settled rows need owner cleanup. | Awaited before listeners; a large build can delay startup, following `billing::usage_rollup::ensure_indexes`. |
| `channel_delivery_receipts` | `user_id` | `personal_erasure_owner_channel_delivery_receipts_user_id` | Per-message receipt upserts (`channel_delivery_service::observe`), with 30-day TTL; local cleanup needs `user_id`. | Awaited before listeners; a large build can delay startup, following `billing::usage_rollup::ensure_indexes`. |
| `channel_email_receipts` | `user_id` | `personal_erasure_owner_channel_email_receipts_user_id` | Per-inbound-email receipt writes (`channel_adapters::aurinko_lifecycle::AurinkoAdapter::receive_batch`), with no TTL; local cleanup needs `user_id`. | Awaited before listeners; a large build can delay startup, following `billing::usage_rollup::ensure_indexes`. |
| `channel_email_sends` | `user_id` | `personal_erasure_owner_channel_email_sends_user_id` | Per-email-send inserts (`channel_adapters::aurinko_lifecycle::AurinkoAdapter::reply_once`), with no TTL; local cleanup needs `user_id`. | Awaited before listeners; a large build can delay startup, following `billing::usage_rollup::ensure_indexes`. |
| `channel_email_batches` | `user_id` | `personal_erasure_owner_channel_email_batches_user_id` | Per-webhook-batch upserts (`channel_adapters::aurinko_lifecycle::AurinkoAdapter::receive_batch`), with 31-day TTL; local cleanup needs `user_id`. | Awaited before listeners; a large build can delay startup, following `billing::usage_rollup::ensure_indexes`. |
| `refresh_tokens` | `user_id` | `personal_erasure_owner_refresh_tokens_user_id` | Per-rotation inserts (`token_service::refresh_tokens`), with configurable expiry (default seven days); local cleanup needs `user_id`. | Awaited before listeners; a large build can delay startup, following `db::ensure_indexes`, `refresh_tokens` block. |
| `service_change_events` | `actor.person_id` | `personal_erasure_owner_service_change_events_actor_person_id` | Per-service-change journal (`models/service_change_event.rs`, `services/service_history/transaction.rs`), with no TTL; shared-history redaction filters by actor person ID. | Awaited before listeners; a large build can delay startup, following `billing::usage_rollup::ensure_indexes`. |
| `service_pool_health_observations` | `pool_id` | `personal_erasure_owner_service_pool_health_observations_pool_id` | Per-request observation stream (`service_pool_health_service::issue_observation_ticket`); a one-hour TTL still admits all live request volume, and joined-child cleanup needs `pool_id`. | Awaited before listeners; a large build can delay startup, following `db::ensure_indexes`, `service_pool_health_observations` block. |
| `assistant_group_mention_notifications` | `user_id` | `personal_erasure_owner_assistant_group_mention_notifications_user_id` | One insert per mentioned participant (`collaboration_group_service::notifications::enqueue`), with seven-day TTL (`collaboration_group_service::ensure_indexes`); local cleanup needs `user_id`. | Awaited before listeners; a large build can delay startup, following `billing::usage_rollup::ensure_indexes`. |

Two diagnostic predicates receive no additional index:
`assistant_wire_logs.user_id` is opt-in diagnostic capture (request header plus
a default-off flag, `handlers::assistant::upstream_echo_collector`) with a 15-minute
TTL (`assistant_wire_log_service::WIRE_LOG_TTL_SECS` and `assistant_wire_log_service::store`).
`invocation_diagnostics.actor_user_id` is default-off, access-token failed-request
handle issuance (`AppConfig::from_env`,
`handlers::invocation_diagnostics::HttpInvocation::finish`), with a 60-second TTL
and 10-second duplicate-issuance throttle
(`invocation_diagnostic_service::HANDLE_TTL_SECS`, `DUPLICATE_ISSUE_WINDOW` and `issue`). These
are short-lived diagnostic flows, so bounded cleanup does not justify a new
startup build and ongoing write cost.

`assistant_group_requests.user_id` needs an additional index. The writer
inserts a row for each addressed group message
(`collaboration_group_service::requests::enqueue_with_uploads_ready`), the
model has no expiry (`assistant_group::GroupRequest`), and
admission removes pending agents without deleting the request
(`collaboration_group_service::requests::admit_in_session`).
The 16-request admission cap applies only to pending requests
(`collaboration_group_service::requests::enqueue_with_uploads_ready`); it does
not bound completed history. The collection therefore grows with group
activity, and its owner erasure/source query otherwise scans the stream.
The checked classifications for other excluded predicates distinguish small/resource lifecycles, short TTLs,
parent-bounded children, and the existing claim `_id` prefix. No existing
deployed index is removed.

The excluded predicates have the following lifecycle bounds (paths below are
relative to `backend/src`; the exact field list is `OWNER_INDEX_BOUNDED` in the
registry):

| Collections without additional indexes | Writer/lifecycle evidence for bounded cleanup |
| --- | --- |
| `agent_key_login_requests`, `auth_device_codes`, `auth_device_codes_v2`, `authorization_codes`, `device_codes`, `login_approvals`, `mfa_challenges`, `oauth_consent_requests` | Login/consent protocols write expiring rows, rather than a retained event stream (`auth_agent_key_login_service::REQUEST_TTL_SECS` and `auth_agent_key_login_service::request`, `token_service::insert_mfa_pending_session_with_method`, and the corresponding expiry indexes in `db.rs`). |
| `assistant_agent_sharing`, `assistant_agent_shares`, `assistant_agent_private_notes`, `assistant_agent_consents` | Sharing policy, recipient grant, private note and connection consent state belongs to agent/recipient/connection lifecycles (`models/assistant_sharing.rs`, `services/shared_agent_service.rs`); this is resource state rather than the message stream. |
| `assistant_agent_learning_members`, `assistant_agent_learning_runs`, `assistant_agent_learning_skill_roots`, `assistant_learning_migrations` | Opt-in membership and explicit learning/publication work (`assistant_agent_learning::set_member_opt_in`, `create_run` and `reserve_run`); bounded feature lifecycle rather than every assistant message. |
| `assistant_async_quotas` | One quota counter per owner (`async_service_operation::reserve_with_artifacts`); replaced resource state rather than an append-only history. |
| `assistant_group_agent_access`, `assistant_group_agent_proposals`, `assistant_group_invites`, `assistant_group_read_states` | Group access/invite lifecycle and one read-state per participant (`services/collaboration_group_service/membership.rs`, `collaboration_group_service::notifications::ensure_state`); invites/proposals also expire (`collaboration_group_service::ensure_indexes`). |
| `assistant_skill_draft_attempts`, `assistant_voice_windows` | Explicit draft attempts expire after a day (`db::ensure_indexes`, `assistant_skill_draft_attempts` expiry index). Voice windows are bounded by a voice session (`voice::runtime::checkpoint_windows` and `voice::runtime::settle_windows`). |
| `channel_adapter_accounts`, `channel_adapter_allocations`, `channel_email_subscriptions`, `openclaw_channel_mappings` | Adapter account/resource setup and one subscription per bot (`channel_email::EmailSubscription`, `services/call_channels/binding.rs`, `services/channel_adapters/openclaw.rs`); no event-per-message history. |
| `channel_meta_provider_targets` | Independent retry/proof children of a signed request, keyed only by request/bot/grant UUIDs and optional scope UUID. Pending children have no TTL. After completion, indexed pages apply the parent's 90-day proof deadline before parent TTL is enabled. Excluded from export; no subject, owner list or copied source/content. |
| `channel_meta_grant_scopes` | One bot/grant UUID pair with keyed subject/binding fingerprints, platform/App ID, generation range, timestamps and independent cleanup proof. Retained erasure bookkeeping, excluded from export; no raw subject, owner list, credentials, participant IDs or copied content. Exact completion releases the app-drain obligation and removes identity, App ID, generation and attribution fields; opaque exact replay proof has no terminal TTL. Subjectless manual retirement removes its unused scope. |
| `channel_meta_oauth_sessions` | Consent sessions expire through the `expires_at` TTL (`services/channel_meta_service.rs::ensure_indexes`). Account erasure removes either `owner_id` or `actor_id`; encrypted candidate credentials are excluded from export. |
| `encrypted_record_migration_findings`, `encrypted_record_migration_runs` | Migration-generation diagnostics/progress (`encrypted_record_migration::advance`), not hot request writes. |
| `feature_flag_overrides`, `invite_codes`, `org_invites` | Administrative targeting and invite lifecycles; embedded invite usage is bounded by the authored invite's usage policy (`models/invite_code.rs`, `services/feature_flag_service.rs`, `services/org_invite_service.rs`). |
| `user_services.created_by.actor.person_id`, `user_services.last_change.actor.person_id` | Only the creator and latest-change summary on each service resource are replaced (`service_history::projection::record_changes` and `service_history::projection::record_service`); historical events use the separately indexed journal. No per-event growth on these resource rows. |
| `oauth_states.history_context.actor.person_id` | A saved initiation context per expiring OAuth flow (`oauth_state::OAuthState`, `db.rs`'s `oauth_states` expiry index); no retained actor-history stream. |
| `platform_guide_selections`, `platform_native_replies`, `platform_onboarding_binding_requests`, `platform_personal_decision_spacing`, `platform_personal_setup_selections` | Guide/setup/binding selection, reply and spacing coordination (`services/platform_guide_service.rs`, `services/platform_native_reply_service.rs`, `services/platform_personal_owner_lifecycle.rs`); low-volume setup flows. |
| `platform_delivery_tests`, `platform_setup_steps` | Joined installation/setup lifecycle children (`platform_installation::STEPS`, `platform_installation::TESTS` and `platform_installation::DeliveryTest`); bounded steps/tests per parent. |
| `service_account_key_read_grants`, `service_pool_member_health` | Owner grant configuration (`models/service_account_key_read_grant.rs`) and one current health row per member/context, with one-hour expiry (`service_pool_health_service::HEALTH_RETENTION_MS` and `service_pool_health_service::issue_observation_ticket`); observations remain separately indexed. |
| `telegram_bot_claims`, `telegram_bot_requests`, `telegram_managed_bot_events` | Bot provisioning/claim lifecycle; request admission lasts 15 minutes and terminal request purge follows lifecycle checks (`telegram_new_service::TelegramNewService::expire` and `telegram_new_service::TelegramNewService::begin_inner`), while managed events track bot creation. |
| `channel_send_claims` | The model has no `conversation_id` field (`channel_send_claim::ChannelSendClaim`); cleanup uses an anchored composite `_id` regex (`services/personal_erasure_service/local.rs`, `personal_erasure_orphan_sweep::jobs`), served by `_id_`. |

## Pre-job orphan sweep

Rows left by account deletions that predate durable erasure jobs can be reviewed
by an administrator with `POST /api/v1/admin/personal-erasure/orphans`. The
request defaults to a dry-run and returns HTTP 202 with a job ID. Poll
`GET /api/v1/admin/personal-erasure/orphans/{job_id}` for its persisted report:
counts, a capped list of orphan identifiers, and a SHA-256 digest binding every
candidate's collection, owner field, owner ID and exact row ID. Applying starts
another job with that digest; no mutation occurs if the newly scanned set differs.

The worker reuses `personal_erasure_service::worker`'s server-time claim,
lease-token fencing and majority checkpoint protocol. Each pass scans at most
512 rows/owner values with an `_id` keyset cursor, including an intra-row cursor
for arrays. Liveness queries contain at most 128 IDs. Candidate pages contain
at most 64 identifier pairs and are persisted with the scan checkpoint in one
transaction; document bodies are never copied. Report construction streams pages
and renews the same lease. Applying resumes the reviewed pages after a restart.
A changed registry specification invalidates an unfinished scan. Jobs and pages
are retained as erasure bookkeeping; their low-volume claim scan uses existing
indexes and adds no startup index build.

Only local-inventory, redaction and joined-child classifications are considered.
An existing person, organization-user, service account or erasure job is live.
The report's per-collection owner list is capped at 128, with exact candidate
counts and an explicit cap flag. Deletes and redactions are repeatable, and the
completed set is recorded in the chained audit log as metadata.

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

### Upgrade compatibility and repair cadence

An in-flight job whose `local_inventory` checkpoint was completed by an older
binary re-runs that domain when `local_inventory_revision` is below 5, before
continuing to source cleanup. This binary stamps the current revision when its
local-inventory checkpoint completes; its newly completed jobs need no repair
scan.

Each worker scans for old completed jobs immediately on process start. Finding
a candidate keeps the one-second scan cadence; finding none pauses only this
repair scan for ten minutes. An old binary completing a job during that quiet
window can therefore wait about ten minutes, plus worker scheduling and any
backlog, before repair starts. Ordinary pending jobs continue on their normal
cadence throughout the quiet window. No additional index is created.

Completed-job repair checks remote cleanup before running inventory pages.
A blocked job releases its lease with a MongoDB-server-time `retry_at` five
minutes later, without stamping the inventory revision. One bounded Telegram
cancellation pass still retires unstarted requests or requests with confirmed
bot cleanup. A persisted `reconciliation_remote_blocked` flag emits one warning
containing only the job id per blocked episode, including across replicas and
restarts; observing a clear gate removes the flag. Repeated blocked retries
retain the flag and do not repeat that warning.

Local-inventory and source deletion wait for platform bot cleanup, Telegram
creation recovery, and personal Stop effects needing active, person-owned
ordinary nodes. A continuous wait on an in-flight job reaches `operator_action`
after 24 hours with reason `remote_cleanup_wait_exceeded`; the account row
remains until an operator resolves the dependency and retries. Status dashboards
can also show `remote_cleanup_pending` or `remote_cleanup_verification_required`.
An old replica in a mixed fleet rewrites the domain array without
`remote_wait_started_at`; that can restart and delay the 24-hour escalation
clock. It does not bypass the cleanup gate.

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

Add `?include_retained=true` to the account deletion request or the admin
erasure progress request to include `retained` on the job response. Each item
contains a registry collection name and its retention reason. The list
describes policy and does not indicate that a particular account had rows in
each collection. The admin console requests this list and displays it
alongside the job's progress.

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

## Shared-attribution audit and redaction

The persisted model and writer audit found the following human identity
snapshots. The registry lists the opaque person selector separately from resource
ownership; an attribution match never grants export access or permission to
delete another owner's resource.

| Collection | Persisted snapshot and authoritative person selector | Cleanup |
| --- | --- | --- |
| `service_change_events` | `HistoryActor` at `actor`, keyed by `actor.person_id` | Delete the person's own journal; on surviving shared events replace existing actor names with `Deleted account` and remove email snapshots. |
| `user_services` | `ServiceChangeSummary` at `created_by` and `last_change`, each wrapping `actor`, keyed by `created_by.actor.person_id` / `last_change.actor.person_id` | Delete personally owned resources; redact the matching creator/updater on other owners' resources, retaining every opaque actor/person/API-key/app id. Endpoint and API-key changes write their summaries here (`service_history::projection::record_changes` and `service_history::projection::record_service`); those two model collections have no separate actor-summary field. |
| `oauth_states` | `HistoryContext` at `history_context`, keyed by `history_context.actor.person_id` | Delete the person's own flows; redact saved initiation labels on another owner's surviving flow. |
| `assistant_group_messages` | `author_display_name`, keyed by `author_user_id` | Existing lifecycle cleanup may remove the author's messages; replace labels on any surviving shared message without changing its text, resource owner or opaque author id. |
| `invite_codes` | `usages[].email`, keyed by that element's `user_id` | Remove only that usage's email, retaining opaque ids, other usages, limits and usage counters. |
| `assistant_agent_shares` | `subject_email`, keyed by `subject_id` with `subject_kind = person` | Existing local inventory and orphan sweep remove the deleted person's recipient grant, including a grant on another owner's agent; other grants and the shared agent remain. |

The model scan covers `HistoryActor`, `HistoryContext` and
`ServiceChangeSummary`; their only persisted containers are the four paths in
the first three rows. `HistoryActor.id` can be an API-key id, so cleanup uses
`person_id`, never an assumed actor-id/person-id equivalence. Current writers
store `name`; legacy `display_name`, `email` and `email_address` aliases at those
same actor paths are also covered. Missing or null name fields remain so.

The other retained/shared attribution models carry opaque ids only:
`downstream_services` (creator/owner), `catalog_skill_revisions` and
`catalog_skill_operations` (actor kind/id), `ownership_transfers` (actor and
previous/new owner), `assistant_group_transfers` (from/to user), and platform
publication/configuration rows (publisher/creator ids). Their resource, agent,
skill and bot names describe the resource, not a human identity snapshot.
`channel_messages.sender_display_name` comes from a platform sender id, not a
NyxID person id; it does not authorize person cleanup of another owner's
transcript. Financial contact names are retained financial data, not authorship.
`audit_log` and `billing_ledger` remain immutable and are never redaction targets.

One shared redaction implementation serves the worker (32-row pages) and the
orphan sweep (64-row pages). It matches only names not already replaced or email
fields still present, so completed rows cannot starve later pages. Each page
uses a majority transaction and the admission barrier; the orphan sweep also
rechecks person/org/service-account liveness and installs the durable tombstone
for a pre-job deletion. Local-inventory revision 3 reconciles previously
completed jobs under their surviving tombstone; neither the domain set nor
`INVENTORY_VERSION` changes. The final source check refuses completion while an
unredacted snapshot remains.

Global-scan tests seed the target's name/email in every listed snapshot, cross
both page sizes, verify mixed creator/updater and live-actor bystanders exactly,
and scan every collection after completion/apply. Only immutable `audit_log` may
contain the sentinel name/email; both immutable-chain fixtures remain identical.

## Coverage and recovery checks

The personal-data registry covers production collection constants and literal collection calls. Tests resolve aliases and associated constants, reject unregistered literals and constants, reject stale entries, compare every local inventory owner predicate with its registry entry, check export mappings and retained policy phrases, and keep the orphan sweep registry-driven. The worker tests cover bounded passes, lease fencing and completed-job reconciliation. The integration table covers seeded local, joined-child and redacted rows, bystanders, organization ownership, retained rows and interruption/resume when the MongoDB test replica set is available. The worker reconciliation pass
also revisits completed jobs created before the current local-inventory revision
under the surviving admission barrier, so mixed-fleet completion does not leave
newly covered personal rows behind.

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
backend/src/models/assistant_agent.rs: COLLECTION_NAME=assistant_agents | created_by, org_maintainer_user_ids, org_user_ids, user_id
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
backend/src/models/assistant_system_skills.rs: COLLECTION_NAME=assistant_system_skills | updated_by (retained platform administrator attribution)
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
backend/src/models/credit_schedule_recipient.rs: COLLECTION_NAME=credit_schedule_recipients, SCAN_COLLECTION_NAME=credit_schedule_recipient_scans | recipient_user_id, cursor_user_id
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
backend/src/models/service_approval_config.rs: COLLECTION_NAME=service_approval_configs | approver_user_ids, user_id
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

## Ownership and retained lifecycle evidence

`ErasureDomain` names a real durable checkpoint and cleanup entry point. Domain
handling may retire a capability or retain a lifecycle tombstone; it does not
imply that every row is hard-deleted. The registry's rationale records that
collection's result. The behavioral inventory test seeds each declared owner
field independently, including nested fields and alternate legacy selectors,
and checks target, bystander and retained rows through job completion.

| Records | Ownership and disposition |
| --- | --- |
| Public deployment proofs and activations, Discord control installation/current history | Shared deployment provenance retains opaque sponsor/activator/requester identifiers. The sponsor is an organization; an actor identifier confers no resource ownership or export access. |
| Public probe consents | `actor_user_id` is personal consent; the local inventory erases it. Probe financial evidence retains the organization's sponsor and metering references. |
| Manager ingress/delivery and public sessions | Restricted replay, admission and recovery bookkeeping retains external platform identifiers. Public-session bindings clear after the fixed lifetime and confirmed remote cleanup; public turns separately expire through their `expires_at` TTL. |
| Personal platform bindings, link intents and challenges | The platform lifecycle revokes/cancels them first. Local deletion waits for durable platform cleanup and for pending personal Stop effects that reference active, person-owned local nodes, preserving their signing credentials until then. Retained Stop captures do not depend on binding rows. |
| MCP notifications | Ownership joins through `session_id` to `mcp_sessions`; there is no notification `user_id`. Children are removed before the credential-bearing session parent. |
| Telegram creation requests | `owner_user_id` owns the request; `actor_user_id` is separate attribution. Shared destination snapshots are redacted for the departing actor. Unstarted requests are cancelled under the erasure barrier; started requests remain until confirmed bot cleanup permits retirement. Managed-bot observations retain external bot identifiers and monotonic replay evidence. |
| Invite codes | Creator-authored notes are removed, usage email snapshots are redacted, and opaque creator/usage identifiers and counters remain. Exports contain only the caller's usage element. |
| Encryption migration records | Run records are global progress. Restricted findings retain keyed diagnostic identifiers and verification metadata. `encrypted_record_migration::lookup_finding` returns a 15-minute encrypted lookup cursor to the operator without persisting it; operator-derived owner lookup fields are not persisted ownership columns. |
| Telemetry capture accounts and dispatches | Capture generation and settled dispatch evidence remain as restricted erasure bookkeeping. Vendor completion fences this evidence; it does not hard-delete it. |

Orphan apply uses the same lifecycle predicates and durable admission barrier as
local erasure, with a current sweep lease write in every mutation transaction.
It rechecks joined parents and account/service-account/erasure-job liveness in
that transaction. Chained digest-bearing start and completion audits bracket the
mutations. Organization accounts are live owners; after organization removal,
only structurally orphaned rows in permitted cleanup classes are in scope.


Continuous remote cleanup waits are recorded on the pending local-inventory or source domain's
`remote_wait_started_at` using MongoDB server time. After 24 hours, the job enters
`operator_action` with `remote_cleanup_wait_exceeded`; an operator must resolve
the blocking dependency and retry. Pagination progress is not a remote wait and does
not consume this bound. Source/recovery credentials remain fenced until cleanup
is confirmed or the documented admin node deactivation clears that local
dependency; escalation never treats a failed remote deletion as success.

The gate runs in each local delete transaction. It also rechecks the particular
node immediately before draining or disconnecting it, because lifecycle recovery
may still persist a captured Stop after account admission closes. New personal
turns and machine scopes require the active-person/binding fence; accepted erasure
writes the same user inactive. Stop capture itself deliberately remains available
for previously admitted work. The pre-drain check therefore supplements, rather
than assumes, the admission fence.

Erasure acceptance already deactivates the person's ordinary nodes and sets
`personal_erasure_drain_required` in the acceptance transaction. A node-blocked
Stop therefore arises only for nodes still active from older acceptance paths;
the operator procedure below covers those jobs.

For `remote_cleanup_wait_exceeded`, use this operator procedure before retrying:

1. Read the person's erasure status and retain its job id, affected domain and
   current retry generation. Inspect the remaining remote recovery records by
   their exact owner and resource identities.
2. Check every predicate used by `personal_erasure_service::local::remote_cleanup_clear`:
   incomplete `platform_bot_cleanup` by `owner_user_id`; incomplete
   `platform_personal_stop_effects` by `active_turn.origin.user_id` only when a
   captured `machine_scopes.node_id` points to a `nodes` row with this owner's
   `user_id`, `is_active: true`, and a provisioning source other than
   `remote_machine`; and active `telegram_bot_requests` by `owner_user_id`.
   A binding's `active_turn` alone does not block deletion. Stop reconciliation
   reconstructs the exact capture from the retained effect, treats an absent
   conversation as settled, and does not read the binding. Missing, inactive,
   other-owned and remote-machine nodes do not block this local gate.
3. Resolve the underlying provider, gateway or machine connectivity/permission
   failure and let its existing cleanup worker verify the exact stop or deletion.
   Platform cleanup runs through `handlers::platform_bots::sweep_cleanup`;
   personal stop effects through `platform_personal_binding_service`'s stop
   reconciliation; Telegram creation recovery through
   `telegram_new_worker::complete_pending_creations`. Confirm those lifecycle
   records have reached their verified terminal state and the relevant predicates
   are clear. For a node-blocked Stop, bring the node online so reconciliation can
   send the exact cancel. If the node is permanently gone, an administrator must
   use `DELETE /api/v1/admin/nodes/{id}` before the operator retries the domain.
   This existing admin route has no owner-activity check and does not wait for
   Stop completion: it soft-deletes the node (`is_active: false`, status offline),
   deactivates its service bindings and disconnects it. Inactive nodes cannot
   authenticate or receive a machine cancel. The retry then allows the local
   pass to drain and delete the inactive node row. Its retained Stop effect may
   remain pending: `NodeUnavailable` is nonterminal, not proof of cancellation.
   Preserve encrypted capabilities and recovery inputs until the applicable
   predicate clears; never edit completion flags manually.
4. Submit the authenticated domain retry with the observed job id, current retry
   generation and a fresh idempotency key. A retry returns the explicit message
   that unresolved remote cleanup immediately returns to `operator_action`
   (`remote_cleanup_wait_exceeded`). Pending status retains
   `remote_cleanup_verification_required` until the worker evaluates it.
5. Poll status until the affected domain advances. The retry keeps the original
   remote-wait clock, so retrying before the remote effect clears immediately
   re-escalates; it does not grant another 24-hour waiting period.

Two existing protocols can remain nonterminal indefinitely: Telegram provisioning
is explicitly excluded from `telegram_new_service::TelegramNewService::expire`
(the ordinary 15-minute request expiry does not cancel provisioning), and
`telegram_new_worker::complete_pending_creations` retries with bounded backoff but
no terminal attempt cap. `platform_bot_config_service::claim_cleanup` and
`handlers::platform_bots::sweep_cleanup` likewise retain unsuccessful cleanup,
retrying without a TTL or terminal attempt cap. Their per-call deadlines/claim
leases bound individual work, not total recovery time. The erasure wait escalation
therefore surfaces both cases without discarding the remaining recovery evidence.


Orphan apply selects the completed dry-run by its returned SHA-256 digest and
consumes that job's persisted candidate pages. It does not rescan the database;
new orphans need a new dry-run. Only one pending apply can hold a reviewed digest,
even across different source dry runs. Admission inserts a fence keyed by digest
in `personal_erasure_orphan_sweep_fences` and claims `active_apply_job_id` on the
source in the same transaction as inserting the apply. The default `_id` index
provides exclusivity; no secondary index is added. Admission also checks pending
applies created before digest fences existed. A concurrent apply returns `1004 conflict` with
`An apply is already pending for this digest`. Every terminal transition,
including failure after an expired claim reaches the retry cap, releases both
source and digest fences in the same transaction. A later apply can process previously
blocked rows after their blocker clears. Every mutation still checks current
account and parent liveness transactionally. `candidate_count` is the reviewed count;
`noop_count` counts vanished, re-parented or already-redacted candidates;
`skipped_count` separately counts blocked candidates, including owners that are
live again, remote cleanup still pending, or lifecycle predicates refusing a
delete. Row mutations, both counts and page advancement commit in one majority
transaction, so page retries cannot double-count outcomes. A completion audit
means the reviewed page set was processed; skipped rows need a later apply after
their blocker clears, even when a new dry-run has the same digest.
Eight consecutive failed page attempts, including expired leases and abandoned
claims, terminate the job as `failed` with `page_retry_limit_exceeded`.
Deployment interruptions count: a worker killed mid-page leaves an abandoned
claim, and the next claim records one failure when that lease expires. Each
successful page checkpoint resets the consecutive-failure streak. After eight
consecutive failures, inspect and resolve the cause, then start a fresh apply
from the still-valid reviewed dry-run; if its pages have expired or been purged,
run a new dry-run first. The failed job remains terminal; recovery uses a new job.
Finished jobs and identifier-only pages are eligible for cleanup after seven
days. Each worker pass purges at most 128 pages from each of at most 16 finished
jobs; pending apply jobs pin their source dry-run pages. After retention expires,
apply admission returns `1004 conflict` with
`Reviewed dry-run expired or unavailable; run a new dry-run`. It also refuses a
source whose page purge has started, checking retention and purge state again
inside the admission transaction. Run a fresh dry-run before applying.
No additional large-collection index is
needed: page reads and deletion use the existing `_id` prefix.

Terminal `durable_operation_grants` and `durable_operation_executions` are compacted
by `personal_erasure_service::local::retain_settled_evidence`: original rows are
removed, while `personal_erasure_local_receipts` retains opaque `user_id`, row id
and settlement evidence. Receipt ownership is therefore personal bookkeeping.
`audit_log` stays immutable; its separate registry-declared `AuditMetadata` export
view returns only event type and creation time for the authenticated person.


The large message-history export also adds one compound index in its owning
module: `assistant_nyxagent::ensure_account_export_indexes`, called and awaited
by `personal_erasure_service::ensure_startup_indexes` before listener binding.
`assistant_messages_account_export` has keys `{user_id:1, created_at:-1, _id:-1}`.
`assistant_nyxagent::begin_owner_turn_in_session` and `finish_turn_inner` append
one row per input/reply without a transcript TTL. The existing
`conversation_id/seq` index cannot serve an owner's cross-conversation
newest-first read or explicit authored-input branch. This single index bounds
that per-event read without a blocking sort; no existing index is dropped,
renamed or rebuilt. Like the existing synchronous additions described above,
startup awaits a large-collection build and may be delayed. Pre-build with:

```javascript
db.assistant_messages.createIndex({user_id:1, created_at:-1, _id:-1}, {name:"assistant_messages_account_export"});
```

The message index joins the 17 erasure indexes above. Upgrade tests allow
precisely the declared additions and preserve the previous
index set, including on an older-replica startup.


Two additional joined-export indexes replace full child-collection scans with
indexed parent-key reads. They are feature-owned in
`platform_provisioning_service::ensure_account_export_indexes`, awaited from
`personal_erasure_service::ensure_startup_indexes` before listening, with the
same synchronous-build/prebuild behavior as the message index. This extends the
previous erasure-only decision to leave these small lifecycle collections
unindexed: a repeated self-service export now requires selective child reads.
These two keys are explicit export exceptions, not a blanket owner-index rule.

| Collection | Key/name | Reason and startup behavior |
| --- | --- | --- |
| `platform_delivery_tests` | `installation_id`; `account_export_platform_delivery_tests_installation_id` | `platform_runtime_service::begin_test` creates per-attempt UUID rows with a 10-minute expiry; their ids cannot be derived from installation ids. Each export must select the person's rows without scanning other owners' tests. Startup awaits the build; prebuild on a large deployment as above. |
| `platform_setup_steps` | `setup_id`; `account_export_platform_setup_steps_setup_id` | `platform_provisioning_service` stores step jobs keyed by creation-request UUID, with setup ids as the authoritative parent relation. Parent-scoped export reads need this separate join key. Startup awaits the build; prebuild on a large deployment as above. |

```javascript
db.platform_delivery_tests.createIndex({installation_id:1}, {name:"account_export_platform_delivery_tests_installation_id"});
db.platform_setup_steps.createIndex({setup_id:1}, {name:"account_export_platform_setup_steps_setup_id"});
```

The additive rollout comprises 17 erasure indexes and six export indexes. Existing definitions remain untouched; the upgrade tests compare
all definitions before/after and replay the old startup fixture afterward.


Three growing history collections also need export selector indexes. Actor selection is
independent of resource ownership: an organization or group can retain many events
authored by other people. Attachment selection must cover every origin, including
tool images without a channel source. The non-partial owner-prefix compound
indexes created by `channel_meta_service::scope_erasure::ensure_indexes` cover
attachment exports across all origins. These are the only additional selector
indexes; small configuration, relationship and lifecycle inventories retain their
existing index sets.

| Collection | Key/name | Writer/volume justification | Startup build behavior |
| --- | --- | --- | --- |
| `assistant_checkpoints` | `user_id`; `account_export_assistant_checkpoints_user_id` | `assistant_nyxagent::write_checkpoint_in_session` replaces a conversation's checkpoint, but conversation histories grow platform-wide with no fixed retention or total cap. Exporting by person needs a selective owner prefix. | `main` awaits `personal_erasure_service::ensure_startup_indexes` → `assistant_nyxagent::ensure_account_export_indexes` → `ensure_checkpoint_export_index` → `create_index` before binding listeners. Startup waits for a large build; prebuild below. |
| `assistant_group_requests` | `actor_user_id`; `account_export_assistant_group_requests_actor_user_id` | `collaboration_group_service::requests::enqueue_with_uploads_ready` appends a UUID request per authored group message. The pending queue cap covers active requests only; settled requests have no TTL. The existing `user_id` owner index and `collaboration.version/actor_user_id` index cannot serve the actor-only export predicate. | `main` awaits `personal_erasure_service::ensure_startup_indexes` → `collaboration_group_service::ensure_account_export_indexes` → `create_index` before binding listeners. On a large collection startup waits for the build; prebuild below. |
| `org_channel_messages` | `author_user_id`; `account_export_org_channel_messages_author_user_id` | `org_channel_service::transcript::publish` persists each member-authored transcript row; no transcript TTL. Existing `_id`, `org_id` and `user_id` prefixes cannot select an author's history across organizations. | `main` awaits `personal_erasure_service::ensure_startup_indexes` → `org_channel_service::ensure_account_export_indexes` → `create_index` before binding listeners. On a large collection startup waits for the build; prebuild below. |

This follows the existing awaited large-collection additions in
`assistant_nyxagent::ensure_indexes` and the message-history export index above.
Each owning initializer reuses any non-partial index with the required first key,
regardless of its name. The Meta erasure indexes on `assistant_attachments` and
`assistant_voice_announcements` start with `user_id` and cover their owner reads.
The fresh, upgrade and mixed-fleet tests include these additions and preserve the
original definitions. Prebuild on a large deployment before restarting replicas:

```javascript
db.assistant_checkpoints.createIndex({user_id:1}, {name:"account_export_assistant_checkpoints_user_id"});
db.assistant_group_requests.createIndex({actor_user_id:1}, {name:"account_export_assistant_group_requests_actor_user_id"});
db.org_channel_messages.createIndex({author_user_id:1}, {name:"account_export_org_channel_messages_author_user_id"});
```

The export-selector coverage test enumerates the actual filters for every
registry subsection and additional view, including each `$or` branch. Joined
views check both the parent owner prefix and the child foreign-key prefix;
message scope checks the conversation-owner query too. Selective prefix reads
bound the candidate set before the general newest-first top-k sort. The large
message history additionally requires its compound owner/time/id index so its
sort does not scan the person's entire transcript. Partial indexes do not count
as coverage unless the export predicate guarantees their condition (currently
none are needed). Adding or changing a selector without coverage or an explicit
exception fails the startup/upgrade tests. A redundant or stale exception fails
too. Group messages have no `approved_user_id` selector: owned-group queries
anchor on `user_id`, and the additional authored-message view anchors on
`author_user_id`; both are indexed.

The following export exceptions are verified against production writer or index
symbols by `export_exception_evidence_and_fixed_lifetimes_match_writers`. The
short-TTL entries pin the actual writer lifetime expression/constant and its
use, and startup tests verify the expiration field/index; a zero-second TTL
index alone is not a lifetime bound. Resource-bounded means metadata at
resource, relationship or configuration cadence, not a hard global row cap.
Where histories or unresolved lifecycle rows have no TTL, the table says so;
exports still retain their query timeouts and report unavailable subsections.

| Collection / selector | Classification and verified evidence |
| --- | --- |
| `agent_key_login_requests.approved_user_id` | **SmallOrResourceBounded**: auth_agent_key_login_service::request writes REQUEST_TTL_SECS=600 login requests; sweep_expired revokes undelivered credentials before terminal-only TTL can purge them. Approved/incomplete cleanup is deliberately retained, so no unconditional short TTL is claimed. |
| `assistant_agent_learning_members.member_user_id` | **SmallOrResourceBounded**: assistant_agent_learning::set_member_opt_in replaces one agent/member relationship by its deterministic id. |
| `assistant_agent_learning_runs.owner_id` | **SmallOrResourceBounded**: assistant_agent_learning::create_run/reserve_run stores analysis-run metadata, not each source turn; at most MAX_PENDING=16 queued/analyzing runs per agent. Historical runs have no TTL or hard total cap. |
| `assistant_agent_learning_skill_roots.owner_id` | **SmallOrResourceBounded**: assistant_agent_learning::ensure_indexes makes (agent_id,skill_id,version) unique; assistant_agent_learning_review writes one root per published skill version, not a turn log. |
| `assistant_agent_private_notes.actor_user_id` | **SmallOrResourceBounded**: shared_agent_service::update_notes replaces the agent/actor notes document; ensure_indexes uniquely fences that pair. |
| `assistant_agent_shares.owner_id` | **SmallOrResourceBounded**: shared_agent_service replaces each agent's audience; MAX_SHARES=500 and unique agent/subject pairs bound each current audience. |
| `assistant_agent_sharing.owner_id` | **SmallOrResourceBounded**: shared_agent_service stores one current sharing policy with _id equal to the agent id. |
| `assistant_async_quotas.user_id` | **SmallOrResourceBounded**: async_service_operation quota admission upserts _id=user_id; one counter per person. |
| `assistant_cma_agents.user_id` | **SmallOrResourceBounded**: assistant_cma_agent_service::register_live upserts one owner/service/remote-agent profile, not each task. Its existing owner prefix is partial on cma_agent_id being a string; the owner-only export does not guarantee that predicate. |
| `assistant_cma_sandboxes.user_id` | **SmallOrResourceBounded**: assistant_cma_sandbox_service::indexes permits one occupied account slot per owner; creation stores numbered resource generations, not per-task events. Historical generations persist for receipt reconciliation. Existing owner prefixes are partial on occupied slots, idempotency keys or remote ids, none guaranteed by the export. |
| `assistant_group_agent_access.owner_id` | **SmallOrResourceBounded**: collaboration_group_service has one current access relationship per group/agent, enforced by a unique index. |
| `assistant_group_agent_proposals.proposed_by` | **SmallOrResourceBounded**: collaboration_group_service::membership upserts deterministic group/proposer/agent proposals with a seven-day expiry; this is membership metadata. |
| `assistant_group_invites.accepted_by_user_id` | **SmallOrResourceBounded**: collaboration_group_service::membership records the sole accepting member on the same expiring invitation. |
| `assistant_group_invites.intended_user_id` | **SmallOrResourceBounded**: collaboration_group_service::membership records an optional intended member on the same expiring invitation. |
| `assistant_group_invites.issuer_user_id` | **SmallOrResourceBounded**: collaboration_group_service::membership stores seven-day, single-use group invitations with at most 32 pending per group, not group messages. |
| `assistant_group_read_states.user_id` | **SmallOrResourceBounded**: collaboration_group_service::notifications updates one read position per group/member; ensure_indexes uniquely fences that pair. |
| `assistant_learning_publication_targets.owner_id` | **SmallOrResourceBounded**: assistant_agent_learning_review reserves a single publication target per skill/version and removes released reservations; ids are target:version, not event UUIDs. |
| `assistant_skill_draft_attempts.user_id` | **ShortTtl**: db::ensure_indexes fixes the created_at TTL at 86400 seconds; assistant_skill_authoring::record_format_rejection updates one conversation/turn counter. |
| `assistant_voice_windows.user_id` | **SmallOrResourceBounded**: assistant_voice::ensure_indexes uniquely fences billing_request_id; billing::voice reserves deterministic call/window funding records, not audio frames. |
| `assistant_wire_logs.user_id` | **ShortTtl**: assistant_wire_log_service::WIRE_LOG_TTL_SECS is 15 minutes and record sets expires_at from it. |
| `auth_device_codes.approved_user_id` | **SmallOrResourceBounded**: auth_device_service writes 10-minute login requests but purge_at is set only after grant cleanup. Pending/approved cleanup can be retried, so AUTH_DEVICE_EXPIRES_IN_SECS is not a fixed retention bound; these are login lifecycle records. |
| `auth_device_codes_v2.approved_user_id` | **SmallOrResourceBounded**: auth_device_service shares the ten-minute login writer with the legacy protocol, initially leaving purge_at unset. Cleanup must revoke undelivered grants before purge, so this is login lifecycle inventory rather than an unconditional TTL bound. |
| `authorization_codes.user_id` | **ShortTtl**: oauth_service::authorize writes expires_at=now+Duration::minutes(5). |
| `channel_adapter_allocations.user_id` | **SmallOrResourceBounded**: call_channels::session uniquely fences catalog/provider/account/resource allocations; management inserts on resource allocation, never for a call event. |
| `channel_email_subscriptions.user_id` | **SmallOrResourceBounded**: channel_adapters::aurinko_lifecycle keys the one callback subscription binding by bot.id. |
| `device_codes.approved_by_user_id` | **ShortTtl**: device_code_service uses DEVICE_CODE_EXPIRES_IN_SECS=900 while pending and DEVICE_CODE_DELIVERY_EXPIRES_IN_SECS=86400 after approval. |
| `feature_flag_overrides.target_key` | **SmallOrResourceBounded**: feature_flag_service updates the current flag/target configuration instead of appending evaluations. |
| `invite_codes.usages.user_id` | **SmallOrResourceBounded**: invite_code_service stores signup invite configuration with embedded usage entries, not a separate per-request stream; max_uses controls redeemable uses. |
| `invocation_diagnostics.actor_user_id` | **ShortTtl**: invocation_diagnostic_service::HANDLE_TTL_SECS=60; diagnostic handles expire from creation. |
| `login_approvals.user_id` | **ShortTtl**: login_approval_service::begin caps request-bound approval expiry at ten minutes. |
| `machine_orphan_inventory.owner_id` | **SmallOrResourceBounded**: remote_machine_events::record_investigation upserts one finding per controlled-resource identity; recurring sweeps refresh last_seen_at rather than append events. No TTL is claimed for unresolved findings. |
| `mfa_challenges.user_id` | **ShortTtl**: token_service::insert_mfa_pending_session_with_method writes a five-minute pending session. |
| `oauth_consent_requests.user_id` | **ShortTtl**: oauth_consent_request_service::create fixes expires_at at 24 hours. |
| `openclaw_channel_mappings.nyxid_user_id` | **SmallOrResourceBounded**: openclaw_channel_service::upsert_mapping updates the current channel/external-user binding instead of creating a row per message. |
| `oracle_workers.enrollment.owner_user_id` | **SmallOrResourceBounded**: oracle_worker_enrollment_service serializes enrollment on the pool and caps enrolled workers at MAX_ENROLLED_WORKERS=256; one enrollment embedded per worker. |
| `org_channel_lanes.actor_id` | **SmallOrResourceBounded**: org_channel_service::transcript::lane upserts one current channel/partition/actor/agent/binding-generation/sender lane, not each message. |
| `org_channel_link_challenges.user_id` | **ShortTtl**: org_channel_service::links::issue writes a ten-minute expires_at; the lifetime applies to issued, observed and confirmed rows. |
| `org_channel_sender_links.user_id` | **SmallOrResourceBounded**: org_channel_service::links::confirm reuses the channel/sender binding; a unique channel_id/sender_hash key prevents per-link-attempt history. |
| `org_invites.redeemed_by` | **SmallOrResourceBounded**: org_invite_service::redeem_invite records one redeemer on a single-use organization invitation. DEFAULT_INVITE_TTL=24h is caller-overridable, so it is not treated as a fixed short-TTL guarantee. |
| `platform_personal_setup_selections.user_id` | **SmallOrResourceBounded**: platform_manager_management replaces the selected setup under _id=personal binding.id; the current context also expires after 30 days. |
| `platform_public_probe_consents.actor_user_id` | **SmallOrResourceBounded**: platform_public_contract_service stores human readiness consent for five minutes; sweep_probes deletes expired consent rows. This uses an application purge, not a MongoDB TTL guarantee. |
| `service_account_key_read_grants.owner_id` | **SmallOrResourceBounded**: service_account_key_read_service replaces the single current key-read grant with _id=service account id. |
| `service_pool_member_health.owner_id` | **ShortTtl**: service_pool_health_service::HEALTH_RETENTION_MS=3600000; current health expires one hour after an observation. |
| `telegram_bot_claims.owner_user_id` | **SmallOrResourceBounded**: telegram_new_claims uniquely fences manager/bot claims. Redeemed claims intentionally unset purge_after, so their bound is bot provisioning metadata, not the initial 15-minute expiry. |
| `telegram_bot_requests.owner_user_id` | **SmallOrResourceBounded**: telegram_new_service keeps one active provisioning request per actor. Terminal requests purge after one day; provisioning/connected are exempt from expiry and are resource lifecycle records, not short-TTL event history. |

## Crash diagnostics collections

`client_error_reports` is owner-linked by `user_id` and is deleted by the local
personal-erasure inventory. Revision 5 reopens revision-4 and earlier local
checkpoints, including completed jobs, to remove these reports. Admission uses
the existing owner erasure barrier.
`client_error_report_counters` contains only anonymous hourly platform/outcome
counts with a 30-day TTL and has no owner to erase. Crash rows expire within 30
days and all reads enforce expiry. Sentry events have no person/account/IP/UA
mapping; local row removal deletes the only event-to-person link. Vendor expiry
and owner acceptance are specified in [CRASH_REPORTING.md](../CRASH_REPORTING.md).

`client_release_registry` has a **shared attribution** classification and a
**retained-with-reason** disposition. Its optional `registered_by` retains only
the opaque admin user ID, never names or emails. Personal erasure does not delete
these rows: they preserve operational release identity and verification for
reports from every account, independently of the registering administrator.
