# NyxBot turn traces (NyxID #2280)

Use the existing admitted turn UUID as the support ID. `X-NyxID-Trace-Id`
returns that UUID when admission finishes before HTTP headers. `X-Request-Id`
remains a separate, server-issued per-request ID. Before admission, the trace
header retains its request-ID fallback. A response that has already started
cannot acquire a new header: queued receipts link the original request to their
winning turn after the binding transaction commits.

This release instruments NyxID. NyxAgent and CMA continue working without
changes, but their internal spans and CMA's queued response adoption require
counterpart releases. Missing records are diagnostic gaps, never success.

## Identity and trust

`backend/src/turn_trace.rs` owns a non-authorizing typed context. A turn's W3C
trace ID is its UUID without hyphens. The logical root span is the first eight
bytes (big endian, zero replaced by one) of SHA-256 over
`nyxid-turn-root-v1:` followed by the 16 UUID bytes. Child spans and request IDs
are fresh. Recovery can reconstruct the root from existing persisted turn IDs.
Admission records use committed admission time; settlement is a separate root
milestone. Attempt spans are independent; engine idempotency keys, including
continuation/image suffixes, remain unchanged. Retry/causal admission links use
`related_turn_id` when the caller already knows the source turn.

Ingress IDs are server-generated. A browser, webhook or arbitrary trace header
cannot select a turn. MCP/proxy callbacks attach only after authentication has
loaded an exact live turn and one valid `traceparent` (at most 256 bytes) matches it. Missing,
malformed or stale propagation emits an uncorrelated metadata record with a
closed `gap_reason`; it never attributes an old callback to a successor turn.
Internal work uses explicit captured context or reconstructs it from its exact
persisted source. Nothing in this module authorizes execution or changes fences.

Turn egress replaces `traceparent`, `x-request-id`, `tracestate` and `baggage`
after execution identity/approval validation. Only the generated W3C parent and
request UUID are sent. Authenticated callbacks with missing or mismatched context
strip caller correlation at egress and use a fresh request UUID, without adopting
the current live turn. This distinction survives detached work and body polling.
Unrelated proxy traffic keeps its existing forwarding behavior. Node metadata is injected before normal signing; signed
bytes and node request IDs are not edited afterward.

`NyxbotEvent.trace_request_id` is optional, server-authored metadata on the
existing receipt insert. Binding reads it from the already-required
`findOneAndUpdate` result, and emits its link only after commit. A source joining
an already-active turn returns the association through the same transaction;
it does not emit a new admission. Legacy receipts have no source-request link. No trace-only recovery lookup or repair worker is added. Delayed
sends use the original event/reply turn, even when the conversation has moved on.

## Output and privacy

The `nyxid::turn_trace` target emits one JSON object per line to stdout through a
dedicated, bounded writer. Other log targets keep their existing format.
Collectors should select JSON objects with `target == "nyxid::turn_trace"` and
`schema_version == 1`; human-readable stdout lines are not trace records.

The closed schema contains:

| Field | Contract |
| --- | --- |
| `schema_version`, `target`, `service` | `1`, `nyxid::turn_trace`, `nyxid` (counterparts use `nyxagent`/`cma`). |
| `instance_id`, `timestamp` | Random process UUID v4 and RFC3339 UTC timestamp; no hostname/config values. |
| `turn_id`, `trace_id` | Canonical turn UUID v4 and its lowercase 32hex form; absent on pre-admission/uncorrelated ingress. |
| `span_id`, `parent_span_id` | Nonzero lowercase 16hex ID; parent is optional. |
| `request_id` | Server-issued UUID v4 for this operation; arbitrary upstream request IDs are excluded. |
| `phase` | `start`, `end`, `link`, `milestone`. |
| `stage` | `ingress`, `admission`, `execution`, `capacity_wait`, `engine`, `first_output`, `mcp`, `proxy`, `proxy_headers`, `proxy_body`, `node_headers`, `channel`, `channel_part`, `replay`, `gateway`. |
| `outcome` | `completed`, `failed`, `cancelled`, `unknown`, `streamed`, `silent`, `confirmed`, `rejected`. Optional on nonterminal records. |
| `duration_ms`, `http_status`, `error_code`, `part_index` | Optional bounded unsigned numbers. Duration is omitted when unmeasured. |
| `ingress_request_id`, `related_turn_id` | Optional server-issued UUID links; no source capability/platform ID. |
| `gap_reason` | Optional `missing_parent`, `invalid_parent`, `mismatched_turn`. No claimed victim trace ID is emitted. |

No prompt, answer, reasoning, tool argument/result/name, content hash, media,
raw error, header map, URL/path/query, credential/ciphertext, platform sender,
chat/thread ID, gateway capability, IP or user agent enters this envelope.
Serialization never inherits surrounding tracing fields. Existing consent/DNT
product telemetry is unchanged; raw turn IDs are not added to analytics.
Existing audit calls gain `event_data.turn_id`, `trace_id`, `trace_span_id`,
`trace_request_id` before hashing/spawning. An explicitly different persisted
source turn retains precedence. No new audit is emitted just for a span, and
existing chained audit rows are never rewritten.

## Timing and coverage

Ingress ends at response headers; it does not drain SSE. Engine/proxy response
spans end at body completion, stream error, or drop. MCP outcomes use the actual
JSON-RPC/tool result, not just HTTP 200. `proxy_headers` measures outbound
admission/send through headers. Existing relay diagnostics supply `proxy_body`
terminal milestones even across detached streaming tasks. `node_headers` ends
at node response headers (or the buffered response), not at a streaming body's
EOF; the enclosing proxy/engine span and relay diagnostic measure consumption.
Node internals are not claimed as instrumented.

Capacity-wait and first-output timings reuse existing turn observations.
`gateway` ends after its bounded response body. A successful gateway HTTP
exchange is transport completion only. `channel` records distinguish confirmed,
rejected, unknown, streamed (platform delivery unconfirmed), and intentional
silence. A crash/drop never triggers an observational retry or fabricated
confirmation. `replay` spans measure snapshot/SSE consumption and end as `streamed`, never
platform confirmation. Native replay/snapshot handling retains its current ownership
barriers and legacy wire shapes; its future negotiated correlation contract is
a counterpart rollout dependency. Execution and delivery outcomes stay separate.

## Admin reader

Export the selected NDJSON records from service stdout, retaining their original
JSON fields. No network/database/credentials are needed by the reader:

```sh
python3 scripts/turn-trace.py --trace-id 11111111-1111-4111-8111-111111111111 --input nyxid.ndjson nyxagent.ndjson cma.ndjson
python3 scripts/turn-trace.py --trace-id 11111111111141118111111111111111 --input nyxid.ndjson --json
```

The reader accepts out-of-order records, deduplicates exports, pairs spans,
links bounded ingress records, and lists the longest observed operations. It
never computes latency from clock differences between hosts or sums overlapping
spans. Missing counterparts, root/terminal records, ingress links and malformed
or truncated input are shown as partial coverage. Channel milestones are kept
separate from HTTP transport outcomes. No records means unavailable coverage.
The reader discards unrecognized fields and never echoes invalid input.

Bounds: 2 KiB per line, 20,000 selected records, a 20,000-record ingress cache,
1,000,000 lines and 128 MiB scanned per invocation. Export a narrower window if
these limits are reached. A missing cached ingress is reported explicitly.

Admins/operators can also use **Admin → Audit log → Search field → Turn ID
(exact)**, or `GET /api/v1/admin/audit-log?turn_id=<UUID>` and the matching
`/count` route. Both use the same UUID validation and exact equality, reject
invalid/empty values, and retain current RBAC. The page/count queries use the
partial `audit_log_turn_trace` index on `(event_data.turn_id, created_at, _id)`
for string turn IDs, a three-second query limit and a 10,000-row page window.
Audit history is supplementary: it is not a complete timing trace. Old audits
without turn metadata remain unchanged and are not retroactively correlated.

## Resource budget and rollout

There are zero trace-only MongoDB reads/writes/transactions and no synchronous
collector calls. Trace metadata piggybacks existing writes; the audit index adds
ordinary index-maintenance cost. No per-token, per-chunk, heartbeat or typing
records are emitted. Each measured operation has at most a start and terminal
record plus bounded lifecycle milestones. Stream wrappers observe terminal
poll/drop without buffering or parsing content.

The process-wide queue holds at most 4,096 closed records, comfortably below
16 MiB. Records contain only fixed-size primitives and static metadata; recording
does not allocate owned strings. The writer formats IDs and serializes records
to less than 2 KiB, reusing one output buffer. Execution uses `try_send`; a full/disconnected writer increments
a fixed counter and never blocks, spills to MongoDB, retries a send, or changes
execution results. The writer reports cumulative drops at most once per minute
when it can make output progress. A process killed while output is blocked can
lose queued records and its last loss count; reports must remain best-effort.
The performance targets are <100 microseconds p99 per boundary, <1 ms p95
pre-dispatch overhead, and <1% mocked-upstream throughput regression. See the
validation report for measurements and shared-host limitations. The manual
throughput check sustains 32 concurrent HTTP calls over 1,024 requests per sample
against a 20 ms mock upstream, with request recording disabled. Two warmups and
twelve measured samples rotate baseline, healthy-writer and full-queue ordering.
The healthy writer uses the production encoder; the benchmark excludes stdout
I/O and measures median elapsed-time regression with the unchanged 1% limit.

Deploy the new audit index before relying on the new filter. The UI offers the
field only when the backend advertises it. List/count responses acknowledge the canonical `applied_turn_id`. The UI
refuses rows/counts from an old replica that silently ignores the filter; retry
after the rollout or pin admin requests to the upgraded pool. Roll out counterparts separately. No historical backfill, feature
activation, external collector, or service deployment is performed by this PR.

## Collection runbook placeholder (owner/infra)

**Expected input:** NDJSON stdout from NyxID, NyxAgent and CMA with the versioned
envelope above. Preserve timestamps, process IDs and trace/span IDs verbatim;
collect all replicas, including asynchronous workers. Index exact trace/request
IDs without turning them into metric labels. Apply administrator-only access
and an explicit operational retention policy.

**Deployment-owned decisions:** collector/product, transport, routing and JSON
selection, retention, access control, loss alerts, cross-service export/query
commands and an end-to-end staging exercise. Fill this section in the infra
work. No collector installation or collector-specific command is included here.
