# LLM billing and usage tracking assessment

Reviewed 2026-10-15 against NyxID commit `821e5f80` (backend package version
`0.70.1`) and the official provider pages linked below.

This is a source-code and public-documentation assessment. It does not establish
which prices, feature flags, provider deployments, or Lago charges are currently
active in production, or whether production traffic follows every documented path.
No billing configuration or application behavior was changed.

This assessment describes the baseline before implementation. The subsequent
additive metering implementation, compatibility boundaries, retained invocation
APIs, and recovery behavior are documented in [LLM invocation metering](LLM_USAGE_METERING.md).

## 1. Current industry standards

### Usage is measured per model invocation

Major providers return token usage alongside inference results. A proxy can inspect
those response metadata fields while forwarding JSON or streaming events, without
retaining the prompt or generated content. This is the appropriate metering point
for NyxBot: one user turn can contain many model invocations, tool iterations,
specialist calls, retries, and utility calls. Counting user messages, final reply
length, or network bytes does not measure the underlying provider consumption.

Request token counting is useful for budget admission. Provider-reported final
usage is the preferred source for settlement. Local tokenization cannot reliably
discover actual cache hits, hidden reasoning, provider-added processing, or all
multimodal usage. Final usage can be unavailable when a stream is interrupted;
that must be represented as partial or unknown usage rather than an exact zero.

### Prices distinguish token classes and execution conditions

Public rate cards commonly quote currency per million tokens, with separate rates
for ordinary input, output, and cached input. Cache writes are an additional class
where supported. Model/version, context length, modality, region, processing tier,
and deployment type can also change the price. These dimensions are provider- and
model-specific; there is no single universal LLM token price.

| Provider | Usage and charging conventions | Implication for NyxID |
| --- | --- | --- |
| OpenAI | Chat Completions uses `prompt_tokens`, `completion_tokens`, and `prompt_tokens_details`; Responses uses `input_tokens`, `output_tokens`, and `input_tokens_details`. Cached input is a subset of input. Current documentation for GPT-5.6 and later includes `cache_write_tokens` as another input subset and a separate cache-write price. Reasoning is included in the relevant output total; detail fields must not be added a second time. | Support both API shapes, subtract cache reads and writes from ordinary input, and select rates by model and execution tier. Older models must retain their own cache semantics. |
| Azure OpenAI / Microsoft Foundry | Standard deployments charge input, cached input, and output; newer model families also expose and charge cache writes. The current prompt-cache documentation shows `usage.prompt_tokens_details.cache_write_tokens`. Global, Data Zone, and regional deployment options differ. Batch has a separate discounted offer. Provisioned Throughput Units (PTUs) are purchased capacity, not a per-request token invoice. | Use OpenAI-compatible usage extraction plus Azure deployment/model metadata. Track token consumption for PTU traffic, but distinguish allocated capacity cost from marginal token cost. Do not assume a public Standard price explains a PTU invoice. |
| Google Vertex AI / Gemini | Native `usageMetadata` includes `promptTokenCount`, `candidatesTokenCount`, `cachedContentTokenCount`, `thoughtsTokenCount`, `toolUsePromptTokenCount`, and `totalTokenCount`. Cached tokens are included in the prompt total. Current price tables identify output as response plus reasoning. Explicit context caches also incur storage charges over time; implicit caching has no storage charge. | Charge ordinary input separately from cache reads and include billable thinking in output. Retain tool-use and modality details. Explicit-cache lifecycle/storage requires metering beyond a generation response. |
| Anthropic Claude | `input_tokens` excludes `cache_read_input_tokens` and `cache_creation_input_tokens`; those are separate input classes. `cache_creation.ephemeral_5m_input_tokens` and `ephemeral_1h_input_tokens` split cache writes by lifetime. The documented write multipliers are 1.25× ordinary input for five minutes and 2× for one hour; read multipliers have model-specific exceptions. | Do not subtract cache tokens from Anthropic's ordinary input. Preserve the write-lifetime split when passing through provider economics. |

Official sources: [OpenAI prompt caching][openai-cache], [OpenAI pricing][openai-pricing],
[OpenAI reasoning usage][openai-reasoning],
[Azure prompt caching][azure-cache], [Azure pricing][azure-pricing],
[Google response usage metadata][google-usage], [Google pricing][google-pricing],
[Google context caching][google-cache], and [Anthropic prompt caching][anthropic-cache].
The Google URLs redirected to Gemini Enterprise Agent Platform branding when fetched.

**A useful charging equation**, with mutually exclusive quantities, is:

```text
charge = ordinary_input × input_rate
       + output_including_billable_reasoning × output_rate
       + cache_read × cache_read_rate
       + cache_write_5m × cache_write_5m_rate
       + cache_write_1h × cache_write_1h_rate
       + other explicitly priced usage
```

Use a single cache-write term for providers without a lifetime split. Convert
per-million published rates to per-token rates before multiplication. Keep currency,
NyxID credits, and any credit-to-currency conversion explicit.

For example, an inclusive input report of 12,000 tokens with 8,000 cache-read tokens
and 2,000 cache-write tokens means 2,000 ordinary input tokens. Charging all 12,000
at the ordinary input rate and then adding the cache charges would overcharge.
For Anthropic, an `input_tokens` value of 2,000 plus those same separate cache
counts already means 2,000 ordinary input tokens.

### Usage, customer charges, and provider cost are different records

Record consumption even when a request is free to the customer or uses BYOK.
Calculate customer charges from NyxID's published selling prices. Track provider
cost independently, using the actual provider account, model, deployment, contract,
and discounts. Allowances and promotional grants change who funds the customer
charge; they do not reduce the provider's consumption.

Azure Monitor provides operational request/token and PTU utilization metrics
([monitoring documentation][azure-monitor]). Operational metrics and response usage
are useful for reconciliation, but provider billing records remain necessary to
explain actual supplier spend, discounts, capacity, storage, and other charges.
Provider exports may only support aggregate matching; request-level reconciliation
should be claimed only where the provider supplies that granularity.

Exact published rates should be captured with effective dates rather than copied
into permanent business logic. Azure's fetched pricing page rendered numeric prices
as `$-`, so this assessment deliberately makes no claim about exact Azure dollar
rates. Public list prices also do not establish our contracted supplier costs.

## 2. Where NyxID is now

### The requested four charge classes already exist

[Service billing models](../backend/src/models/service_billing.rs) already define
`input_tokens`, `output_tokens`, `cache_read_tokens`, and `cache_write_tokens`, as
well as total tokens, requests, bytes, images, and voice seconds.

Each service has independent optional BYOK and platform-key pricing lanes. A lane
has a primary price and additional metric components. The selected lane's synced
primary and synced components produce independently funded usage rows and Lago
events. Total `tokens` is charged only if explicitly configured. A four-class LLM
price can therefore be expressed today as:

```json
{
  "platform_key_pricing": {
    "metric": "input_tokens",
    "credits_per_unit": "<input price per token>",
    "components": [
      { "metric": "output_tokens", "credits_per_unit": "<output price per token>" },
      { "metric": "cache_read_tokens", "credits_per_unit": "<cache-read price per token>" },
      { "metric": "cache_write_tokens", "credits_per_unit": "<cache-write price per token>" }
    ]
  }
}
```

This is a structural illustration, not a complete API payload or a proposed live
rate change. Sync state and metric codes are server-owned. Configuring a total-token
charge alongside these classes would be an additional charge, so that choice needs
to be deliberate.

The lane selection and sync behavior are documented in
[Platform keys and inference](PLATFORM_KEYS_AND_INFERENCE.md#billing-lanes-and-durable-accounting)
and implemented in [billing route context](../backend/src/services/billing/route_context.rs).
An unsynced primary retains whole-lane legacy/free fallback; an unsynced additional
component is free until synced. A missing matching lane is free when lane mode is
configured. These are material rollout considerations.

### Current capture and normalization

[LLM usage service](../backend/src/services/llm_usage_service.rs) extracts common
OpenAI Chat/Responses, Anthropic, and Gemini JSON fields. It aggregates supported
SSE usage snapshots/deltas and deduplicates completed image events; realtime
collection also handles response identities. Shared paths feed `PlatformUsage`.

The implementation already distinguishes provider accounting from priced quantities:

- `TokenBreakdown` preserves prompt, completion, cache-read, and cache-creation
  values according to the provider's convention.
- `ReportedLlmUsage::apply_to` subtracts cache counts from inclusive input reports
  and leaves Anthropic's already-exclusive ordinary input unchanged.
- Audio and image token subsets are retained for observability; they are not
  independently priced token classes today. Images and voice seconds are separate
  supported billing metrics.
- Missing reported usage falls back to approximately `ceil(bytes / 4)` for legacy
  total tokens, with a minimum of one. The input/output/cache quantities remain
  zero when those fields are unavailable. That estimate cannot recover cache classes
  or hidden reasoning.
- Token capture considers `llm-` slugs and configured token-family metrics on
  either lane, including unsynced components. Merely adding inference metadata
  is not the complete capture-capability contract.
- Forced `stream_options.include_usage` currently uses a fixed service-slug list.
  An arbitrary Azure or custom compatible service is not automatically covered.

Existing tests include OpenAI cached input, Anthropic cache reads/writes and
streaming, Gemini JSON cache normalization, split snapshots, image subsets, and
deduplication. These were inspected; this assessment did not run application tests.

### NyxBot's underlying model calls already have a metering path

The [NyxAgent engine contract](chat/08-nyxagent-engine.md#per-conversation-credential)
states that NyxAgent calls its model through `/api/v1/proxy/s/{model service}` using
the thread's system-managed Agent Key. Examples are `chrono-llm-public` and
`llm-deepseek`. This provides the right boundary to capture each inference call,
including specialist calls, when the deployment follows that contract.

The initial `llm-nyx` orchestration hop is separately classified through the shared
proxy. It must not be treated as a second copy of the underlying model token cost.
Direct Chrono-LLM chat also uses the shared proxy and requests streaming usage.
Assistant title/learning/voice utility inference uses a metered one-shot path.

`UsageMeterRow` already stores actor, billing owner, API key, service/connection,
model, metric, transaction identity, quantity, funding, and settlement state.
Hourly rollups also retain model. It does not have explicit assistant conversation,
turn, agent, provider-request, or general usage-quality fields. API-key attribution
is a useful starting point, but durable per-turn reporting needs an explicit
correlation contract that survives credential rotation/revocation and raw-row retention.

### Accounting is already durable and exact

NyxID performs admission and records reservations before forwarding. Final usage
is persisted, component settlement is recoverable, and transactions/events have
stable identities. Funding applies allowance units, then expiring grants, then the
wallet. Platform-key execution charges the acting person's wallet; org BYOK can
retain org-wallet attribution.

Money uses checked integer picocredits and MongoDB Decimal128, with up to twelve
decimal places. Settlement freezes rates, posts balanced journal entries, and
reconciles wallet/grant accounts; hash chains provide tamper evidence. Lago handles
plan charges, wallet integration, usage events and invoicing. NyxID now authors
service prices and performs local funding/rating calculations, so the old ADR's
description of Lago owning all pricing logic is insufficient as a description of
the current implementation. See [exact accounting](BILLING_EXACT_ACCOUNTING.md).

The present model-rate support is **partial**, rather than absent: `BillingRateCache`
and settlement lookup support a model-specific entry, falling back to a wildcard.
However, admission calls `fresh_rate(..., None, ...)`, the normal Lago plan-rate
refresh populates wildcard rows, and the service lane editor authors a service-wide
rate per component. That is not an end-to-end model/deployment/context/tier rate card.

### Concrete gaps and their effects

| Gap verified in source | Current effect | Priority |
| --- | --- | --- |
| Cache-write extraction only checks Anthropic's `cache_creation_input_tokens`, not OpenAI/Azure `input_tokens_details.cache_write_tokens` or `prompt_tokens_details.cache_write_tokens`. | Newer compatible-provider writes are not charged as writes; they remain in ordinary input and a configured write component receives zero. | Immediate |
| Gemini output extraction uses `candidatesTokenCount` without `thoughtsTokenCount`; `toolUsePromptTokenCount` is also not extracted. | Separate output pricing misses billable thinking, and the normalized classes do not explain all reported total tokens. An explicit total can still be retained for legacy total-token pricing. | Immediate |
| Shared SSE event acceptance recognizes specific event names or OpenAI-style explicit totals; `has_explicit_total` lacks `/usageMetadata/totalTokenCount`. | A native Gemini `data: {"usageMetadata": ...}` frame with no recognized event name can be discarded even though the JSON extractor recognizes its fields. Translated OpenAI-compatible streams are a different path. | Immediate for native Gemini |
| Anthropic cache writes are collapsed to one quantity. | Five-minute and one-hour writes cannot carry distinct rates in the same invocation under the current metric uniqueness rules. | Before offering mixed TTL pass-through pricing |
| Usage classes default missing fields to zero and ordinary meter rows lack durable completeness/provenance. | Missing final output usage can look like zero output; legacy byte estimates are not clearly separated from reported usage in the durable meter contract. | Immediate |
| Streaming usage opt-in is slug-based. | A new compatible service can stream successfully without requesting the final usage chunk. | Immediate |
| Inference discovery's protocol enum offers Anthropic Messages and the two OpenAI protocols, with no native Gemini protocol. | Recognizing Gemini JSON fields does not establish complete native Vertex/Gemini model discovery, request construction, and streaming support. OpenAI-compatible gateways can use their own supported shape. | Before offering native Vertex inference |
| Rate authoring/refresh/admission are service-wide, despite partial model lookup support. | Different models on the same gateway cannot reliably receive different fully supported rates and reservation bounds. | Next |
| Generic token reservations use request-byte estimates; the bounded native-text path is stronger. | A short prompt with a long answer or reasoning budget can exceed the reserved output amount. Existing holds do not by themselves guarantee a full-call exposure bound. | Next |
| No explicit-cache resource/time meter or supplier-cost reconciliation contract was found in the reviewed billing path. | Generation token accounting alone cannot explain Google explicit-cache storage, Azure PTU spend, or all provider invoice differences. | Before claiming full cost alignment |

These are source-level findings, not measurements of production underbilling.

## 3. Recommended alignment plan

### Stage A: complete the four-class path and measure coverage

Keep the existing wallet, funding, ledger, and Lago infrastructure. Harden the
existing usage adapters before changing live rates:

1. Add OpenAI/Azure cache-write field variants, including nested Responses event
   envelopes. Normalize inclusive input as `input - cache_read - cache_write`.
   Preserve provider-reported totals and check inconsistent counts rather than
   silently treating a clamped quantity as fully trustworthy.
2. Add native Gemini SSE recognition and explicit thinking/tool-use capture.
   For the documented native Gemini shape, output should include candidates plus
   billable thoughts. Keep tool-use input separately visible and map it according
   to the applicable provider SKU. Do not infer that every unexplained total-token
   difference is output.
3. Select usage adapters and streaming options from explicit service capabilities:
   protocol, usage shape, snapshot/delta semantics, cache inclusion, and terminal
   usage behavior. Azure deployment details belong alongside provider metadata;
   native Gemini needs an appropriate protocol contract rather than an OpenAI flag.
4. Persist usage source (`provider_reported`, `estimated`, `unknown`), completeness
   (`complete`, `partial`, `missing`), adapter version, and terminal outcome.
   Preserve absent versus explicitly zero fields. Provider-request and response
   identities should be retained when available. Persist only allowlisted metadata,
   not provider bodies, prompts, secrets, or arbitrary reflected identifiers.
5. Define the commercial policy for incomplete usage. Prefer pending reconciliation
   or absorbing the unknown portion for an initial rollout. Any estimate charged
   to customers must be disclosed and later correctable; never fabricate separate
   cache or reasoning quantities from byte counts. Release/retain holds according
   to that explicit policy without indefinite reservation leakage.
6. Configure the four separate lane prices for verified providers. Inspect every
   component's sync state, explicitly review legacy/resale charges, and prevent
   accidental total-token plus component charging. Capture usage for free/BYOK
   calls independently of whether a paid lane is enabled.

Validate with provider fixtures containing nonzero input, output, cache reads and
writes; Anthropic's exclusive inputs; Gemini thinking and native SSE; missing versus
zero fields; chunk-boundary splits; duplicate cumulative snapshots; cancellations;
retries; and crashes during component settlement. Every provider invocation and
each charge component must settle at most once. A retried invocation that consumes
additional provider resources is a distinct attempt, with an explicit customer
charging policy.

### Stage B: make pricing and budgets model-aware

Build on the existing model-keyed cache seam, but complete authoring, lookup,
admission, settlement, synchronization, and historical reporting together.

- Resolve a versioned rate card using provider/service, resolved model/version,
  credential lane, deployment/region, processing tier, modality, and context band
  where relevant. Preserve both requested and resolved model identities when they
  differ. Do not blindly trust a client-supplied alias for rating.
- Author separate customer selling prices and supplier cost schedules. Include
  currency and the exact credits conversion. Freeze the applicable customer rate
  version at admission; delayed settlement must not pick up an unrelated later
  price. Verify that wallet settlement and Lago invoice charges use the same rate
  contract, including requests crossing price changes.
- Reserve against a bounded output/reasoning allowance and a conservative input
  estimate. Count cache creation at the possible write rate when warranted; do
  not assume a cache hit will happen. Apply turn/agent budgets across all underlying
  attempts, including specialists and utility inference according to product policy.
- Extend write pricing for Anthropic's five-minute/one-hour split only when needed.
  Use distinct quantities or a well-defined price dimension; one blended write
  quantity cannot reproduce both provider rates exactly.
- Add independent modality prices when the product offers them. Existing audio/image
  subset observation is useful, but those subsets must partition an inclusive token
  total rather than create duplicate charges.

### Stage C: explain NyxBot cost and reconcile supplier spend

Propagate server-verified `conversation_id`, `turn_id`, `agent_id`, parent invocation,
and attempt identity from NyxID to NyxAgent and back through model requests.
Do not let arbitrary caller headers assign costs to another turn or owner.
Aggregate model calls into a durable turn cost view without counting the same
usage again at the orchestration boundary. Keep utility inference identifiable so
its attribution is an explicit product decision.

Expose ordinary input, output, cache reads, cache writes, reported/partial coverage,
gross customer charge, allowance/grant funding, wallet debit, supplier cost, and
cost variance. A turn can use multiple models and providers, so retain those
dimensions in the drill-down. High-cardinality turn data should use a bounded
detail/summary store, not automatically expand every existing hourly rollup index.

For Google explicit caching, collect cache resource creation, token size, expiry,
TTL changes and deletion, then meter token-time using the supplier's storage rules.
Reconcile with provider billing records because actual storage lifetimes and billing
granularity may differ from local estimates. For Azure PTUs, report consumption
alongside capacity utilization and an explicitly labeled allocated supplier cost.

Compare captured quantities and estimated supplier cost against provider usage and
billing exports by account, deployment, SKU/model, region and time window. Separate
free-tier discounts, contractual discounts, capacity, cache storage, provider tools,
taxes and exchange rates from usage mismatches. Reuse existing billing integrity
reporting for local ledger checks, while adding a distinct supplier-reconciliation
view; local money conservation does not prove the provider invoice agrees.

### Rollout and acceptance

Start with observed OpenAI-compatible/Chrono-LLM and Anthropic NyxBot traffic, then
native Gemini/Vertex and Azure deployment variants. Shadow-record corrected usage
and proposed supplier/customer costs before applying new rates. Publish the new
rate schedule, enable a small cohort, and watch missing-usage rates, stream
interruptions, rate misses, duplicate settlements, and supplier variance.

Completion means: normal successful calls have complete provider-backed class
counts; incomplete calls are visible and follow the agreed charging policy;
price changes and retries cannot double-charge; reservations bound the admitted
work; each NyxBot turn can explain its underlying invocations and charges; and
provider totals reconcile within documented scope, timing, and granularity.
Historical rows without the new details must remain explicitly unknown; do not
invent a backfilled cache, thinking, or provider-cost split.

## 4. Required capability: assign cost to specific usage

The clarified product requirement is to attribute provider consumption to an end
user and optionally pass the relevant supplier costs through to that user. A
supported category must work through **measurement, rate selection, exact cost
calculation, funding/settlement, and an explainable charge record**. A field that
only appears in analytics does not satisfy this requirement.

### Current capability matrix

| Usage category | Independently priceable today? | Qualification |
| --- | --- | --- |
| Ordinary input tokens | Yes | Provider normalization and capture must succeed; rates are authored per service/lane. |
| Output tokens | Yes | Gemini thinking is currently missing from the normalized output quantity. |
| Cache-read tokens | Yes | Common provider fields are supported, subject to streaming coverage. |
| Cache-write tokens | Yes, as one class | Anthropic aggregate writes are captured; newer OpenAI/Azure write fields are missing. |
| Five-minute versus one-hour cache writes | No | They are collapsed into one class; duplicate metrics in a lane are rejected. |
| Reasoning tokens as a visible subtype | Not independently represented in the shared breakdown | OpenAI's inclusive output total can already charge them; a separate reasoning charge must partition that total rather than add to it. |
| Audio input/output tokens | No | Existing fields are analytics subsets, not independent price components. Voice seconds are independently priceable. |
| Image input/output tokens | No | Existing fields are analytics subsets. Generated-image counts are independently priceable. |
| Requests, transferred bytes, images, voice seconds | Yes | Actual capture semantics and supported paths still depend on the service. |
| A declared service operation | Yes, per request | Operation prices require a Requests primary and replace its rate. They do not count provider-hosted tool executions inside one model response. |
| Provider-hosted searches, tool calls, sessions, or storage | No general itemized contract | Each billable item requires a trustworthy usage extractor, defined unit, and rate mapping. Counting one HTTP request does not establish its internal usage. |
| Explicit-cache storage token-time | No | Requires resource lifecycle and duration measurements. |
| Rates by model, tier, modality, region, and context band | Partial foundations only | Model lookup exists; end-to-end authoring, admission and synchronization are incomplete. |
| Supplier cost and user price for the same measured item | Not a complete dedicated contract | Existing platform/resale layers are charge layers; they should not be relabeled as supplier cost without implementing that semantic distinction. |

### Target charge item

Each item must retain enough information to reconstruct its price without using
today's mutable rate configuration:

| Information | Required meaning |
| --- | --- |
| Invocation and ownership | Internal invocation/attempt identity, actor, billing owner, service/connection, credential class; verified agent/conversation/turn references where applicable. |
| Provider identity | Provider account/deployment reference, requested and resolved model, provider request/response identity when supplied. Never a raw credential. |
| Measured quantity | Unit, usage class, quantity, provider source field, adapter version, complete/partial/missing state. Unknown quantity is distinct from zero. |
| Applicable dimensions | Input/output direction, text/audio/image modality, uncached/read/write class, write TTL, processing tier, context band, region or operation as needed by the provider's price. |
| Supplier valuation | Versioned supplier rate, denomination, currency, effective time, computed cost, and reconciliation state. Public-list-price estimates remain labeled as such. |
| Customer valuation | Versioned selling-price/pass-through policy, applicable conversion and fee, exact gross credit charge, and allowance/grant/wallet funding. |
| Lifecycle | Admission, completion, settlement, corrections, and stable deduplication identities. Corrections reference original items through auditable adjustments. |

These are logical requirements, not a mandate to duplicate all metadata on each
MongoDB row. A durable invocation record and immutable price references can supply
shared context while the existing component rows retain funding and settlement.

### Priceable dimensions and overlap rules

Token direction, modality and cache class can intersect. For example, cached audio
input may have a different rate from cached text input. Independent marginal
totals for audio tokens and cached tokens do not reveal their intersection. If a
provider does not report enough information for the intended rate, mark the cost
unresolved or use an explicitly published supported blended rate. Do not invent
the missing split.

Every billable quantity must belong to one defined price bucket, unless a separate
fee is explicitly additive. Reasoning may be shown separately while sharing the
output rate. A reasoning-detail count already included in output must not create
a second charge. The same rule applies to modality subsets and cache quantities.
Provider-reported totals remain useful as validation/control totals.

Prices should support a clear denomination, such as credits per token, currency
per million tokens, credits per image, or currency per token-hour. Calculation
must preserve exactness within the supported precision. Unsupported precision or
units must be rejected or explicitly converted under a documented policy, never
silently rounded per request. The existing exact-credit primitives should remain
the accounting foundation.

### Billing activation and proof of capability

For a newly introduced pass-through policy, activation should validate the complete
required rate set and provider capabilities. An explicit zero price means free;
an absent price means unconfigured. Existing lane fallback behavior must remain
compatible during migration, but it is not sufficient evidence that all intended
pass-through items are priced. Unknown models or missing required usage classes
need an explicit admission/settlement policy rather than an accidental free rate.

Acceptance requires the following checks through the actual metering and settlement
path, including Lago compatibility where relevant:

1. Give input, output, cache reads and cache writes distinct test rates. A fixture
   with nonzero values in all four must produce the exact expected line items,
   funding split and wallet total, without charging the total again.
2. Run the same measured quantities through two models with different rates, and
   through supported tier/context variants. Both admission and settlement must
   select the intended price version.
3. Change prices while a request is running and replay its settlement later.
   Its agreed customer price remains reproducible, and the invoice agrees with
   the local settlement. Supplier valuation retains its own effective-rate rules.
4. Verify JSON and streaming representations of the same invocation yield the
   same quantities. A missing terminal event produces partial/unknown usage,
   never an asserted complete zero.
5. Re-deliver usage events and crash during settlement. One invocation/component
   is charged once; a genuinely new provider attempt remains a separate record.
6. Exercise inclusive input, separate Anthropic cache counts, reasoning subsets,
   modality intersections, and mixed write TTLs for each claimed supported adapter.
7. Reconstruct a NyxBot turn from its invocations. Customer line items sum exactly
   to the gross charge; benefits explain the difference from the wallet debit.
   Internal utility calls and infrastructure retries follow explicit policies.
8. Demonstrate provider reconciliation at the granularity the supplier supports.
   Record unexplained differences rather than calling every calculated cost final.

Until those checks pass, the accurate capability statement is: **NyxID supports
independent prices for four core token classes, but complete provider-specific
cost attribution and pass-through billing still require implementation work.**

## Implementation evidence

| Area | Source anchors |
| --- | --- |
| Four token metrics and lane components | `backend/src/models/service_billing.rs:8`, `:43`, `:284` |
| Provider-preserving versus normalized counts | `backend/src/models/service_billing.rs:231`; `backend/src/services/llm_usage_service.rs:50` |
| Cache-write and Gemini extraction gaps | `backend/src/services/llm_usage_service.rs:461`, `:478`, `:527` |
| Streaming opt-in and native Gemini acceptance gap | `backend/src/services/llm_usage_service.rs:386`, `:776`, `:787` |
| Missing usage and byte estimate | `backend/src/services/llm_usage_service.rs:372`, `:742` |
| Capture eligibility | `backend/src/services/billing/metric_resolution.rs:80` |
| Cancellation settles captured usage/estimates | `backend/src/handlers/proxy.rs:6290` |
| Durable usage dimensions | `backend/src/models/usage_meter.rs:221`; `backend/src/models/usage_rollup_hourly.rs:8` |
| Model cache seam, wildcard admission/refresh | `backend/src/models/billing_rate_cache.rs:9`; `backend/src/services/billing/reservation.rs:1084`; `backend/src/services/billing/reconcile.rs:213` |
| Model lookup during settlement | `backend/src/services/billing/funding.rs:933` |
| Flat standard plan-rate reading | `backend/src/services/billing/lago_client.rs:887` |
| Reservation quantities | `backend/src/services/billing/route_context.rs:312` |
| Component settlement intent and recovery | `backend/src/services/billing/meter.rs:211` |
| Existing normalization test | `backend/src/services/llm_usage_service.rs:1613` |
| NyxBot model-proxy contract | `docs/chat/08-nyxagent-engine.md:222` |
| Direct and utility inference | `backend/src/handlers/assistant_direct.rs:155`; `backend/src/services/assistant_oneshot_inference.rs:958` |

[openai-cache]: https://developers.openai.com/api/docs/guides/prompt-caching
[openai-pricing]: https://developers.openai.com/api/docs/pricing
[openai-reasoning]: https://developers.openai.com/api/docs/guides/reasoning
[azure-cache]: https://learn.microsoft.com/en-us/azure/foundry/openai/how-to/prompt-caching
[azure-pricing]: https://azure.microsoft.com/en-us/pricing/details/azure-openai/
[azure-monitor]: https://learn.microsoft.com/en-us/azure/foundry-classic/openai/how-to/monitor-openai
[google-usage]: https://docs.cloud.google.com/gemini-enterprise-agent-platform/reference/rest/v1/GenerateContentResponse
[google-pricing]: https://cloud.google.com/gemini-enterprise-agent-platform/generative-ai/pricing
[google-cache]: https://docs.cloud.google.com/gemini-enterprise-agent-platform/models/context-cache/context-cache-overview
[anthropic-cache]: https://platform.claude.com/docs/en/build-with-claude/prompt-caching
