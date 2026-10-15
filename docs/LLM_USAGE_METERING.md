# LLM invocation metering

NyxID retains additive provider observations for future usage valuation. Existing
billing quantities, prices, charge identities, wallet/grant/allowance precedence,
and ledger canonical encodings continue to govern settlement. This release does
not activate a new rate card or supplier-cost calculation.

## Measurement and current charges

`LlmUsageObservation` version 1 carries allowlisted optional signed 64-bit token
counts. An absent count is unknown; an explicit zero is measured zero, except
where the native adapter defines an omitted field as nonapplicable zero. Strings,
floats, negatives and overflowing integers are invalid measurements. The legacy
billing parser retains its previous coercion and fallback behavior. Invalid new
measurements cannot alter an existing charge.

Observations record provider input/output/total control counts, four normalized
classes (ordinary input, output, cache reads and cache writes), reasoning and tool
subsets, audio/image/text subsets, known cache/modality intersections, and
Anthropic five-minute/one-hour write subsets. Subsets are not additional token
charges. Reasoning, modalities and write TTLs are not new independently priced
`BillingMetric` variants.

| Adapter | Observation normalization | Existing charge behavior |
| --- | --- | --- |
| OpenAI-compatible Chat/Responses, including Azure | Inclusive input less reported cache reads and writes. Reasoning remains inside output. Unknown custom-provider partitions remain unknown, while the inclusive reported input is retained. | Newly recognized `cache_write_tokens` fields are observation-only. Existing input/read/write quantities and total fallback remain unchanged. No opt-in charge remapping is introduced in this release. |
| Resolved native OpenAI/Azure routes | Omitted write counts may be treated as nonapplicable zero only on resolved HTTPS native OpenAI/Azure origins, never inferred from a model name. This does not fill missing reads or malformed fields. | Same existing charging rules. |
| Anthropic Messages | Input is already exclusive of separate cache reads/writes. Write TTL fields are subsets of aggregate writes. | The existing four token metrics remain independently priceable. |
| Native Gemini JSON/SSE | Ordinary input excludes cached content. Normalized output includes candidates and thoughts once; provider controls retain candidates separately. Native omitted cache, thought and tool counts are zero; generation has no token-write class. | Published single-response JSON charging remains unchanged. Newly recognized native Gemini SSE and JSON-array stream measurements are observation-only; existing total/byte fallback charging remains unchanged. |

Control totals must match a fully known adapter equation. Known subset sums may
not exceed their parent counts; TTL splits must equal the aggregate write count
when both lifetimes are reported. Checked arithmetic marks overflow invalid.
Completeness is `complete`, `partial` or `missing`, separately from the execution
outcome (`succeeded`, `interrupted`, `failed`, `unknown`). Complete requires a
successful terminal outcome, all four classes and consistent counts. Optional
unreported modality intersections stay unknown even for a complete four-class
observation. HTTP errors and known interrupted transports override observational
success; a later successful terminal marker cannot erase a failure.

Provider response, resolved model and processing tier identities are restricted
to 256 ASCII identifier characters. Requested model/tier are retained separately.
Bodies, prompts, completions, credential material and arbitrary provider metadata
are not copied into invocation details.

## Aggregation and execution coverage

JSON and SSE share the strict extractor. Sparse cumulative snapshots merge their
reported maxima; explicit deltas add checked counts. Completed image indexes are
deduplicated before either legacy or observational counts are merged. Streams
without a terminal provider marker retain interrupted/partial measurements.
Native Gemini/Vertex `:streamGenerateContent` JSON arrays use the same cumulative
snapshot aggregation on the already bounded body copy. Repeated final chunks do
not add counts twice. Empty or syntactically malformed/truncated arrays supply no
measurements; valid chunks with invalid counts retain partial invalid observations.
Generic custom-response arrays are not traversed.

Realtime details describe a `realtime_connection` aggregate. Distinct response
IDs are deduplicated. Each class is summed only if every response reports it;
missing per-response classes make that connection total unknown. Reported and
unreported response counts describe coverage. Multiple responses clear the
single-response identity; mixed model/tier/adapter metadata is cleared or marked
unknown. Native omitted-zero inference never runs on an aggregate and cannot
reconstruct a missing partition after summation. Legacy byte estimates remain
outside the provider observations.

Capture follows resolved inference execution paths across HTTP/SSE, provider and
gateway translation, node proxy, MCP, realtime, assistant direct execution,
utility one-shot inference, opt-in platform inference and voice token usage.
Catalog inference metadata enables free/BYOK capture without loading wallets or
unrelated machine state. Known model-list/detail metadata paths do not create missing-usage observations.
Recognized provider reports remain capturable on custom and Vertex inference
paths, including `:predict`, `:rawPredict`, `:streamRawPredict` and `/generate`. Pool attempts keep distinct request/attempt
identities, including rejected or unknown attempts whose existing policy bypasses
normal quantity settlement. Observational presence alone never certifies a
billable quantity or changes that policy.

Chat `stream_options.include_usage` is injected only for Chat Completions paths
and an existing known provider capability or explicit
`capabilities.supports_stream_usage` with OpenAI Chat protocol metadata. Unknown
custom services receive no new unsupported parameter. The capability is API
editable and preserved through service and catalog metadata form edits.

Assistant conversation/agent/live-turn references come from the authenticated
server-loaded conversation authority already verified for execution. Caller
headers cannot assign a payer, conversation or another user's query scope.
Title generation retains the authorized conversation and agent. Learning retains
its verified agent; voice utility classifiers retain the authorized call
conversation. Voice token invocations retain that conversation, its agent and the
requested model from verified preferences. They do not claim a chat turn identity
without live execution authority. Utility operations without a conversation
retain their actor and request context.

## Durable capture and retention

The dispatch marker atomically seeds a missing/unknown observation and retry
intent on the primary row for eligible inference. This survives cancellation or
process loss even when no normal completion is reached. First settlement replaces
provisional measurements while retaining any stronger durable failed/interrupted
outcome. Missing replays cannot erase reported measurements. These writes never
settle quantities or change existing unknown-usage money policies. Translated
stream body errors retain their established no-settlement behavior while
persisting partial failed observations.

A primary `usage_meter` row stores the observation and durable
`llm_detail_pending` retry marker with settlement intent, independently of its
`pending_platform_usage` snapshot. Component rows retain their existing charge
identities and do not duplicate the observation. Older binaries may deserialize
and reserialize the pending usage snapshot, dropping nested new fields; their
field updates preserve the independent primary observation and retry marker.
Old BSON without the new fields remains readable. Existing ledger bytes do not
include the new fields.

The indexed `llm_usage_invocations` collection retains one bounded detail per
primary row for 365 days from invocation creation, beyond the normal 30-day raw
row retention. It snapshots actor/owner, service/connection/credential class,
request and available assistant/pool identities, measured classes, resolved and
requested model/tier, and exact existing charge/funding results. Exact credits
use BSON Decimal128 and API decimal strings. Unknown costs stay null. These are
customer settlement amounts, not invented supplier costs.

Materialization scopes all charge sources by request and flush sequence. At most
32 source rows may certify a snapshot; overflow fails explicitly. `released`
alone is not settled funding. Charged rows require terminal funding settlement;
zero/meter-only and abandoned unknown-quantity policies are handled explicitly.

Transactions fence the current primary marker, protect all source rows from TTL
expiration, and commit a complete retained snapshot together with marker/hold
removal. ACK writes atomically honor holds; concurrent ACK and completion restore
normal expiry for acknowledged sources. Stale workers and idempotent replays
cannot reintroduce a hold after completion. Complete monetary snapshots are
immutable. Incomplete snapshots remain refreshable.

After primary capture is durable, derived detail-write failures defer to an
independent bounded recovery worker, every 30 seconds, without failing a paid
provider result or requesting another provider attempt. Recovery also runs when
billing is disabled. A failure to persist the primary intent is still an error;
acknowledged observations are not silently discarded. Financial and retained
usage records follow their retention even when user-facing chat data is deleted.

## Invocation read APIs

Authenticated routes are registered in OpenAPI alongside the existing billing
usage APIs. Existing usage response shapes are unchanged.

- `GET /api/v1/billing/usage/invocations`
- `GET /api/v1/billing/usage/invocations/{id}`

List filters are `owner_id`, `from`, `to`, `conversation_id`, `turn_id`, `limit`
and opaque `cursor`. The default is the acting person's most recent 30 days;
windows are positive and bounded to 31 days. Limits are 1–100 (default 50).
Descending creation time and ID provide stable cursor pagination, including tied
timestamps. Queries have a five-second server limit. Detail accepts optional
`owner_id`. Responses use dedicated DTOs and omit expired records immediately,
independently of asynchronous Mongo TTL deletion.

Personal reads require the acting owner. Organization reads resolve active
membership and effective service scope, including scoped admins and viewers.
Scope matches the recorded `user_service_id`; unknown historical connections do
not bypass restrictions. Unauthorized owners are forbidden; inaccessible detail
IDs return not found. Request bodies or correlation headers cannot widen access.

## Local verification

Use an isolated MongoDB 8 replica set and set `NYXID_TEST_DATABASE_URL` to its URI.
The database fixtures require transactions. Run one test process with two test
threads; large parallel Mongo fixture suites can exhaust local file descriptors.
The metering review used these bounded build settings from the repository root:

```sh
export CARGO_TARGET_DIR=/tmp/nyxid-llm-metering-target
export CARGO_BUILD_JOBS=4
export CARGO_INCREMENTAL=0
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_PROFILE_TEST_DEBUG=0
cargo test -p nyxid --no-run
cargo test -p nyxid -- \
  services::llm_observation services::llm_usage_service \
  services::billing models::credits models::usage_meter handlers::llm_invocations \
  handlers::llm_gateway handlers::proxy::tests \
  handlers::proxy::pool_attempt::tests \
  handlers::service_pool_proxy_tests::billing \
  pool_ai_unknown_openai_provider \
  services::mcp_service::tests::node_stream_requires_an_explicit_terminal_frame \
  services::mcp_service::tests::mcp_usage_estimates_only_token_services \
  services::assistant_oneshot_inference handlers::assistant_titles \
  services::voice::tokens \
  voice_options_use_catalog_models_voices_and_supported_protocols_only \
  platform_onboarding_definite_model_refusal \
  pool_detail_store_failure \
  --test-threads=2
```

These filters exercise strict/legacy extraction, native JSON/SSE/array streams,
realtime coverage, dispatch/drop recovery, old BSON snapshots, transactional
retention/ACK interleavings, owner and service scope, tied-timestamp pagination,
four distinct exact charge rates, derived-store failure and utility attribution.
The distinct-rate proof uses the existing Anthropic charge mapping; OpenAI write
extensions and corrected native Gemini measurements stay observation-only.

From `frontend/`, validate capability preservation through both forms:

```sh
./node_modules/.bin/tsc -b
npm test -- src/pages/service-edit.test.tsx src/components/services/catalog-tool-metadata.test.tsx
./node_modules/.bin/eslint src/components/services/catalog-tool-metadata.test.tsx \
  src/components/services/catalog-tool-metadata.tsx src/pages/service-edit.helpers.ts \
  src/pages/service-edit.test.tsx src/schemas/services.ts src/schemas/tools.ts src/types/api.ts
```
