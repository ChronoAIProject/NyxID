# Preserve Buffered Proxy Responses During Billing Settlement

## Goal

Preserve an already-successful downstream HTTP response when NyxID cannot persist billing settlement after forwarding the request.

## Invariant

Billing is metadata-only after a proxy request has been forwarded. Once the downstream response succeeds, a billing persistence failure must not replace its status, headers, or body with a NyxID error.

## Current Failure

The proxy handles small or diagnostic responses by buffering their bodies. After the downstream response has completed, this path awaits synchronous billing settlement before building the client response. A settlement persistence error therefore escapes the handler and rewrites a successful downstream response as NyxID's generic internal error.

Streaming proxy responses already use the intended behavior: they persist a durable settlement intent, schedule settlement, and log a warning if that work fails without changing the response.

## Design

Use the existing deferred settlement path for buffered responses as well as streaming responses. The buffered path will call the existing `settle_meter_async` helper after usage calculation, then return the original downstream response.

No new billing abstraction, MIME special case, ZIP-specific branch, proxy retry, Ornn retry, or Aevatar fallback is introduced. The change applies at the shared buffered proxy response path so every content type receives the same semantics.

## Data Flow

1. NyxID resolves billing and opens a usage meter row.
2. NyxID forwards the request and marks the meter row as forwarded.
3. The downstream returns a response that NyxID buffers.
4. NyxID computes response bytes and any reported LLM usage.
5. NyxID calls the existing deferred settlement helper, which persists settlement intent and schedules settlement.
6. NyxID returns the downstream status, headers, and buffered body unchanged.

## Failure Handling

- Failures before forwarding retain the existing fail-closed billing behavior.
- A failure to persist or execute settlement after forwarding is logged with the existing billing request identifier and durable-retry semantics.
- A post-forward settlement failure does not change the downstream response.
- Client cancellation and downstream failures retain their existing behavior.

## Test

Add one behavioral regression test around the mounted buffered proxy route. The downstream returns a known `200` response, while billing settlement persistence is forced to fail after forwarding. The test must fail on the current synchronous path because the handler returns an internal error, then pass after the buffered path adopts deferred settlement by asserting the original status and body are preserved.

Reuse the existing Mongo-backed proxy billing integration harness and failure controls if available. Do not introduce a billing interface or production-only injection point solely for this test.

## Scope

Only the shared buffered response settlement call and its focused regression test are in scope. Production deployment, Ornn changes, Aevatar fallback behavior, and unrelated billing refactors are excluded.
