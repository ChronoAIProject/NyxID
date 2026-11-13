# Security rollout readiness and required approval

Security rollout has two boundaries: what the serving deployment supports, and
what a stored policy authorizes. New configuration must not be mistaken for
proof that every serving process can enforce it. Stored operation scopes,
machine assignments and live ACLs keep their enforcement even when their
configuration flags are off. This change does not rewrite grants or turn every
feature flag on.

## Deploy and declare the serving roster

Drain all old or unregistered serving backend replicas and relevant writers.
Upgrade every replica, including replicas reachable through alternate ingress
or internal routing, before relying on required web approval or enabling the
listed security features. For activation, each process must use a unique, stable
`INSTANCE_NAME`, the same complete comma-separated `SECURITY_ROLLOUT_REPLICAS`,
and `SECURITY_ROLLOUT_DRAINED=true`. A single local deployment must explicitly
declare its own one-name roster too. Restart a process to change its local
roster or drain declaration.

The drain declaration and roster are operator assertions. Mongo observations
cannot discover an old binary hidden behind a load balancer, prove that an
omitted process was drained, or prevent an old binary from serving. A green
report is meaningful only after that operational cutover. Do not restart old
binaries against the cutover deployment.

Every upgraded process registers its actual startup generation before serving
and renews its observation every 10 seconds. Readiness accepts observations up
to 45 seconds old and rejects timestamps more than five seconds in the future.
It is anchored to the executing process's immutable generation and roster:
another fresh singleton cannot certify a process whose own observation expired.
Missing, duplicate, undeclared or differently declared rosters block **activation**.
Restarts retain distinct generations; an old fresh generation with the same name
blocks activation until it expires. Already activated capabilities can continue
serving through that overlap if every live generation supports them and all hard
checks pass. Observation reads use an
`observed_at` index and fail closed above 256 live generations. A seven-day TTL
removes old diagnostic rows; TTL deletion is not required for freshness.

Use the admin/operator endpoint `GET /api/v1/admin/security-readiness` to inspect
per-capability activation and serving state, expected roster and live generation count.

| Field | Meaning |
| --- | --- |
| `ready`, `blockers` | Existing strict activation result and unchanged blocker strings. |
| `activated`, `active_since` | Whether MongoDB records a prior strictly clean assessment, and its first timestamp (UTC, or null). This is capability support evidence, not a feature flag or an authorization grant. |
| `serving_ready` | A clean assessment, or recorded activation with only roster warnings. |
| `serving_with_roster_warnings`, `roster_warnings` | Whether serving is permitted despite roster inconsistencies, and the exact roster blocker strings (also listed when serving is refused). |
| `serving_blockers` | Hard blockers plus `capability_not_activated` when an unready capability lacks activation evidence. |

Startup and the ten-second heartbeat record clean capabilities as separate UUID
rows (`record_kind: capability_activation_v1`) in `security_rollouts`. A unique
capability index and insert-only updates preserve the first activation across
concurrent replicas. Records have no TTL and do not turn any flag on.
Recording is best effort: a failed evidence read/write never fails the observation
upsert or startup; the next heartbeat retries, and one capability's failure does
not prevent recording the others. Requests
never write activation records or trust a local activation cache; the existing
cutover read also loads activation evidence, keeping the healthy path at two
MongoDB commands (plus the existing platform catalog inventory). A clean request
can serve before the next heartbeat records evidence, but an inconsistent roster
cannot bootstrap an unrecorded capability.

Only `missing_or_duplicate_replica:*`, `undeclared_replica:*`,
`inconsistent_serving_roster` and `serving_roster_not_declared` are tolerable.
`SECURITY_ROLLOUT_DRAINED=false` on **any** live generation still fails all gated
traffic; option B does not relax the drain assertion.
Missing/expired executing-process observation, any undrained process, future
observations, unsupported protocol/capability on **any** live generation (including
duplicates and undeclared names), all platform-policy blockers and missing catalog
policy remain hard refusals. More than 256 live generations, an uninitialized
runtime, malformed persisted state and database errors also fail closed.

Serving under warnings schedules `security_readiness_serving_with_roster_warnings`
at warn level and as a system audit event through the chained append service.
A local throttle runs before any diagnostic I/O, followed by the shared
`RateWindowStore` (one event per distinct sorted roster-warning set per hour
cluster-wide). Both use a SHA-256 digest of the warnings, never raw names as keys.
The local map holds at most 256 conditions and evicts the oldest; the shared
window still limits emissions after eviction. Changed warning sets are reported
immediately. Payloads contain the warnings, live generation count, expected roster,
and `first_observed_capability`, which identifies the triggering capability and is
not an exhaustive list of affected capabilities. Diagnostics
run asynchronously with a five-second timeout; failures never fail or delay the
request. Failed attempts retry locally after 60 seconds; success and shared-window
denial retain the one-hour local delay. Shared admission and the chained audit
append commit together, so a failed append does not consume the shared hour. The admin report itself does not emit a serving event.

### Rolling out the serving fix (#2277)

1. Keep a clean, drained one-generation roster while introducing the fixed binary.
   An already-running pre-fix binary retains strict runtime behavior and **still
   fails closed during its own overlap**. Route traffic to the fixed process only
   after its readiness is established; this change is not graceful-drain support.
2. Let old observations expire (45 seconds), with every actual old worker drained.
   On the fixed process, inspect `/api/v1/admin/security-readiness`. For every
   capability needed by traffic, confirm `ready: true`, `activated: true` and a
   non-null `active_since`. Do not scale out based only on `/health` or a flag value.
3. Scale to two supported fixed replicas, even if the Deployment reuses one
   `INSTANCE_NAME`. Confirm `serving_ready: true`, `serving_with_roster_warnings:
   true` and the expected duplicate-name warning, while `ready` remains false.
   Exercise NyxBot/model traffic and inspect the warning audit. A supporting
   old/new generation overlap follows the same serving rules.
4. Before a platform rollout-control or publication action in the exhaustive
   [strict list below](#activation-versus-serving-entry-points), restore a clean
   complete roster and drain obsolete generations. Ordinary user and worker
   traffic, including permission creation and per-run enrollment, keeps serving.
   Shared names deliberately cannot activate a new capability. Roster-by-count
   or prefix, SIGTERM/graceful drain and verifying all background sweeps under
   multiple replicas remain separate work on #2277.

Activation evidence certifies a past observed snapshot; it cannot certify a
future replica or discover a hidden worker. Every serving request rechecks live
support. Activation records are permanent: binary rollback and later roll-forward
retain them, so supported traffic can resume under roster warnings without another
clean roster. This does not make old binaries safe readers or undo sticky cutover.

If that resumption is unacceptable, use an explicit maintenance reset: disable
affected ingress and subsystem flags where available, drain all accepted work,
and stop **every** backend/worker that publishes observations. Wait at least
45 seconds so the capability is hard-unready, then remove only its evidence:

```javascript
db.security_rollouts.deleteOne({
  record_kind: "capability_activation_v1",
  capability: "<exact capability key>"
})
```

Keep ingress disabled while bringing up a fully upgraded, clean, drained roster.
Confirm fresh `ready: true`, `activated: true` and `active_since` before reopening
traffic or scaling. Never delete `security_rollouts/platform-policy-v1`; that
separate sticky enforcement marker has no live downgrade procedure. Do not delete
activation evidence while healthy heartbeats can immediately recreate it.

To check the report before scaling (set `disabled` only to deliberately unused
optional capabilities, such as platform policy while its env is false and
automation v2 while its env is false):

```sh
curl --fail --silent --show-error -H "Authorization: Bearer $ADMIN_TOKEN" \
  "$NYXID_URL/api/v1/admin/security-readiness" > readiness.json
jq -e --argjson disabled '["platform:operation-policy","automation:coordinator-completion-v2"]' \
  'to_entries | map(select(.key as $key | $disabled | index($key) | not)) |
   all(.[]; .value.ready and .value.activated and (.value.active_since != null))' readiness.json
```

Use `disabled: []` when every capability is required. For a failing entry inspect
`blockers` first, correct the clean-roster/drain/support/policy issue, and wait for
the next heartbeat. If `ready` is true but `activated` remains false, inspect the
distinct `Security capability activation recording` logs and restore MongoDB
read/write/index/validator health; requests do not create missing evidence.

The public `/health` endpoint remains a process health check. Neither endpoint
establishes OS user separation, node key-service custody, or externally
witnessed audit integrity.

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
| `permissions:execution-authoring-v1` | Part A assistant MCP/proxy proposal readers/writers, human decisions, retained-effect resume and legacy decision refusal. Upgrade/drain every writer before activation. |
| `permissions:execution-authoring-v2` | B1 personal external-key, standalone LLM, native-key, watch and human voice-task proposals. Default connected waits never become background work; external background execution requires explicit async opt-in. Required for person/org/global activation alongside the existing capabilities. |
| `permissions:execution-authoring-private-conversations-v1` | Optional v3 assistant proposal reader/writer and explicit across-private-conversations choice. Gates v3 creation, decisions, resume and send; does not become a prerequisite for existing cohort activation or v1/v2 sources. Upgrade/drain every reader and worker first. |
| `permissions:execution-enrollment-v1` | Broad enrollment snapshot, scoped hints and live admission integration. Advertised by production heartbeats with complete auth/admission and Phase B integration. Required for global/org activation and clear-that-enables. |
| `assistant:org-instruction-v1` | Exact owner instruction or a single-use owner card before covered org credential effects, in every access mode. |
| `assistant:hosted-connect-intent-v1` | New assistant MCP and Plugins links persist immutable personal/platform intent; completion always enforces an existing lock. |

Admin enable writes return HTTP 503 (`security_readiness_unavailable`, code
12900) while readiness is red; disabling remains allowed. Request-path reads
return the resolved boolean, preserving existing overrides, membership changes,
override clearing and defaults even if readiness later becomes unavailable.
Enabled gated reads emit `security_readiness_flag_in_effect_unready` only when
the capability cannot serve (`serving_ready: false`), at most once per flag per
minute cluster-wide. A local per-replica throttle runs before any diagnostic I/O,
then a shared rate window admits the serving-readiness report. The event includes
the flag key, user ID, strict `blockers` and actionable `serving_blockers`, with
the user as audit actor. An activated capability serving with roster warnings
emits no per-flag event: the same serving report schedules the shared hourly
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
fetch/bind rechecks require serving readiness after a clean, drained activation.
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
and have the setting true. Initial capability activation is strict; subsequent
credential use may tolerate only roster warnings with durable activation evidence.
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

Prepare policies and the exact serving roster before enabling this variable.
Once cut over, restore all hard checks and policies to recover serving; restore
a complete clean roster to recover activation readiness. Changing an env value to
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

The default-off capabilities `assistant:org-instruction-v1` and `assistant:hosted-connect-intent-v1` use the same admin enable readiness checks for global, org and user overrides. Deploy compatible backend readers and the frontend first: they must understand `org_connection`, preserve its pending cards/controls in Full, and read defaulted intent locks/receipt fields. Then upgrade and drain every API/admission, executor, job/watch worker and link-completer writer, including alternate ingress. Declare the complete `INSTANCE_NAME`/`SECURITY_ROLLOUT_REPLICAS` roster and `SECURITY_ROLLOUT_DRAINED=true`; confirm both capabilities ready before enabling the rules or locked writes. Old binaries must not rejoin after activation. Flag disabling is not a binary rollback plan.

Legacy selection rows have no positive owner provenance and never become instruction. New exact org bindings omit only mode preference, so pending cards remain redeemable after Auto-to-Full; old bindings are revalidated and may require a fresh exact card. New bounded jobs/watches persist server-owned instruction receipts. Old org jobs without proof refuse and require a reviewed replacement command; old org watches pause for an owner card covering the existing remote operation's declared status/result/cancel lifecycle. Old automatic Full receipts are insufficient. Legacy connect links default the lock to false and retain completion overrides; locked links enforce intent even if the write flag is later disabled. Complete deployment and activation evidence belongs in the implementation/PR review report; do not infer a drained writer from a green load-balancer health check.

## Execution-permission broad cohorts

Before global/org activation, upgrade every serving replica and worker, drain old
writers, and verify `permissions:execution-v1`, `permissions:execution-dispatch-v1`,
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
channel/settings policy writers and outbox cleanup, then drain old writers before
allowing this capability to become ready. Fresh unready sources stay legacy;
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
is insufficient to enforce the exact-current-turn guest boundary. Deploy all API
and async workers, drain older workers, inspect both readiness reports, then
opt in a linking person before broader rollout. Flag-off denies new guest
fetch/model reads, including model-setup waits, without deleting retained
human-owner uploads. Retention and erasure continue normally.

### Platform model family contracts

`inference:platform-model-contracts-v1` gates authoring and executing the new v2
family/transport fixture digests. Historical v1 tariffs keep their gates and
immutable digests. New tariffs use existing fields only, so old readers can
inspect rows, but old selectors/certification reject v2 before send. Upgrade and
drain all API/worker replicas before activation. New executors recheck the live
fleet before private credentials/funding and at dispatch; assistant callbacks
share these checks. Admin readiness includes the capability for affected models,
and public executable-price/image projections fail closed while it is unready.
See [platform models](PLATFORM_MODELS.md#current-models-through-versioned-api-contracts-2274-follow-up).

## Organization guest Grants cutover (#2366)

`nyxbot:org-guest-grants-v1` gates cap removal and explicit Grants adoption.
Verify it with `GET /api/v1/admin/security-readiness` after upgrading/draining
all API, proxy/MCP/LLM and queued/delivery workers in the existing declared roster.
No new environment variables or feature-flag policy are introduced; the existing
`nyxbot:org-specialist-channels` flag/readiness still applies.

- Unready/mixed fleet: retain cap/list enforcement; no adoption. Old binaries
  ignore additive markers and continue enforcing their original fields.
- Ready: remove the daily cap on every binding; preserve each legacy service list
  until its admin explicitly adopts saved Grants for all linked bots.
- New ready bindings: guest-off; enabling uses Grants (including default Use).
- Hard serving-readiness loss: use retained legacy restrictions; continue settling all holds. Roster warnings alone preserve already activated Grants execution and explicit adoption.

A registered green fleet cannot detect hidden old workers: roster/drain assertions
remain mandatory. Rollback disables guests and drains/settles work on upgraded
binaries before downgrade. Cap-free spend and all legacy settings remain retained;
old code may refuse above-cap admissions or changed digests. Never reset spend or
silently regenerate legacy authority to make rollback permissive.

### Activation versus serving entry points

This is the authoritative exhaustive list of operations requiring a clean roster:

- Platform feature-flag override enables and clear-that-enables, for every scope.
- Platform-admin model activation/enable, active route publication, model tariff
  creation, model-only catalog credential configuration, and model-evidence
  allowance authoring. The admin model enable-switch projection remains strict.
- Startup/heartbeat recording of a capability's first activation evidence.

Pure diagnostics (`/admin/security-readiness`, automation completion diagnostics,
and the existing flag-in-effect warning) retain strict `ready` and expose the
additive serving state; they do not authorize traffic.

Every end-user, org-admin, agent/API-key, channel and worker path uses
`require_serving` / `serving_report(...).serving_ready`. This includes permission
proposal creation/decisions, collaboration membership, native gateway selection,
org-channel linking/guest configuration/adoption, automation v2 per-run enrollment,
model execution, media, permission execution/resume/readback, and Discord socket
acquisition/renewal. Soft gates keep their existing legacy fallback on **hard**
serving-readiness loss. Database/operational errors retain each caller's existing
propagation behavior; this change does not turn such errors into a successful
fallback. Existing flag reads keep their boolean semantics.
`require_activation` permits unknown flag keys because ordinary flags are ungated;
`require_serving` explicitly refuses unknown execution capabilities.
