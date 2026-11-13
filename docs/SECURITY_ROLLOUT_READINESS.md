# Security readiness from observed fleet support

No configuration or operator step is required for security readiness. Start any
number of backend replicas with the default environment. Each process publishes
its generation and capabilities before serving, then renews every ten seconds.
A new capability becomes usable after every live generation supports it across a
90-second observation window. Roster names and drain declarations are optional
advisories; they never authorize or refuse a request.

Feature flags retain their defaults and precedence. The security-gated flags are
default off and still need an admin enable; recording capability support does not
enable a flag, publish a model, grant permission, or configure a provider. Stored
policy, live ACLs, revision checks, entitlements, tariffs and all other fences remain.

## Convergence and self-healing

Observations are live for 45 seconds and timestamps more than five seconds ahead
are refused. The executing process must have its own live generation observed;
a different healthy process cannot stand in for it. All live generations count,
including duplicate names and names absent from an optional roster. Reads use an
`observed_at` index and fail closed above 256 live generations. A seven-day TTL
removes old diagnostic rows, but expiry from readiness does not wait for TTL deletion.

Serving, model/route/tariff publication, model-only credentials, model-evidence
allowances and gated flag enables/clear-that-enables all require durable activation
and **no hard blocker**:

- The executing generation is missing or expired.
- Any live generation lacks the capability or uses another observation protocol.
- Any live observation is future-dated beyond the five-second allowance.
- For `platform:operation-policy` only: local or fleet policy configuration is
  disabled/inconsistent, or an active master-capable catalog row lacks its policy.
- More than 256 live generations, uninitialized runtime, malformed evidence or a
  database error prevents a complete assessment.
- No durable activation record exists yet (this is what the heartbeat resolves).

All roster conditions and `old_replicas_not_declared_drained` are **advisory**.
Missing or duplicate declared names, undeclared names, differing or empty rosters,
and a false drain declaration locally or on any live row never refuse serving,
publication, enables or recording. They still appear in the legacy strict result.

The heartbeat records immediately when the legacy `ready` assessment is clean.
Otherwise it records after consecutive assessments without a hard blocker across
`ACTIVATION_SETTLE_WINDOW = 2 × MAX_AGE_SECS` (90 seconds), with no gap between
successful assessments longer than `MAX_AGE_SECS` (45 seconds). A longer gap
restarts the window at the next successful assessment and resets its reported
countdown to 90 seconds; a gap of exactly 45 seconds retains continuity. This includes
empty rosters, missing declared replicas and false drain declarations. A hard blocker,
failed assessment, malformed evidence or inventory/write error resets the affected
window; a shared read/bound failure resets all pending windows. Each `Runtime`
owns a monotonic in-memory debounce; restart starts it over. It is advisory progress,
not cross-process authority, and requests never use it to authorize anything.

Activation is a separate UUID row (`record_kind: capability_activation_v1`) in
`security_rollouts`, with a unique capability index and insert-only upsert. It is
permanent and fleet-shared; competing replicas preserve the first timestamp.
Publishing the observation keeps its existing error contract. Recording is best
effort: its failures do not fail startup/observation, and one capability's recording
failure does not skip the others. Requests never write activation records or cache
them as authority; evidence shares the existing cutover read, so command counts stay
unchanged. Even a clean request waits until the heartbeat records its capability.

A supported scale-up, restart or crash does not erase recorded support. A newly
unsupported generation refuses that capability while it is live; after its last
observation expires, recorded capabilities resume automatically. A never-recorded
capability starts its 90-second window only after the unsupported observation is
no longer live: at least 45 seconds of freshness plus 90 seconds of settling after
that old generation's last heartbeat. Other capabilities remain independently usable.

## Optional declarations and what to set

For an ordinary Deployment, leave `SECURITY_ROLLOUT_REPLICAS` and
`SECURITY_ROLLOUT_DRAINED` unset. `INSTANCE_NAME` needs no special value: its existing
fallback is `POD_NAME`, then `HOSTNAME`, then `nyxid-backend`. Distinct generated pod
names and a shared name both work. After convergence, normal capabilities report
`activated: true` and `serving_ready: true`; `ready` remains false and the advisory
list explains the absent declarations. No roster-mismatch event is emitted for this
supported default. Without a roster there is no declared missing-replica diagnostic.

Optional declarations preserve the old diagnostic and immediate-recording path:

- Shared-name example: `INSTANCE_NAME=nyxid-backend`,
  `SECURITY_ROLLOUT_REPLICAS=nyxid-backend`, `SECURITY_ROLLOUT_DRAINED=true` on every
  replica. Two or more live generations produce `ready: false` and the duplicate
  warning; they serve and permit flag enables/publication after activation. New
  capabilities wait 90 seconds. This cannot detect a missing replica by count.
- Stable per-replica names (for example a StatefulSet): optionally list every name
  identically on every replica and set drained true to obtain legacy `ready: true`
  and immediate first recording while that declaration matches observations.
- If a Deployment uses generated pod names but an optional roster omits them,
  those generations are reported as undeclared. This is advisory, never a refusal.

These settings are still parsed and published unchanged so older binaries can read
them. In this binary, false and unset drain values have the same advisory meaning.
A declaration exists for mismatch logging when the local process or a live row
contains a nonempty roster or a positive `drained=true` assertion; historical false
booleans cannot distinguish explicit false from an unset variable.

Node socket ownership and cluster leases do not require unique instance names:
`node_dispatch.rs` compares name plus generation, `NodeOwnerFence` also binds the
connection, and `coordination_service.rs` filters by holder instance, generation
and lease ID. Generation identity, not the name alone, fences each process.

## Reading the report

`GET /api/v1/admin/security-readiness` requires admin/operator access and returns
one entry per capability, including optional `automation:coordinator-completion-v2`.
Public `/health` remains process health, not a fleet-support signal.

| Field | Meaning |
| --- | --- |
| `ready`, `blockers` | Unchanged legacy clean, declared, drained-roster assessment. Pure diagnostic and immediate-recording signal; nothing enforces it. |
| `activated`, `active_since` | Durable evidence exists and its first UTC timestamp (null without evidence). |
| `serving_ready`, `serving_blockers` | Durable activation plus no hard blocker; the sorted blocker list includes `capability_not_activated` until evidence exists. |
| `serving_with_roster_warnings`, `roster_warnings` | Serving with advisory roster/drain conditions; sorted existing blocker strings, also shown when hard blockers refuse serving. The drain condition is included here so all declaration advisories share the existing report/diagnostic envelope. |
| `activation_pending_seconds` | Nullable nonnegative integer: this responding replica's advisory seconds remaining, rounded up. Null without pending progress, after activation, or while currently hard-blocked. Zero awaits a successful heartbeat. It grants no authority. |
| `expected_replicas`, `live_generations` | Optional local declaration and the observed live-generation count. |
| `platform_policy_required`, `catalog_missing_policy`, `catalog_explicit_empty_policy` | Existing sticky policy state and catalog inventory diagnostics. |

Use `serving_blockers`, not `ready`, to diagnose an unavailable capability.
A missing record with a pending countdown converges automatically. A persistent
hard blocker identifies the unsupported process, policy issue, or unavailable
assessment. Recording errors have distinct `Security capability activation
recording` logs. Optional platform policy and automation still require their own
feature settings; readiness does not turn those settings on.

## Diagnostics

A pass inserting one or more records emits one info log and one chained
`security_capability_activated` system event. Payload: sorted `capabilities`
actually inserted by this process, `roster_clean`, `roster_warnings` (including drain
advisories), `live_generations`, `expected_replicas`. No insert means no event.
Audit work has a five-second timeout; failure is debug-logged and never changes
activation or observation success.

Serving with a mismatched **declared** roster/drain schedules
`security_readiness_serving_with_roster_warnings`: one warn and chained audit per
sorted advisory set per **24 hours cluster-wide**. An entirely undeclared fleet
emits none. A bounded local throttle runs before extra I/O; then a shared rate
window admits the append transaction. Digest keys contain no raw names and use a
daily domain distinct from old binaries' hourly keys. Changed advisory sets report
immediately. The local map holds at most 256 sets, evicts the oldest, and retries
failures after 60 seconds. Work is asynchronous with a five-second timeout and never
holds up a request. Shared admission and audit append commit together. Payloads
include advisories, count, expected roster and `first_observed_capability` (the
triggering capability, not an exhaustive list). The admin report emits no serving event.

Enabled gated reads emit `security_readiness_flag_in_effect_unready` only when the
capability cannot serve, at most once per flag per minute cluster-wide. It includes
legacy `blockers` and actionable `serving_blockers`. Advisory-only states do not
produce this event. Ordinary flag reads retain their resolved booleans and existing
local-before-I/O throttle; diagnostics failures remain non-fatal.

## Upgrade, rollback and the observation floor

Observations were introduced in commit `000672691` (#1976, 22 October 2026); the
first release tag containing that publisher is **v0.74.0**. A binary older than
that protocol which publishes no observation is invisible. This is the remaining
assumption: every serving process/worker relevant to these gates participates in
observations. The settle window restarts after assessment gaps longer than the
45-second freshness horizon, so time without observation cannot satisfy it. It
still cannot prove that a non-publishing or partitioned process is gone, or prevent
support from changing between an assessment and a request's effect. Observation
cannot discover a hidden pre-protocol binary or certify node OS isolation, provider
contracts or externally anchored audit integrity.

Current production #2422 (`f22e7048e`) can deserialize every record written here.
The UUID/string ID, record kind, capability and BSON `active_since` shape are unchanged;
no new fields are added to persisted activation or observation documents. Its own
declaration rules remain stricter, including a hard false-drain condition and
clean-only recording, flag enables and publication. Its legacy clean assessment
can also serve before a record exists; this binary always requires durable evidence.
This binary cannot change old code's decisions.

With production's current shared-name roster and drained=true settings, existing
activated capabilities serve on both versions during overlap. Old replicas still
refuse publication/enables under duplicates; new replicas allow them. A newly
introduced capability remains refused while an observed old replica lacks it.
After expiry, recording is immediate if the legacy declaration now matches;
otherwise the settle window applies. No declaration change is required.
Removing the declarations from new pods while #2422 pods still run publishes
undrained observations that those old pods refuse, even if their own drain setting
is true. The new pods tolerate those advisories; they cannot alter the old decision.

With no readiness settings, #2422 is already unready. New replicas can record and
serve supported capabilities after their window even while supporting old replicas
are visible; old replicas keep refusing because their local/observed drain settings
are false. New capabilities unsupported by the old version wait for its observation
to expire. Once all replicas run this version, convergence requires no operator step.

Rollback preserves readable documents and permanent activation timestamps. #2422
still applies its old gates: with valid drained declarations it can reuse records
written by the settle path for its supported capabilities; with no declarations it
fails closed again. A later roll-forward reuses the same records and rechecks current
support. This does not make unrelated new data formats safe for old readers:
feature-specific upgrade/rollback and retained-work requirements below still apply.
No clean-roster window, drain assertion, manual activation write or reset is required
for this readiness system. Use feature flags and authorization to disable behaviour;
deleting support evidence only delays it until a heartbeat records it again.

## Entry points and what requires a clean roster

**Nothing is refused for roster or drain warnings once a capability is recorded.**
A clean legacy assessment only makes first recording immediate rather than waiting
90 seconds. Empty rosters and missing declared names can settle too.

- `require_serving` / `serving_report(...).serving_ready`: user/worker execution,
  model and tariff publication, routes, model-only credentials, model-evidence
  allowances, admin projections, enrollment and native/Discord gateway work.
- `require_flag_enable`: flag enables and clear-that-enables at every scope,
  including execution-permissions prerequisites and all three platform-model
  prerequisites. It uses the same serving gate. Unknown ordinary flag keys remain
  ungated with zero readiness I/O; unknown execution capabilities fail closed.
- `report`: diagnostics expose both states. The heartbeat alone writes evidence.

Soft gates retain legacy fallback on hard support loss. Existing authorization,
revision, validation, funding, entitlement and protocol fences remain unchanged.

## Feature configuration and execution

Admin activation (`enabled=true`, global/user/org override) of the following
feature flags requires deployment readiness:

| Capability | Existing boundary retained |
| --- | --- |
| `channel:discord-gateway` | Relay socket ownership and gateway transport handoff; every transport writer must wait for relay shutdown before remote setup. The socket worker additionally requires live readiness at acquisition and renewal. |
| `assistant:org-agents` | Creation/configuration and use-oriented organization-agent discovery gate (`delegation_summaries`); direct use retains live membership/resource ACLs. |
| `assistant:operation-scopes` | Scope editing gate; stored scopes are enforced during authentication and execution with the flag off. |
| `assistant:machine-capabilities` | Capability editing and deliberate legacy-assignment materialization behavior; stored assignments and live machine ACLs retain their execution checks. |
| `assistant:machine-contexts` | Separated-context configuration; backend observations do not certify the node's OS implementation. |
| `assistant:live-turn-gate` | Conditional execution/route gate; channel and platform turn fences and conversation-key management denial retain their stronger unconditional checks. |
| `permissions:execution-v1` | Explicit execution-contract reader. Per-person activation requires reader/dispatch/authoring; broad activation additionally requires enrollment readiness below. See [contract](EXECUTION_PERMISSIONS.md). |
| `permissions:execution-dispatch-v1` | Phase A coordinator, live egress fences and one-operation claims. |
| `permissions:execution-authoring-v1` | Part A assistant MCP/proxy proposal readers/writers, human decisions, retained-effect resume and legacy decision refusal. Every live writer must advertise support before activation. |
| `permissions:execution-authoring-v2` | B1 personal external-key, standalone LLM, native-key, watch and human voice-task proposals. Default connected waits never become background work; external background execution requires explicit async opt-in. Required for person/org/global activation alongside the existing capabilities. |
| `permissions:execution-authoring-private-conversations-v1` | Optional v3 assistant proposal reader/writer and explicit across-private-conversations choice. Gates v3 creation, decisions, resume and send; does not become a prerequisite for existing cohort activation or v1/v2 sources. Every live reader and worker must advertise support. |
| `permissions:execution-enrollment-v1` | Broad enrollment snapshot, scoped hints and live admission integration. Advertised by production heartbeats with complete auth/admission and Phase B integration. Required for global/org activation and clear-that-enables. |
| `assistant:org-instruction-v1` | Exact owner instruction or a single-use owner card before covered org credential effects, in every access mode. |
| `assistant:hosted-connect-intent-v1` | New assistant MCP and Plugins links persist immutable personal/platform intent; completion always enforces an existing lock. |

Admin enable writes return HTTP 503 (`security_readiness_unavailable`, code
12900) when durable activation is absent or a hard support check fails; advisory roster/drain conditions do not refuse an enable. Disabling remains allowed. Request-path reads
return the resolved boolean, preserving existing overrides, membership changes,
override clearing and defaults even if readiness later becomes unavailable.
Enabled gated reads emit `security_readiness_flag_in_effect_unready` only when
the capability cannot serve (`serving_ready: false`), at most once per flag per
minute cluster-wide. A local per-replica throttle runs before any diagnostic I/O,
then a shared rate window admits the serving-readiness report. The event includes
the flag key, user ID, strict `blockers` and actionable `serving_blockers`, with
the user as audit actor. An activated capability serving with roster warnings
emits no per-flag event: the same serving report schedules the shared daily
roster-warning diagnostic described above. Healthy reads retain the existing
diagnostic command count. Readiness report or rate-window errors
are debug-logged and do not fail requests. An effective false value retains
existing flag-off behavior and does not erase stored policy.

Discord socket acquisition and renewal use serving readiness; roster warnings
alone do not stop an activated socket capability. Hard readiness loss stops it. Flag resolution
itself remains unchanged. Disable ingress and drain its socket holders before
rolling back to writers that do not coordinate the gateway handoff; see
[Discord relay setup](CHANNEL_BOT_RELAY.md#discord-ordinary-chat-and-interactions).

`assistant:channel-media-v2` gates new owner ingestion for Discord, Slack,
WhatsApp and X relay sources under the existing `assistant:channel-media` flag.
It is an execution capability, not a separate feature flag: admission and
fetch/bind rechecks require serving readiness after durable activation with all hard checks passing.
Lark/Feishu/Telegram retain their existing owner-media gates. No guest-media
authority is added. Keep upgraded workers until accepted media drains before
binary rollback; see [channel media](chat/10-uploads.md#channel-rollout-and-rollback).

The `approval:required-web-v1` capability is **diagnostic**, not an enforced
activation switch. Required approval is implemented by every upgraded process
without waiting for the readiness report. Drain and upgrade every serving
replica before relying on the new default and web fallback. A capability row
does not fence an old binary and is not a substitute for that cutover.

## Platform operation-policy cutover

`PLATFORM_REQUIRE_OPERATION_POLICY` starts false. The first startup observation
with it true persists `security_rollouts/platform-policy-v1` before registering
that process or serving traffic. This marker is sticky and does not expire.
All actor-addressed catalog master-credential authorization then requires the
executing process and every live generation to support this capability
and have the setting true. Initial capability activation is immediate on a clean roster or follows the
hard-blocker-free settle window; both paths retain every platform-policy hard check.
Subsequent credential use may tolerate roster warnings with durable activation evidence.
The sticky marker is enforcement intent, **not** activation evidence. A false-local
process must refuse even after the true sibling's heartbeat expires.

Readiness also inventories active master-capable catalog rows: authenticated
rows with explicit platform-key configuration or `requires_user_credential=false`.
This conservative superset excludes inactive and no-auth rows. Every selected
row needs an explicit `proxy_operation_policy`; no credential bytes are read.
Missing/null policy blocks readiness. An explicit empty rules array is counted
separately, is ready as configured, and denies all actual operations through
the existing operation evaluator. Server-chosen master-credential consumers
retain their separate existing contract.

Prepare the catalog policies before enabling this variable.
Once cut over, policy/configuration hard checks must pass to recover serving; no
roster or drain declaration is required. Changing an env value to
false, losing a heartbeat, or waiting for a lease does not downgrade enforcement. A deliberate rollback requires full
maintenance drain and an explicit deployment/data recovery plan; the product
does not automatically clear this marker or provide a live downgrade command.

## Required approval and delivery

A user with no explicit global preference defaults to **Writes only**: reads
pass, while writes and destructive operations require approval. This holds on
the first execution, before notification settings exist. Lazy channel creation,
including tool approval, leaves the preference absent and uses a 300-second
timeout. Explicit stored true means **All requests**; stored false means **Off**.
Existing booleans are never reinterpreted or migrated. The earlier unreleased
W9.3 build also wrote true lazily, without provenance: those rows keep true until
the owner deliberately chooses Writes only. Missing preference fields now mean
Writes only; missing timeout fields retain the legacy 30-second timeout.
Per-service and org policies, Deny effects, auto-connected exemptions, Session
bypass, guest restrictions, and scheduled/durable checks keep their precedence.

**All-replica cutover is mandatory.** Main-era replicas treat an absent channel
or preference as not-required and can execute a new user's writes unapproved.
The earlier unreleased W9.3 build instead requires all verbs and persists true
on lazy creation. Drain both versions from all ingress, internal routes and
workers; upgrade every participating replica before accepting new-user
programmatic traffic under the writes-only guarantee. Capability diagnostics do
not prove an old process is gone. A rollback reopens the old missing-preference
behavior; drain affected traffic before rolling back. Do not backfill booleans
as a substitute for completing the cutover.

When approval is required, a pending request is persisted before notifications
are attempted. Missing registration, absent provider credentials, invalidated
tokens or transient send failure leave it pending. Users can approve or reject
in `/approvals/history`; the normal and empty views expose **Refresh** to
discover requests created after the page opened. Expiry or rejection does not
execute the operation. Registration and last-channel removal never suspend a
stored required preference.

Notification settings distinguish these facts:

| Field | Meaning |
| --- | --- |
| `global_approval_mode` | `writes_only`, `all`, or `off`; preserves the distinct unset default. |
| `approval_required` | Compatibility flag: true for writes-only/all, false for off. Legacy boolean updates explicitly select all/off. |
| `approval_suspended` | Always false on this implementation; retained for older clients. |
| `approval_web_available` | Explicit true confirms this server implements required web decisions without delivery. |
| `approval_delivery_configured` | An enabled registration has matching initialized provider configuration. Delivery may still fail. |

Clients must preserve unknown values when talking to older servers. An absent
web capability or a true legacy suspension flag must not be displayed as a
guarantee of web-only enforcement. Keep a working notification channel until
the upgraded server explicitly confirms the new contract. Ordinary client UI
changes do not alter the separate mobile dependency remediation constraints.

## Protected org use and assistant hosted intent rollout

The default-off capabilities `assistant:org-instruction-v1` and `assistant:hosted-connect-intent-v1` use the same admin enable readiness checks for global, org and user overrides. Deploy compatible backend readers and the frontend first: they must understand `org_connection`, preserve its pending cards/controls in Full, and read defaulted intent locks/receipt fields. Every observed API/admission, executor, job/watch worker and link-completer must support both capabilities; their durable recording and serving readiness converge automatically before enabling the rules or locked writes. Old binaries must not rejoin after activation. Flag disabling is not a binary rollback plan.

Legacy selection rows have no positive owner provenance and never become instruction. New exact org bindings omit only mode preference, so pending cards remain redeemable after Auto-to-Full; old bindings are revalidated and may require a fresh exact card. New bounded jobs/watches persist server-owned instruction receipts. Old org jobs without proof refuse and require a reviewed replacement command; old org watches pause for an owner card covering the existing remote operation's declared status/result/cancel lifecycle. Old automatic Full receipts are insufficient. Legacy connect links default the lock to false and retain completion overrides; locked links enforce intent even if the write flag is later disabled. Complete deployment and activation evidence belongs in the implementation/PR review report; do not infer a drained writer from a green load-balancer health check.

## Execution-permission broad cohorts

Global/org enables require durable activation and live support for `permissions:execution-v1`, `permissions:execution-dispatch-v1`,
`permissions:execution-authoring-v1`, `permissions:execution-authoring-v2` and
`permissions:execution-enrollment-v1`. The
last capability also gates clear-that-enables; explicit disabling is always allowed.
Pilot orgs can inspect the admin flag response's scoped reconciliation progress.
Global activation uses bounded snapshot hints and never rewrites the population.
Auth adds no reads; fresh candidates resolve the live flag before marked admission.

For rollback, disable every effective cohort, including explicit org/user enables,
then drain marked tasks, proposals/resumes and Phase B descendants on upgraded
binaries before rolling back. Global false alone does not override a user/org true.
A racing admitted send may finish; flags are rollout controls, while saved permission
and claim fences remain authority. Off→on may resume pending valid marked work,
never a consumed/uncertain claim. See [enrollment contract](EXECUTION_PERMISSIONS.md#global-and-organization-enrollment).


Machine execution-permission authoring additionally requires
`permissions:execution-authoring-machine-v1`. Its closed Machine source and
retained-adapter variants are not readable by B1-only servers, so deploy every
API/worker reader before allowing these rows. This gate applies only to Machine
proposals; existing B1 sources remain available. Node-side
`machine_execution_approval_v1` negotiation is separately required and defaults
off when absent. Old nodes keep legacy prompts indefinitely. Drain/remove all
Machine-variant proposal rows, including terminal receipts, before a binary
rollback to readers without this capability. Disabling authoring never converts
a retained occurrence into a legacy request or permits another send.

`permissions:execution-authoring-channel-v1` is an optional v4 channel source,
review and result-delivery capability. It is not a new prerequisite for existing
web/v1/v3 or B2/v2 cohorts. Upgrade all API replicas/workers, deploy the monotonic
channel/settings policy writers and outbox cleanup. Observed support and durable
activation then allow this capability to serve without a drain declaration. Fresh unready sources stay legacy;
existing marked work refuses at task/decision/send/delivery barriers. Disable
and drain on upgraded binaries before a binary rollback. The saved permission
remains v1 with a Channel audience; it never becomes a private permission.

## Platform model foundation

PRs 1–3 advertise `inference:platform-model-registry-v1` for registry/entitlements,
`billing:model-prices-v1` for exact tariffs and `inference:platform-models-v1` for
the bounded stateless Chat executor. They do not advertise the paired NyxAgent
`assistant:platform-models-v1` contract. The default-off
`inference:platform-models` flag cannot be enabled until all required local and
fleet capabilities are implemented and ready. Reads and disabling remain usable.
Upgrade and drain every credential consumer before enabling model-only catalog
records: old replicas ignore their additive purpose field. The restriction remains
enforced when the flag is off. See [the registry contract](PLATFORM_MODELS.md).

PR 3b additionally advertises `inference:platform-model-images-v1`. Certified
image-tariff authoring and image execution require that capability across the
fleet, in addition to the existing model gates. Text execution and historical
text tariffs do not acquire an image dependency. The assistant guard remains;
this capability does not advertise `assistant:platform-models-v1`. No new
environment setting or automatic model/rate activation is introduced.

### Channel guest media

`assistant:channel-guest-media` is a separate default-off flag and replica
capability. Its readers also require `assistant:channel-media-v2`: Stage A alone
is insufficient to enforce the exact-current-turn guest boundary. Both capabilities
must be recorded and supported by every observed API/async worker before a linking
person opts in; no roster/drain declaration is required. Flag-off denies new guest
fetch/model reads, including model-setup waits, without deleting retained
human-owner uploads. Retention and erasure continue normally.

### Platform model family contracts

`inference:platform-model-contracts-v1` gates authoring and executing the new v2
family/transport fixture digests. Historical v1 tariffs keep their gates and
immutable digests. New tariffs use existing fields only, so old readers can
inspect rows, but old selectors/certification reject v2 before send. Observed
API/worker support gates activation automatically. New executors recheck the live
fleet before private credentials/funding and at dispatch; assistant callbacks
share these checks. Admin readiness includes the capability for affected models,
and public executable-price/image projections fail closed while it is unready.
See [platform models](PLATFORM_MODELS.md#current-models-through-versioned-api-contracts-2274-follow-up).

## Organization guest Grants cutover (#2366)

`nyxbot:org-guest-grants-v1` gates cap removal and explicit Grants adoption.
Verify it with `GET /api/v1/admin/security-readiness` after all observed API, proxy/MCP/LLM and queued/delivery workers support it.
No new environment variables or feature-flag policy are introduced; the existing
`nyxbot:org-specialist-channels` flag/readiness still applies.

- Unready/mixed fleet: retain cap/list enforcement; no adoption. Old binaries
  ignore additive markers and continue enforcing their original fields.
- Ready: remove the daily cap on every binding; preserve each legacy service list
  until its admin explicitly adopts saved Grants for all linked bots.
- New ready bindings: guest-off; enabling uses Grants (including default Use).
- Hard serving-readiness loss: use retained legacy restrictions; continue settling all holds. Roster warnings alone preserve already activated Grants execution and explicit adoption.

A green fleet cannot detect invisible pre-observation workers; the observation
floor above still applies. Rollback disables guests and drains/settles work on upgraded
binaries before downgrade. Cap-free spend and all legacy settings remain retained;
old code may refuse above-cap admissions or changed digests. Never reset spend or
silently regenerate legacy authority to make rollback permissive.
