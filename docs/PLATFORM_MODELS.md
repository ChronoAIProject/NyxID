# Platform models

This is the approved platform-model contract for
[#2274](https://github.com/ChronoAIProject/NyxID/issues/2274), including the owner's
public-name, credential and first-release scope decisions. It supersedes the
design's earlier brand-neutral naming and keyless-first recommendations.

**Current implementation: PRs 1–4 — registry, authorization, tariffs, bounded execution, and admin/customer/CLI surfaces.**
It stores configuration, evaluates live entitlements and executes the stateless
Chat text/function subset below through complete immutable model tariffs and
the existing billing path. Web and CLI surfaces inspect and administer these
contracts. NyxAgent selection and actor-bound internal execution remain PR 5;
this does not yet satisfy the real-conversation acceptance of #2274.
No models, credentials, prices, connections or cloud resources are seeded.

Public IDs are immutable real model names under `nyxid/`, for example
`nyxid/claude-opus-5-5`. The model is public; its route, upstream, project, account
and region are private. Every route must serve that same model. The foundation
accepts exact names or a dated spelling (`@YYYYMMDD` or `-YYYYMMDD`). Other
version equivalences require an explicitly reviewed adapter mapping; an arbitrary
alias cannot substitute another model.

## Implemented foundation: configuration and API

New collections are `platform_models` and `platform_model_routing_revisions`.
Model configuration embeds a bounded entitlement policy, so the kill switch,
policy and authorization revision change atomically. Route sets are immutable
revisions referenced by the model; old revisions remain readable. Every write
uses `expected_revision`, a transactional compare-and-swap, live admin-role
fencing and a chained metadata audit. The public ID and family cannot be changed;
retired IDs cannot be reused. A stale edit returns 409. A lost write response is
recovered by reading the model and its revision, not overwriting it blindly.

First-party human admins write; admins/operators read private configuration:

| Method and path (under `/api/v1`) | Purpose |
| --- | --- |
| `GET /admin/platform-models?after=nyxid/...&limit=50` | Private paginated inventory |
| `POST /admin/platform-models` | Create a disabled draft |
| `GET /admin/platform-models/{id}` | Private contract, routes and readiness |
| `PUT /admin/platform-models/{id}` | Revision-checked metadata/lifecycle edit |
| `PUT /admin/platform-models/{id}/routes` | Publish an immutable draft route revision |
| `PUT /admin/platform-models/{id}/entitlements` | Replace the bounded access policy |
| `GET /admin/platform-models/{id}/access?user_id={person}` | Explain effective person access without inference |
| `GET /llm/models?after=nyxid/...&limit=50` | Entitled public metadata only; empty while the flag is off |

Create example:

```json
{
  "public_id": "nyxid/claude-opus-5-5",
  "display_name": "Claude Opus 5.5",
  "description": "Claude Opus 5.5",
  "family": "claude",
  "capabilities": {
    "protocols": ["anthropic_messages", "openai_chat"],
    "input_modalities": ["text"],
    "output_modalities": ["text"]
  }
}
```

Capabilities are declarations, not adapter certifications. Unknown capability
values remain absent; authoring them never activates an executor. Updates include
`expected_revision`, display text, capabilities, lifecycle, enabled and optional
`billing_service_id`. Only an active model can be enabled. Activation requires the local execution
capability and the same capability on every declared live replica.
Disabling, retirement, reads and entitlement revocation remain available.

An entitlement body has `expected_revision` and `entitlements`:

```json
{
  "expected_revision": 1,
  "entitlements": {
    "all_people": false,
    "rules": [
      {"subject_kind": "person", "subject_id": "PERSON_UUID", "effect": "allow"},
      {"subject_kind": "org", "subject_id": "ORG_UUID", "effect": "allow"},
      {"subject_kind": "group", "subject_id": "GROUP_UUID", "effect": "deny"}
    ]
  }
}
```

Default deny; a matching deny wins over every allow, including `all_people`.
Organizations require a live active owner and active `can_proxy()` membership;
viewers do not qualify. Groups use live direct `User.group_ids` membership, with
an existing group row; parent groups are not inherited. Inactive people and
org-principal identities cannot acquire personal entitlements. Admin role never
bypasses execution entitlements. A membership/database error is not permission.
Membership snapshots are request-local. Listing applies policy **before** keyset
pagination, so cursors do not disclose inaccessible model names.

The logical offering retains existing service/key and org-resource ceilings.
Restricted keys must name an active authorized connection, respecting live
membership, inherited resource scope, `admin_only` and stored operation scopes.
Entitlements do not fund usage or broaden a specialist's service grants. PR 5 will
add the paired NyxAgent actor-bound inference contract; service-account, relay and
delegated discovery are not activated by this foundation.

## Private routes and credentials

Routes use closed typed targets:

- `vertex_publisher`: `publisher` (`google` or `anthropic`), project, location and
  native model; Gemini and Claude family checks apply.
- `vertex_openapi_maas`: project, location, publisher and native model; only the
  open-model family. The adapter will certify each publisher/model usage shape.
- `direct_provider`: `openai` or `anthropic` and native model; the same public
  model/family constraint applies.

Each route includes a UUID, credential catalog-service UUID, enabled state,
priority and positive weight. No target accepts a URL, caller route override,
secret or arbitrary pool. The executor constructs trusted authorities and
resolves the active route set server-side. No customer API returns the private
route plan. An internal metadata plan is **not** a decrypt/send permit.

`PlatformKeyConfig.purpose` is additive and defaults to `ordinary`. Existing
catalog records retain their behavior. Admins can stage dedicated records through
existing catalog APIs:

- `model_route`: private catalog master credential; restricted audience with no
  owner grants. Ordinary credential authorization, server-selected master use,
  generic proxy resolution, platform discovery and provisioning refuse it.
- `model_offering`: logical model service with no credential, not a generic proxy.
  It supplies a stable identity for later grants and billing.

Enabling a model-only catalog record requires registry fleet readiness. Its
purpose cannot be reset or changed by catalog updates; legacy forms omitting the
field are refused. A database predicate and the catalog authority transaction
fence stale saves racing a purpose transition. Generic execution checks the
restriction independently of the feature flag and readiness. Do not convert an
existing directly offered service to hide its routes: create a separate dedicated
record so current customers retain their existing product.

Credential material stays in the existing encrypted catalog field. The executor
validates a GCP key strictly as `service_account`, rejects executable/external/file
sources and untrusted token URLs, and mints/caches short-lived tokens using the
fixed Google OAuth token authority. ADC/WIF is deferred behind the token-provider abstraction.
This foundation does not claim that merely storing JSON certifies a GCP key.

## Deployment and readiness

`inference:platform-models` is a code-declared, default-off flag using existing
person/org/global overrides. No new environment variable is introduced.

PRs 1–4 advertise `inference:platform-model-registry-v1`,
`billing:model-prices-v1` and `inference:platform-models-v1`. They do not advertise
`assistant:platform-models-v1`. Flag enable and active-model publication fail
closed until this binary **and** the whole declared fleet support execution and
billing. Fabricating future capabilities in heartbeat rows cannot make a PR 1
binary activate. NyxAgent exposure additionally needs the paired assistant
capability. `nyxid/` requests cannot fall through to provider-name heuristics.

Before enabling model-only credentials, upgrade/drain all API replicas and every
alternate/internal credential consumer and writer. Use the existing immutable
`INSTANCE_NAME`, complete `SECURITY_ROLLOUT_REPLICAS` roster and
`SECURITY_ROLLOUT_DRAINED=true` declaration. Older binaries ignore `purpose`, so
optional/defaulted serialization alone is not a security rollout guarantee.
Old binaries must not rejoin after restricted credential records are enabled.

PR 3 supplies bounded admission and the four serving families described below.
PR 5 adds the paired NyxAgent contract. Prices never fall back silently to legacy
lanes. Metadata access or a manually inserted flag cannot bypass live execution
readiness, entitlement and funding checks. Ordinary services, pools and billing
retain their existing paths with the flag off.

## Approved execution contract and remaining integration

The first release includes **all four** serving families and the paired NyxAgent
integration. A picker or registry alone does not complete #2274. Public IDs such
as `nyxid/gemini-3-pro`, `nyxid/claude-opus-5-5` and `nyxid/gpt-oss-120b` are
illustrations, not seeded or certified offerings. A materially different model
gets a new ID. Routes may use equivalent endpoints/versions of that same model;
they may never silently substitute another model or family.

| Serving family | Upstream contract | Activation condition |
| --- | --- | --- |
| Gemini native on Vertex | `generateContent` / `streamGenerateContent` on `projects/{project}/locations/{location}/publishers/google/models/{model}` | Native usage extraction and certified translation for the advertised caller protocols |
| Claude on Vertex | `rawPredict` / `streamRawPredict`, Anthropic Messages envelope, `anthropic_version: vertex-2023-10-16` | Claude usage/cache fixtures and supported model/region |
| Open MaaS on Vertex | Documented OpenAI-compatible Chat Completions API | Per-model certified usage fixtures; no assumed cache or hidden token counters |
| Direct OpenAI/Anthropic | Existing admin-managed master credential and native provider protocol | Same public model, equivalent supported features and certified exact usage |

Vertex regional authorities are `{location}-aiplatform.googleapis.com`; the
supported global form uses `aiplatform.googleapis.com` with `locations/global`.
Adapters construct authorities from validated fields. A global endpoint is not a
residency promise. Project IAM, model enablement/terms, region availability and
usage fixtures must be verified before activation. Model Garden is a catalog of
several serving families, not a universal Chat Completions endpoint. Customer
endpoint deployment/training and uncertified models remain out of scope.

GCP credentials must be encrypted service-account JSON with
`type: service_account`. Reject external accounts, executable/file sources and
caller-controlled token authorities; never send the JSON itself as a bearer.
Use a token-provider abstraction with bounded, single-flight, short-lived token
caching keyed by credential identity/version and early refresh. Rotation
invalidates cached authority. Trusted Google token authorities are constructed or
strictly validated, never followed from arbitrary credential input. ADC/WIF and
impersonation onboarding are deferred. Secrets, identifiers from credentials,
prompts and completions do not belong in logs or attempt diagnostics.

The executor resolves a public model to a private route set server-side. It
intersects the exact request's protocol, tools, modalities, limits, residency,
credential authority and evidence contract. Clients cannot select a private
route, account, project, region or upstream. Reuse bounded priority/weight,
health/cooldown and deadline primitives; one planner owns retries. No mutable
user pool, recursive alias, generic proxy or caller destination may bypass this
boundary. A metadata plan is not a decrypt/send permit: the executor revalidates
the live actor, model, route/revision, credential, operation and funded admission.

A retry/fallback requires proof the request was unsent or rejected with zero
consumption. An HTTP status alone does not prove zero consumption. Never retry
after stream commitment, ambiguous dispatch, caller cancellation, lost authority
or lost funding lease. Known consumption settles its attempt and normally ends
the request. Missing final usage remains a recoverable hold, never a zero-cost
success or guessed final charge. Provider and Lago failures retry through the
existing recovery machinery without releasing an unsettled debit.

### Public/private boundary

Users know the model; only admins and trusted internal configuration see route
identity. Extract native usage before translating responses. Public output uses
allowlist DTOs and the requested public model ID. Replace native model/resource,
response and trace identifiers with safe NyxID identifiers; omit topology,
project/quota/debug headers, upstream error bodies, destination URLs and native
continuation/artifact URLs. Billing, exports, activities, metric codes and invoice
labels contain public model/component identities, never routing identities.

Future stateful continuations use owner-authorized NyxID handles bound to actor,
model, route, credential epoch and expiry. They cannot migrate provider state to
a different route. Stateless text/tools are the initial contract; media jobs,
cache storage and hosted tools require their own recovery and billing adapters.
Opacity applies to NyxID-controlled metadata. It does not promise to censor model
names from generated prose or suppress legally required publisher/grounding
attribution; incompatible features stay disabled pending review.

## Approved pricing and exact accounting contract

Reuse ordinary `usage_meter`, credential classes, funding, settlement, ledger,
rollups, Integrity and Lago. No separate wallet or billing pipeline is introduced.
A model selects one complete immutable retail tariff, independent of its private
route's cost. Model prices **never** fall back to a base or legacy lane price.
Missing, invalid, retired or partially synchronized prices refuse new admission.
Old services retain their existing lane/fallback semantics.

Immutable tariff revisions reference the logical offering and complete component
set; CAS publication changes only future admission. Freeze rates, units, model
and tariff identity with the reservation. Retain historic revisions and retired
rate-cache rows for settlement and historical explanations. Sync every component
before activation. Use stable, collision-checked, bounded Lago codes shaped as:

```text
platform_svc_<logical_slug>_<pk|byok>_mdl_<immutable_model_key>_t<tariff_revision>_<component_key>
```

The code must never contain upstream, project, account, region or route. The
`byok` shape leaves room for later support; new GCP BYOK onboarding is not part of
the first release. Existing BYOK products remain unchanged.

| First-release component | Required evidence |
| --- | --- |
| Input/output tokens | Provider integer counters with explicit inclusion rules |
| Cache reads/writes | Exact counts; writes qualified by TTL where priced differently |
| Reasoning tokens | Exact disjoint split; otherwise price aggregate output once |
| Image/audio input/output tokens | Exact disjoint counters; never estimate from bytes/duration |
| Image generation | Final images with dedupe; no billing of previews as additional finals |
| Requests | Defined billable request event and durable identity |
| Search/grounding | Only with adapters that explicitly enable and meter those tools |

Cache storage, hosted runtime, durations, deployments and other infrastructure
units remain follow-ups. Each observed price-bearing class must be charged once,
explicitly free, or explicitly included in an aggregate; unknown classes close
the route for new admission and preserve evidence for reconciliation.

Strict normalization distinguishes **zero, absent, inapplicable and invalid**.
Reject fractional/out-of-range counters; no float truncation or byte estimates.
Deduplicate cumulative/terminal stream frames. Provider-specific rules include:

- OpenAI cached input is a subset of inclusive input; reasoning is a subset of
  output. Subtract a separately priced subset exactly once.
- Claude ordinary input excludes separately reported cache creation/read counts;
  do not subtract caches twice. Preserve 5-minute/1-hour write distinctions.
- Gemini separates prompt, cached, candidate, thought and tool-use counters.
  Cached input is a prompt subset and thinking must be charged once, according
  to the certified model's counter semantics.
- Open MaaS gets no assumed cache or modality split without per-model fixtures.

Admission reserves a bounded complete component bundle atomically, using
actual-quantity metric-matched allowances, then soonest-expiring grant credits,
then wallet credits. No partial admission or unintended overdraft. All money uses
checked i128 picocredit `Credits` and Decimal128; derive the wallet share by
subtraction. Metering-only rows consume no benefits. Existing Lago-owned tax
applies only to wallet-funded usage, through the existing settlement path.

Only wallet-funded quantities go to Lago, using cumulative-floor durable residues
and claim-fenced funding outputs. Settlement preserves existing charge/dedupe
identities, v2 balanced hash-chained ledger postings, wallet locks, account
checkpoints, tax handling and recovery. Reconcile checks ledger chains, accounts,
Lago drift, missing evidence and pending/failed settlements. Disabling the feature
never releases or erases accepted obligations.

Normal usage rows, hourly/daily rollups, flat `query_*` mirrors and covering
indexes gain compatible public-model/component dimensions and measures together.
Do not duplicate request or token totals across financial component rows. User
billing/usage, admin usage, execution history, CLI and invoices show the public
model and readable units. Historical charge/explain selection uses the same
frozen tariff derivation; native `reported_model` values never reprice old usage.
See [exact accounting](BILLING_EXACT_ACCOUNTING.md),
[billing operations](BILLING_OPERATIONS.md) and
[platform pricing](PLATFORM_KEYS_AND_INFERENCE.md).

## Approved NyxAgent and internal-consumer contract

NyxAgent integration is required in the first release. A paired versioned
contract lets NyxAgent list/accept public IDs and call the NyxID gateway with
actor-bound authority. Admins configure private routes; neither the picker nor an
external caller supplies route selectors. Other first-party consumers enter the
same server-side model authorization boundary.

Intersect live model entitlement with service/key/operation, agent/specialist and
org-resource authority. Revalidate at list, turn start and dispatch; a stale list
or unreachable model-list provider cannot authorize a platform selection. Reject
revocation and unavailable selections with the assistant's existing
`model_unavailable` (12103) contract and actionable guidance. Never silently
switch model, identity or funding owner. Scheduled/replayed turns retain their
original scope ceilings and current authorization checks.

Platform inference is billed **once at the gateway**; any existing orchestration
charge remains distinct. Active org membership grants access, not sponsorship:
platform-master usage bills the acting person through the existing owner
resolver. Org-principal, service-account and guest use requires a separately
reviewed actor/payer contract and is refused for the new offering until then.
Existing products retain their current payer rules.

## Delivery and acceptance

| PR | Scope and release gate |
| --- | --- |
| 1 (this foundation) | Registry, readers, public/private DTOs, CAS/lifecycle, typed routes, credential confinement, entitlements, default-off flag/readiness. No executable capability. |
| 2 | Immutable complete tariffs, strict units/evidence, shared explain selector, Lago synchronization and compatible usage/rollup readers. No legacy-price fallback. |
| 3 | All four first-release serving families, GCP token provider, bounded atomic admission, opaque transport, retries, stream/cancel and recovery. |
| 4 (implemented) | Admin model/route/access/rate tools; public discovery, CLI and public-model billing/usage labels. No inference execution. |
| 5 | Paired NyxAgent/internal-consumer actor-bound integration, selection, revocation and single inference charging. Required for release. |
| 6 | Additional certified protocols/families and stateful/media/tool/storage/BYOK lifecycles, one bounded contract at a time. |

Each PR merges safely with the flag off. Do not advertise readiness before its
entire boundary works. Upgrade all admission, billing, reconciliation, rollup and
reader processes and drain older writers before new enum units, tariff fields or
credential restrictions can be authored/activated. Optional fields do not make
old binaries safe readers. Runtime admission checks local and fleet readiness;
upgraded workers continue settling accepted work after readiness loss.

Stage without seeding; verify credentials, terms, supported regions and complete
usage/prices with authorized provider canaries; enable an explicitly entitled
staff cohort before expansion. No provider resources or paid probes are created
by PR 1. Rollback disables every effective user/org override as well as the global
flag (a more-specific true overrides global false), suspends new admissions and
drains work while upgraded readers/reconcilers stay running. Never rewrite or
delete financial history to permit a binary downgrade.

Required tests include live entitlement revocation and deny precedence, membership
and key ceilings, source-free public DTOs, CAS races and alternative-ingress
credential denial; adapter/auth/usage fixtures; complete-tariff and atomic-funding
failure cases; stream ambiguity, recovery and duplicate settlement; usage/rollup
and bill parity; wallet-only Lago residue, ledger/account/drift verification; and
paired NyxAgent list/turn/dispatch/revoke tests.

**#2274 done-when:** an authorized person chooses a Model Garden model in NyxBot,
uses it in a real conversation, and sees that model's usage charged at its own
configured rate on the bill. Demonstrate exact funding/settlement, no double
charge, and no private routing disclosure for that conversation. PR 1 alone does
not satisfy this end-to-end release gate.

## Official API references

Verified during the approved design on 2026-11-06; revalidate model-specific
availability and usage at adapter certification. Some Google links redirect to
Gemini Enterprise Agent Platform documentation while retaining the Vertex API.

- [Model Garden scope](https://cloud.google.com/vertex-ai/generative-ai/docs/model-garden/explore-models)
- [Gemini generateContent](https://cloud.google.com/vertex-ai/generative-ai/docs/reference/rest/v1/projects.locations.publishers.models/generateContent)
- [Gemini streamGenerateContent](https://cloud.google.com/vertex-ai/generative-ai/docs/reference/rest/v1/projects.locations.publishers.models/streamGenerateContent)
- [Gemini response and UsageMetadata](https://cloud.google.com/vertex-ai/generative-ai/docs/reference/rest/v1/GenerateContentResponse)
- [Claude on Vertex](https://cloud.google.com/vertex-ai/generative-ai/docs/partner-models/claude/use-claude)
- [Open MaaS calls](https://docs.cloud.google.com/gemini-enterprise-agent-platform/models/maas/call-open-model-apis)
- [Supported managed open models](https://cloud.google.com/vertex-ai/generative-ai/docs/maas/use-open-models)
- [Locations and global endpoints](https://cloud.google.com/vertex-ai/generative-ai/docs/learn/locations)
- [Vertex/Model Garden pricing](https://cloud.google.com/vertex-ai/generative-ai/pricing)
- [Claude cache semantics](https://cloud.google.com/vertex-ai/generative-ai/docs/partner-models/claude/prompt-caching)
- [429 and capacity handling](https://cloud.google.com/vertex-ai/generative-ai/docs/provisioned-throughput/error-code-429)

## PR 2: immutable tariffs and exact units

PR 2 introduced `billing:model-prices-v1`; PR 3 adds execution readiness, while
the paired assistant capability remains absent. Upgrade **all** API, worker and reader binaries and confirm the
existing drained-fleet readiness roster before authoring tariffs or allowances
using the new units. No new environment variable, price, model or rate is seeded.
The platform-model flag stays default-off; inference requires the PR 3 execution
capability across the fleet.

The `platform_model_tariffs` collection stores immutable economics. A lane's
`model_prices` contains model UUID + exact variant bindings, separate active and
pending tariff IDs, and derived allowance-unit metadata. Prices exist only in
the immutable revision. Ordinary catalog saves preserve bindings and recheck
against the live record transactionally; they cannot publish, forge or clear them.

Admin API additions (first-party humans only; admin writes, admin/operator reads):

- `GET /api/v1/admin/platform-models/{id}/tariffs`: latest 100 revisions.
- `POST /api/v1/admin/platform-models/{id}/tariffs`: create a pending revision.
  `revision` is the model's current revision (CAS); the new tariff consumes the
  next model revision. Read the model and tariff list after an ambiguous response.
- `POST /api/v1/admin/platform-models/{id}/tariffs/{tariff_id}/retire` with
  `{"revision": CURRENT_MODEL_REVISION}`: stop new selection, preserve history.
- The existing `/admin/billing/services/{service_id}/resync` retries publication;
  bounded billing reconciliation also retries pending revisions automatically.

The create body includes `revision`, `lane` (`byok` or `platform_key`), exact
`variant` (at most eight finite string selectors), `source` (private verification
reference), `contract`, `bounds`, and `components`. No caller-supplied Lago code
is trusted; codes are generated as:

```
platform_svc_<slug>_<pk|byok>_mdl_<model_uuid_without_hyphens>_t<revision>_<component_key>
```

The 255-byte limit and component-key uniqueness are checked without truncation.
Each component has `key`, `metric`, tagged `selector`, exact decimal
`credits_per_unit`, `explicit_free`, and `max_quantity`. Zero requires
`explicit_free: true`; missing is never free. Components allocate benefits in
lexicographic component-key order, independently of request or provider order.
A replacement never mixes components with its predecessor. Every charge and
matching exact rate-cache entry must synchronize before the binding moves.
Old charges and rates remain for accepted work and billing-period obligations;
there is no destructive tariff garbage collector. Retirement forbids admission,
not settlement, and never reuses a code.

A contract fixes the adapter version, a 64-hex certified `fixture_sha256`, family,
`tokens`, `cache_read`, `cache_write_ttls`, `split_reasoning`, input/output modality
splits, explicit Gemini `bill_tool_input`, `images`, and a distinct `requests` fee.
Versions are `openai.responses.v1`, `anthropic.messages.v1`, `gemini.generate.v1`,
and `open_maas.certified.v1`. Open MaaS has no generic OpenAI-shaped wire parser:
its adapter must certify the mapping to integer evidence. Certification and
stream-event accumulation are enforced by the serving adapters below.

Selectors are `{"kind":"input"}`, `output`, `reasoning`, `cache_read`,
`image_input`, `image_output`, `audio_input`, `audio_output`, `images`, `requests`,
`{"kind":"cache_write","ttl":"five_minutes"}` (also `one_hour`,
`thirty_minutes`, `twelve_hours`), and
`{"kind":"cached_modality","modality":"image"}` (or `audio`). The contract
derives the complete disjoint component set; missing, duplicate, overlapping,
or unexpected selectors fail authoring. A tariff uses image-token accounting or
per-image accounting, not both. Phase 1 rejects combined write-TTL/modality and
reasoning/output-modality splits without a certified intersection contract.
Gemini tool input can be aggregate-priced or excluded; separate tool-input
modality prices also require a future certified intersection contract. Supplied
Gemini modality arrays must sum exactly to their corresponding total.

Bounds include `max_input_tokens`, `max_output_tokens`, `max_candidates`,
`max_tool_iterations`, `max_images`, `max_request_bytes`, `max_response_bytes`.
Every component's prepaid bound must cover the full candidate/tool-loop ceiling.
Each bounded component is conservatively reserved together in the existing
funding transaction. PR 3 enforces those request limits **before dispatch**;
there is no byte-to-token estimate and no one-token cache admission. Unknown
variants, absent/stale/retired/mismatched rates and unavailable paid billing
refuse. A deliberate all-zero tariff remains metered without consuming benefits.

Strict evidence keeps missing, known zero and inapplicable distinct. Fractions,
negative values, overflow, inconsistent totals and incomplete required splits
refuse. Native integer counters and the normalized per-code vector are retained
on the coordinator with a payload digest; changed evidence on replay conflicts.
Only the coordinator contributes request/token totals. All component rows retain
the public model, distinct code, actual unit and frozen reservation rate.
Examples covered by tests:

1. OpenAI input 12,000, read 4,000, write 7,000 (3,000 at 30m + 4,000 at 12h):
   ordinary input 1,000, separate write classes, no aggregate write charge.
2. Claude ordinary 100, read 800, write 200 (150 at 5m + 50 at 1h): ordinary
   stays 100; input-related total is 1,100.
3. Gemini prompt 1,000, cached 200, candidates 100, thoughts 50, tool input zero:
   input 800 + cache 200, and either output 150 or output 100 + reasoning 50.
4. Gemini image input 300 of 1,000 and cache 200 without cached-image counts:
   split image/cache pricing refuses; aggregate input 800 + cache 200 is valid.

The additive metrics are `reasoning_tokens`, `image_input_tokens`,
`image_output_tokens`, `audio_input_tokens`, and `audio_output_tokens`. TTLs are
selectors on existing cache units. Generic lane editors cannot author these new
units: they require strict model evidence. Metric-matched allowances, exact
credits, grant ordering, wallet tax, the ledger and Lago residues remain shared.
Multiple TTL components of one unit share allowance availability within the same
transaction. Paid model admissions forbid overdraft and metering-only fallback.

`resolve_service_billing` exposes `model_prices` with active/pending IDs, exact
components and blocking reasons using the admission selector. Its ordinary class
view reports `model_tariff_required`, never a free or legacy fallback. Existing
usage readers and hourly/daily covering indexes already group by metric, code
and model; new units use their existing quantity measures and flat cost mirrors.
No financial backfill, historical rekey or new quantitative rollup field is needed.
UI/CLI unit labels are additive; route/native model identities are not exposed.

Missing final evidence preserves forwarded holds. Existing durable settlement
intent recovery, claim fencing, ledger/account reconciliation and wallet-only
Lago emission handle finalized vectors. Disabling the feature does not discard
those obligations. PR 3 supplies bounded adapter recovery and diagnostics
for provider-accepted work without recoverable final evidence. Missing evidence
never implies zero consumption, and not every provider supplies usage lookup.

## Admin, customer and CLI surfaces (PR 4)

Admin → **Platform Models** lists disabled drafts, active models and retired IDs
in stable cursor pages. Administrators edit the public contract, private same-model
routes, person/org/group access, and immutable tariffs; operators inspect them.
The UI is read-only while `inference:platform-models` is off for the signed-in
person. Backend administrative disable/revoke controls remain available during
rollout. Feature access never bypasses entitlement or execution readiness.
No page sends a provider probe or creates credentials.

Public IDs and families cannot be edited. Contract/route/access writes include the
model's `expected_revision`; tariff create/retire includes its current `revision`.
A 409 preserves the form and requires explicit reload/discard; there is no blind
retry. The enable switch remains unavailable while execution is not ready.
Readiness is reported independently of lifecycle and rate sync. Admin responses
return `execution_ready: false` and `blocking_reason: execution_fleet_not_ready`
until the registry, billing and execution fleet gates pass; once ready, the
blocking reason is empty. `required_readiness` contains those three capabilities,
not the separate assistant gate. Public rows retain their `retail_prices` and
report `availability: request_dependent` when that fleet is ready: each request
still needs supported inputs, a usable tariff, live authority and funding. Older
replicas' `unavailable` and `execution_not_implemented` values remain readable.

Access starts with default deny. Rules select people, organizations or groups,
with a searchable target picker or UUID input. Every matching deny overrides
allows, including All people. **Explain access for a person** performs the existing
live entitlement evaluation and reports policy, feature and model readiness
separately; it neither impersonates that person nor calls a model.

Price revisions show exact decimal-string credits per base unit, explicit free
components, TTL-qualified cache units, sync errors and active/pending readiness.
A new revision takes an administrator-supplied certified usage contract, fixture
digest and request bounds (advanced JSON fields), verification source, finite
public price variant, and editable components. No certification is invented by
the UI. **Publish / retry rate sync** calls the existing logical billing service
resync endpoint. Reconciliation also publishes pending revisions when all rates
are synced. Retirement is confirmed and revision-fenced. The history shows the
latest 100 revisions; the effective billing view reports the selected revision.

The customer **Platform Models** page is hidden with the flag off and makes no
model request. When on, it uses only `GET /api/v1/llm/models`, including its existing
`after`/`limit` pagination. The additive `retail_prices` field contains only public
variant, availability, and components (`metric`, `selector`, `credits_per_unit`,
`explicit_free`). Readers default absent prices to unavailable for old servers.
Selection uses the billing admission/explain selector over batched authorized
models and their platform-key tariffs. Missing/stale/disabled prices are unavailable,
never free or a legacy-rate fallback. Public DTOs omit tariff IDs, verification
sources, bounds, metric codes, private sync errors, billing-service bindings,
upstreams, routes, projects and regions. Price availability does not imply that
model execution is enabled. Price-variant labels are public metadata and must
never encode route identity.

Usage and execution records continue to use the public `nyxid/...` model string;
existing model breakdown/labels render it without substituting private native
model names. Administrative billing explanations link model tariff readiness to
its registry editor. The PR 4 surfaces add no separate inference path, assistant
model selection, new billing semantics or paid probe; execution uses the PR 3
gateway contract below.

### CLI

`nyxid models list [--limit 50] [--after nyxid/last-model]` lists entitled public
metadata and exact retail prices. `nyxid models show nyxid/model-name` follows
public pages to find that exact entitled model; unknown and inaccessible models
both report unavailable. Neither command accesses private administration routes,
and both JSON/table output use an explicit public-field projection.

Administrative commands require a human login and **model UUIDs**:

```sh
nyxid admin models list
nyxid admin models show MODEL_UUID
nyxid admin models create --file public-contract.json
nyxid admin models update MODEL_UUID --revision 4 --file mutable-contract.json
nyxid admin models retire MODEL_UUID --revision 5
nyxid admin models routes show MODEL_UUID
nyxid admin models routes set MODEL_UUID --revision 4 --file routes.json
nyxid admin models access show MODEL_UUID
nyxid admin models access set MODEL_UUID --revision 5 --file access.json
nyxid admin models access explain MODEL_UUID --user PERSON_UUID
nyxid admin models prices list MODEL_UUID
nyxid admin models prices create MODEL_UUID --revision 6 --file tariff.json
nyxid admin models prices verify MODEL_UUID
nyxid admin models prices publish MODEL_UUID
nyxid admin models prices resync MODEL_UUID
nyxid admin models prices retire MODEL_UUID TARIFF_UUID --revision 7
```

Manifests mirror the API bodies above, excluding revision (passed only by the
flag). Route files wrap `routes`; access files wrap `entitlements`; create/update
use their complete contract body. Price components use quoted decimal strings,
never floating-point numbers. `verify` is read-only and shows the same effective
billing/readiness selection; `publish`/`resync` use the existing billing resync
route. Writes never auto-retry a conflict or uncertain response. Future unknown
billing units retain their labels and exact prices in CLI output.

## PR 3 execution contract

`POST /api/v1/llm/gateway/v1/chat/completions` accepts a `nyxid/<model>` plus
`messages` and an explicit positive `max_tokens` or `max_completion_tokens`.
The gateway reserves the namespace before ordinary provider-name inference.
Stateless text and function tools, one candidate and at most one model
invocation per attempt are enabled. PR 3b adds the separately certified image
subset described below. `stream: true` returns Chat SSE. Native
caller protocols, uncertified images, audio, hosted tools, cache creation, provider state,
continuations and arbitrary passthrough fields are refused, not silently removed.
The registry may describe future capabilities; that does not enable an adapter.
Direct OpenAI Responses requires an output bound of at least 16; smaller bounds
are refused before admission, following its documented native minimum.

The ordinary entrance accepts personal sessions, access tokens and general API
keys. Live token/session/key authority, original and current connection/operation
ceilings, model entitlements, the offering policy and route credential version
are checked again after waits and before send. Actor-bound assistant, service
account, delegated, relay and execution-permission-enrolled transports are
refused here pending their private-model adapter; an HTTP approval cannot replace
an enrolled one-use permission fence. PR 5 must bind those internal occurrences
before enabling them. API-key checks use live enrollment resolution as well as
authentication hints, so a cold/stale process snapshot cannot bypass that fence.
Public discovery's `request_dependent` availability means
that execution checks the concrete request, configuration and funding; it is not
a provider health promise. Admin fleet readiness is also not such a promise.

The transport mappings are:

| Route | Native request | Certified evidence |
| --- | --- | --- |
| Vertex Google | `POST /v1/projects/{project}/locations/{location}/publishers/google/models/{model}:generateContent` (stream: `streamGenerateContent?alt=sse`) | `usageMetadata`; separate candidate/reasoning output |
| Vertex Claude | Same publisher path with `anthropic`, `rawPredict` / `streamRawPredict` | Messages usage, separate cache counters/TTLs; body `anthropic_version=vertex-2023-10-16` and no `model` field |
| Vertex Open MaaS | `/v1/projects/{project}/locations/{location}/endpoints/openapi/chat/completions` | Certified `openai/gpt-oss-120b-maas` mapping only; aggregate prompt/completion counts |
| Direct OpenAI | `https://api.openai.com/v1/responses`, `store=false` | Responses usage including cached input and reasoning subset |
| Direct Anthropic | `https://api.anthropic.com/v1/messages` | Same Messages usage contract as Vertex Claude |

Regional Vertex authorities are `{location}-aiplatform.googleapis.com`; global
uses `aiplatform.googleapis.com`. Typed validated admin fields construct every URL.
No user headers, query strings or destination selectors reach native endpoints.
The `gpt-oss-120b-maas` native ID is an exact reviewed equivalent of public
`nyxid/gpt-oss-120b`; no generic `-maas` alias rule exists. Its documented
`max_tokens` bound and final SSE usage are used without assuming OpenAI-only
options or cache/reasoning counters. Other MaaS models need reviewed fixture
certification before they can execute.
Each tariff's `fixture_sha256` must equal the SHA256 of its checked-in fixture in
`backend/src/services/platform_model_execution/fixtures/`. Admins must certify the
real model's native context/output limits; admission reserves the entire declared
native context, never a byte-to-token guess. The explicit output bound is sent
to the provider. Request and response bounds are enforced before send/during
read, capped at 1 MiB and 8 MiB. No variant-specific contract executes yet.

GCP auth signs RS256 service-account assertions for the cloud-platform scope and
posts only to `https://oauth2.googleapis.com/token`. JSON-supplied URLs are checked
as metadata and never fetched. Cache keys bind credential identity, ciphertext
and version; rotation cannot reuse an old token. The cache has 128 entries,
single-flight minting per key, at most eight concurrent token requests, a
10-second mint timeout, 60–89 seconds of early refresh and two-second suppression
after failure. Only idle entries can be evicted. Credential/token strings use
zeroizing storage and are never logged, audited or placed in errors.
Bearer syntax is checked before funding; selecting service-account JSON for a
direct route cannot forward its private key to that provider.

One private planner samples routes by priority then weight, at most four attempts.
Cooled routes do not spend this attempt budget or hide a later healthy route.
The dispatch/read deadline is 120 seconds (approval waiting precedes it). Existing
cluster leases bound each project/location/model target to eight concurrent calls
and two per person, plus any offering-specific limit; a process also caps each
target at four. Lease loss cancels work. Cooldowns are credential-version scoped,
bounded and jittered; no independent HTTP retry layer resends inference.

Fallback requires a proven connection failure before send or a bounded,
adapter-certified authentication/permission/rate-limit error envelope with its
matching 401/403/429 status. A status alone, arbitrary proxy error page, 408, 5xx,
redirect, timeout or broken accepted body never permits fallback. Once successful
headers commit a route, neither output failure nor cancellation can switch it.

All tariff components reserve atomically through the PR 2 funding transaction.
A snapshot/majority transaction marks the full meter vector forwarded and creates
one private attempt marker before provider I/O. Final normalized evidence is
persisted as one digest-fenced component intent before success or `[DONE]` reaches
the caller. Existing settlement/reconcile handles exact benefits, wallet tax,
ledger postings and wallet-only Lago quantities, including provider retry and
cumulative-floor residue. No native routing identity enters those records.

Streaming uses bounded SSE framing, cumulative snapshots (never sums snapshots),
sequence duplicate checks where the protocol provides them, and final-usage
validation. Text may be delivered incrementally; function arguments are released
only when complete. Native identifiers, headers, reasoning state, signatures and
error bodies are omitted. Cancel/disconnect drops transport. Authoritative final
usage already received still settles; absent, malformed, truncated or
out-of-contract evidence leaves a hold with `model_usage_pending`.

`platform_model_attempts` is private recovery metadata, not another billing
system. It retains the immutable routing revision/route reference and a
credential-version digest for reviewed investigation; these never enter public
usage or response DTOs. The existing reconcile tick CAS-claims at most 100 expired attempts,
recognizes persisted component intents and otherwise flags `pending_review` in
Integrity. These stateless providers do not guarantee a recoverable usage lookup:
there is no inference redispatch, guessed charge, automatic zero or timed hold
release. Operators must obtain authoritative evidence or authorize a reviewed
write-off; PR 3 does not expose an automated write-off endpoint. Prompts,
completions and raw provider errors are never retained in recovery rows.

The MaaS model ID, output parameter and aggregate usage shape were rechecked on
2026-11-07 against [the gpt-oss model card](https://docs.cloud.google.com/gemini-enterprise-agent-platform/models/maas/openai/gpt-oss-120b)
and [the request/SSE examples](https://docs.cloud.google.com/gemini-enterprise-agent-platform/models/maas/call-open-model-apis).
The committed fixtures are synthetic examples of those documented shapes; they
do not claim a live provider canary.

The direct Responses output bound, terminal SSE and usage fields were checked
against [OpenAI's Responses reference](https://developers.openai.com/api/reference/resources/responses/methods/create).

## PR 3b: certified raster image input

The ordinary stateless Chat entrance also accepts ordered **user** `text` and
`image_url` parts, where `image_url.url` is a raster **data URI** and optional
`detail` is `auto`. System/developer instructions, assistant messages and tool
results remain text/function-only. Remote URLs, paths, provider file IDs,
unknown content fields and other detail modes are refused locally. NyxID never
fetches an image URL, resizes an image, selects an animation frame, or drops a
part. Existing tool-call/result correspondence and public response projections
are unchanged. This does not enable assistant calls: the PR 5 actor-bound guard
and attachment-authority work remain separate.

The closed certification lookup binds an exact native model, transport,
accounting version and immutable fixture digest. The initial cohort is:

| Native model | Transport | Image accounting |
| --- | --- | --- |
| `gemini-2.5-flash` | Vertex Google `inlineData` | Aggregate input/cache, or exact `ImageInput` and `CachedModality(Image)` partitions |
| `claude-sonnet-4-5` | Vertex and direct Anthropic base64 image blocks | Aggregate input/cache only |
| `gpt-5` | Direct OpenAI Responses `input_image` | Aggregate input/cache only |
| `gpt-oss-120b-maas` | Vertex MaaS | Text only; images refused |

No registry metadata alone certifies an adapter. Unknown models or transports
fail closed. Existing text fixture files and hashes are unchanged; historical
text tariffs retain their economic identity. New image tariffs use the SHA-256
of the corresponding `*-images-v1.json` fixture. Claude uses the SHA-256 of the
**concatenated bytes of `claude-vertex-images-v1.json` followed by
`claude-direct-images-v1.json`**, binding both same-model transports to one tariff.
All files live under `backend/src/services/platform_model_execution/fixtures/`.
They contain sanitized synthetic request/response/SSE examples, including a
missing-evidence response and a certified zero-consumption refusal. They are
protocol regression fixtures, not evidence of a live canary or account access.
No credentials, prices or models are seeded.

Before reserving image decode capacity, decoding, image execution or image-tariff
authoring, the fleet must additionally pass `inference:platform-model-images-v1`.
The gateway checks after flag/entitlement authorization and before image parsing;
the planner and dispatcher recheck readiness. Existing platform flag, entitlement,
registry, billing and execution gates remain required. Old replicas cannot send
an image under a text digest; do not author/activate the new image contracts until
all serving replicas advertise readiness. Reads and disabling remain available.
No new persisted field or environment variable is required.

Validation covers the whole request/history: PNG, JPEG and static WebP only,
byte-sniffed against the declared MIME type, at most 10 images, 8 MiB/file,
16 MiB total file bytes, 32 million pixels/image and 64 million pixels total.
Claude further limits each file to 5 MiB and either dimension to 8,000 pixels.
`tariff.bounds.max_images` can lower the count. Each raster is fully decoded and
discarded in sequence, with a 128 MiB decoder allocation limit and checked output
size; malformed/truncated inputs, animation and multi-image JPEG containers are
refused. A 384 MiB process-wide budget covers transient decode work, weighted at
twice the serialized request size plus the 128 MiB decode and 8 MiB file bounds.
Each authenticated person gets at most one active decode and at most half that
budget. Two people with requests at the legal file/JSON ceilings can decode
concurrently. Exhaustion returns `ServiceConcurrencyLimited` (429), without a
queued wait or spending. CPU decoding runs on the blocking pool after acquiring
the permit; cancellation leaves it with the decoder until that work exits.
Both the byte reservation and person slot are released before parsing returns,
including on failure. `Request` and its clones hold no decode permit during
human approval, funding, provider I/O or streaming. Retained request bodies keep
the existing ingress/tariff size bounds; there is no separate dispatch permit.

Non-image content remains capped at 1 MiB. The existing configured LLM body limit
is enforced by bounded ingress before JSON allocation; the platform entrance
also enforces 32 MiB. The tariff, configured ingress limit and platform ceiling
bound the actual serialized request and translated native body. Public
`retail_prices[].image_input`, when present, reports the intersected limits and
`included_in_input_tokens` or `exact_image_and_cache_partition`. It contains no
route, project, credential, tariff ID or native model information. Absent means
no certified executable image contract has been established for that caller.

Image-token billing uses existing selectors and complete immutable tariffs.
`contract.images` / `Images` still mean generated-image charges and are not used
for image input. When Gemini reports prompt 1,000, image 300, cached 200 and
cached-image 100, the exact partition is ordinary input 600, image input 200,
ordinary cache 100 and image cache 100. A missing intersection cannot be guessed;
a split tariff retains the unsettled obligation. Aggregate tariffs include image
tokens in their aggregate input/cache quantities and advertise that fact.
No pixel-based billable estimate, base-price fallback or extra image fee is added.

All candidate routes must satisfy the entire image/tariff contract before
credential materialization or admission. Fallback remains limited to proven
unsent requests or certified zero consumption. Atomic allowance/grant/wallet
admission, wallet tax, immutable usage evidence, ledger, Lago residues and recovery
use the existing coordinator. Missing usage, interrupted streams and cancellation
after send retain their forwarded obligations; they cannot become zero usage.

Protocol references: [Gemini image understanding](https://cloud.google.com/vertex-ai/generative-ai/docs/multimodal/image-understanding),
[Anthropic vision](https://platform.claude.com/docs/en/build-with-claude/vision),
and [OpenAI image inputs](https://developers.openai.com/api/docs/guides/images-vision).
