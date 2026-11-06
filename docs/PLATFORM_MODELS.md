# Platform models

This is the approved platform-model contract for
[#2274](https://github.com/ChronoAIProject/NyxID/issues/2274), including the owner's
public-name, credential and first-release scope decisions. It supersedes the
design's earlier brand-neutral naming and keyless-first recommendations.

**Current implementation: PR 1, registry/readers/authorization.** It stores configuration and evaluates
live entitlement metadata. It does **not** execute inference, mint Google tokens,
decrypt route credentials, publish model prices or modify NyxAgent selection.
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
`billing_service_id`. Only an active model can be enabled. PR 1 refuses activation
because complete execution and billing readiness are not yet implemented.
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
secret or arbitrary pool. The later executor constructs trusted authorities and
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

Credential material stays in the existing encrypted catalog field. PR 3 will
validate a GCP key strictly as `service_account`, reject executable/external/file
sources and untrusted token URLs, and mint/cache short-lived tokens using trusted
Google authorities. ADC/WIF is deferred behind the token-provider abstraction.
This foundation does not claim that merely storing JSON certifies a GCP key.

## Deployment and readiness

`inference:platform-models` is a code-declared, default-off flag using existing
person/org/global overrides. No new environment variable is introduced.

PR 1 advertises only `inference:platform-model-registry-v1`. It deliberately does
not advertise `inference:platform-models-v1`, `billing:model-prices-v1` or
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

Later PRs add complete model tariffs and exact usage classes, bounded admission
and the four serving families, admin/user UI and CLI, and the paired NyxAgent
contract. Prices never fall back silently to legacy lanes. No execution is
unlocked by this PR's metadata API or a manually inserted flag. Ordinary services,
pools and billing retain their existing paths with the flag off.

## Approved execution contract (subsequent PRs)

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
boundary. The metadata plan from PR 1 is not an execution permit: later
credential materialization requires a server-constructed, actor-bound permit for
the live model, route/revision, credential, operation and funded admission.

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
| 4 | Admin model/route/access/rate tools; public discovery, CLI and billing/execution labels. |
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
