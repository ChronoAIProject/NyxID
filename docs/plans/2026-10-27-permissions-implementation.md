# NyxID permissions recovery (#2004, #2007)

Plan revision: 3  
Workstream: permissions  
Branch: `fix/assistant-permissions-2004-2007`  
Base: `b1dd6548ade04ec4f0051203e1353796331c7f87`  
Issues: [#2004](https://github.com/ChronoAIProject/NyxID/issues/2004), [#2007](https://github.com/ChronoAIProject/NyxID/issues/2007)  
Owner: `ctkm-aelf`  
PM: GPT-6-Astra in Heca worktree `01a17a96-9c05-76b2-89c9-951191466647`

The user has explicitly amended the workflow to a pure Astra–Sol process.
Earlier Claude Opus plan and final-review gates are recorded as
`SUPERSEDED_BY_USER`; they are historical evidence only. Astra personally
approves this exact revision before launching the single GPT-6.1-Sol
implementer. Live callback body, deployed-version, and Lark/web acceptance
evidence remain mandatory final gates and are never waived by local fixtures.

## Evidence, request legs, and root-cause status

### What issue #2004 actually establishes

The issue reports a Lark conversation in which an ordinary user question and a
later denied action repeatedly produced Ask-every-time cards while the selected
service was `chrono-llm-public` and the operation was `POST responses`. It also
records distinct acknowledgement IDs and an empty failed continuation. The issue
does not contain the production callback body, NyxID operation classification,
or a trace from the callback through the proxy. Therefore the deployed callback
payload and its exact production version remain an open acceptance dependency.

### The two HTTP legs are separate

NyxID's `assistant_nyxagent::proxy_raw` installs the private
`proxy::NyxAgentAdmission` extension for the first leg, NyxID to the fixed
`llm-nyx` engine service. In `handlers/proxy.rs`, the extension is additionally
accepted only when the target slug is `llm-nyx`; the code can change the
**outer engine transport** descriptor to read after checking method, path,
override, destructive effects, and the NyxAgent service row contract.

NyxAgent then makes a second HTTP request to the selected model service,
typically `/api/v1/proxy/s/chrono-llm-public/responses`, authenticated by the
conversation's agent key. That request enters NyxID afresh through
`api_key_auth_user`, receives its own assistant turn fence, and runs
`assistant_service_execution::check_http_projected`. An Axum request extension
does not cross this HTTP boundary. The existing `NyxAgentAdmission` marker
therefore cannot be the cause of, or a safe fix for, the reported model
callback. The plan explicitly rejects forwarding that marker, a forgeable
header/body signal, or a blanket stateful model bypass.

### Upstream request-builder evidence

A read-only clone of NyxAgent was inspected at
`/tmp/nyxagent-permissions-inspect-20261027`, commit
`f6988c5a3955f69cfd895fcfa7c9e5c2a986541f`. In `src/runtime.rs`,
`Bridge::bridge_request` authenticates a server-generated local bridge
capability, parses the Codex request, overwrites `model` with the configured
profile model, forces `store: false`, rejects nested `tools` in input messages,
and retains only known NyxID function tools plus optional `web_search`. It then
forwards the normalized JSON to the configured NyxID path with the original
agent key. It does not forward a NyxAgent admission marker. This is source
evidence for the intended callback contract, not proof of the currently
deployed production version; that distinction remains in the ledger.

The existing NyxID source already allows `tool_choice` in the closed stateless
Responses contract. It does not allow the top-level `client_metadata` field.
The botfather completion review records that Codex 0.160.1 emits
`client_metadata` (alongside `tool_choice` and the declared tools) in a raw
10,653-byte request. With `store:false`, that otherwise safe callback currently
fails `stateless_inference` because the unknown field makes the classifier
return false. `http_operation` consequently emits `kind: "http"`; the later
`legacy_inference && stateless` predicate remains false, so the POST becomes a
change and can produce the repeated card observed in #2004. This is the current
high-confidence local root-cause hypothesis. It must be proved by a sanitized
regression fixture and, if available, the live callback trace before calling
the live gate complete.

The checked-in Codex fixture at
`docs/fixtures/codex-public-responses-0.160.1.json` supplies the concrete
metadata shape without relying on production logs: seven scalar string keys,
36-byte turn/install IDs, a 38-byte window ID, and a 697-byte
`x-codex-turn-metadata` string. The implementation fixture will replace all
values with deterministic placeholders while preserving the observed field
lengths and 1,121-byte sorted compact JSON size, then assert the exact seven-key,
2,048-byte whole-object bound described below.

### #2007 evidence

PR #2014 correctly moved security-readiness enforcement to activation/configuration
writes. Enabled flags remain readable and ordinary execution does not become a
read-path outage. The operation-scope editor resets its form only after a
successful save. The remaining client defect is that `assistantJson` parses a
structured HTTP 503 with numeric code `12900` and key
`security_readiness_unavailable`, but `assistantRequestErrorMessage` discards
those fields and displays generic temporary-unavailability text. The user is
not told that an administrator must complete the rollout or that the draft is
still unsaved.

## Intended behavior and security contract

1. A valid, server-normalized, stateless Responses callback (`store:false`,
   closed endpoint and payload contract) is classified as model inference and
   uses the existing model-callback/turn fences. The operation retains its
   service, path, query, body digest, semantic-header digest, connection and
   credential/generation bindings.
2. The classifier recognizes only the bounded metadata fields that are part of
   the observed callback contract. The minimal proposed change is adding
   top-level `client_metadata` with this exact shape: an object containing only
   the seven observed string keys `root_turn_id`, `session_id`, `thread_id`,
   `turn_id`, `x-codex-installation-id`, `x-codex-turn-metadata`, and
   `x-codex-window-id`; no nested objects or arrays; and no additional keys.
   The four turn IDs and installation ID are ASCII `[A-Za-z0-9_-]`, 1-128 bytes each; the
   window ID uses the same bound; `x-codex-turn-metadata` is UTF-8 text capped at
   1,024 bytes (the inspected capture was 697 bytes); and the UTF-8 JSON encoding
   of the object with lexicographically sorted keys and no insignificant
   whitespace is capped at 2,048 bytes. These limits cover the
   captured 36/38-byte IDs and metadata with headroom while preventing a hidden
   payload channel. The values are metadata only and never supply identity,
   authority, destinations, handles, or provider state. No field that can carry
   executable effects may be admitted through this change.
3. `store:true`, `background:true`, conversation/file/previous-response and
   other resource handles, unknown extensions, method overrides, wrong paths or
   services, guest requests, provider-hosted tools, MCP/resource handles, and
   external writes remain exact change/destructive operations. Function tools
   remain allowed only as model callbacks to NyxAgent; their own effects still
   receive independent approvals.
4. The existing `NyxAgentAdmission` extension remains confined to the outer
   `llm-nyx` transport. No callback caller can opt into it, and no callback
   operation is made read-class by a header, body marker, or assistant-key
   shortcut.
5. A denied action produces one terminal explanation event under the existing
   acknowledgement/owner fences. The explanation's normalized model callback
   does not create a second equivalent model card, execute the denied effect, or
   leave an orphan pending approval. User acknowledgement and reply-channel
   authority remain unchanged.
6. A readiness save failure with HTTP 503, `12900`, and
   `security_readiness_unavailable` tells the owner that an administrator must
   complete security rollout readiness and that the proposed changes remain
   unsaved. The bounded `X-NyxID-Trace-Id` remains available for support.
   Other 12900 uses, including OAuth authority failures, retain their existing
   copy.

## Concrete implementation

### Backend stateless callback contract

1. Update `backend/src/services/assistant_service_execution/request_identity.rs`
   so the closed `stateless_inference` allowlist recognizes the actual safe
   top-level `client_metadata` field emitted by the inspected Codex bridge. Enforce
   the exact seven-key, scalar-string, ASCII/UTF-8 and 2,048-byte bounds above;
   reject nested handles, arrays, executable fields, or any unknown key. Keep
   `store:false`, the model requirement, endpoint allowlist, recursive handle
   rejection, and function/web-search tool-type checks unchanged. Do not accept
   arbitrary JSON by allowing all unknown fields.
2. Do not change `NyxAgentAdmission`, `proxy_raw`, or the callback HTTP ingress
   to forward a marker. Do not alter `legacy_model_generation` or the action
   predicate to classify every assistant-key POST as read. Preserve body and
   semantic-header digests, model-consent binding, operation-scope checks,
   execution-authority/generation checks, credential epochs, and turn fences.
3. Add a sanitized Rust fixture matching the upstream builder's shape
   (`model`, `input`/messages, `store:false`, `stream`, declared function tools,
   `tool_choice`, bounded `client_metadata`, and no prompt/credential values).
   Preserve the observed 36/36/36/36/36/697/38-byte field lengths and 1,121-byte
   compact JSON size with deterministic replacements. Assert before/after
   behavior at both layers: the old allowlist rejects the fixture, the new
   allowlist accepts it, `http_operation` emits inference, and
   `check_http_projected` reaches the existing model-callback read path. Assert
   that changing the body or semantic header changes the bound operation.
4. Add negative regressions for `store:true`, `background:true`, conversation,
   file and previous-response handles, unknown fields, invalid metadata shape,
   method override, wrong service/path, guest calls, provider-hosted tools,
   resource-bearing nested values, and external write operations. Each must
   remain change/destructive and require the existing exact approval.

### Denial and continuation regression

1. Reuse `owner_denial_decided`, acknowledgement CAS, live-turn fences, and
   existing event identity. Do not introduce a retry event or client workaround.
2. Add a backend regression for a denied change followed by the
   `action_denied` explanation callback. Prove one denial decision, no second
   equivalent model card, no denied provider effect, a non-empty terminal
   explanation, and no orphan pending approval. Include duplicate decision,
   concurrent continuation, reload/expiry, and cancellation cases.
3. Exercise both Lark/channel and web ingress fixtures with the same callback
   contract. Keep only sanitized operation kind, IDs, status and fixed reason
   codes in evidence. Do not log prompts, callback bodies, credentials or
   marker values.

### Readiness-aware operation editor copy

1. Extend `frontend/src/lib/assistant/request-error.ts` with optional structured
   error key/code inputs. Map only HTTP 503 plus `error_code === 12900` and
   `error === "security_readiness_unavailable"` to fixed actionable copy that
   names administrator action, rollout readiness, and unsaved changes.
2. Pass both parsed fields from `frontend/src/lib/assistant/assistant-http.ts`
   while preserving `AssistantHttpError`, trace-ID validation, billing handling,
   and all other status mappings.
3. Add focused Vitest coverage for the exact readiness envelope, another 12900,
   generic 503, trace correlation, malformed envelopes, and network failures.
   Extend the operation-scope component/request test to reject a readiness save,
   retain the selected five-operation dirty draft, and render the actionable
   alert. `form.reset` must remain success-only.

## Shared contracts and automation boundary

The automation workstream proposes server-derived `trigger_run_id` on
`AgentEvent`, `GroupFollower`, and `ChatAuthority`, trigger-aware
`TurnClaim`, selective event draining, and a fenced continuation claim. Permissions overlaps
only the existing `assistant_service_execution` classifier and the denial
continuation regression. It will not add or reinterpret those fields, wake
logic, scheduler ownership, or selective draining. A trigger-bound continuation
must preserve its exact run/thread identity and pass the same normal live-turn
and acknowledgement fences before a model callback. The combined Opus review
must inspect this interaction; branches remain independently testable and no
cross-branch import is permitted.

## Failure, retry, rollout, and rollback

* Missing, stale, malformed, or over-budget `client_metadata` fails closed to
  exact change approval. A changed body/header/path, service connection,
  credential epoch, execution generation, or turn fence cannot consume another
  approval.
* A denied provider action is never retried. Its explanation may use the
  normalized model callback once; a failed explanation is a terminal turn error
  with no orphan card.
* Readiness diagnostics remain available for reads and fail closed for the
  activation write. A failed save keeps the local draft so the owner can retry
  after rollout readiness is repaired.
* Deploy backend replicas containing the allowlist change before relying on the
  new callback shape; mixed replicas continue to fail closed to exact approval.
  Deploy the frontend after the structured error mapping is available. Monitor
  card counts and readiness-save responses without logging request bodies or
  secrets. Rollback only after draining in-flight continuations; rollback may
  restore an extra card but must never broaden authorization.

## Acceptance matrix and evidence

| Area | Required evidence |
| --- | --- |
| Reproduction | Sanitized fixture proves the Codex/NyxAgent callback is rejected before the `client_metadata` allowlist change and classified as stateless inference after it; no prompt or secret logging. |
| Callback contract | Read-only upstream source citation, deployed-version caveat, and (if available) a sanitized live callback trace showing `store:false`, endpoint, and metadata shape. |
| Informational turn | Ask-every-time pure question returns to its originating conversation without a model approval card; bounded local/replay evidence is complete even if live Lark access remains open. |
| Denial | One clear explanation, no second equivalent model card, no denied effect, no empty failure, no orphan pending card. |
| Exact authorization | Stateful resources, provider-hosted effects, external writes, arbitrary POSTs, unknown extensions, wrong service/path, overrides and guests remain exact change/destructive paths. |
| Idempotency | Duplicate clicks, concurrent continuations, expiry, reload and cancellation leave one durable outcome and no duplicate effect. |
| Operation scopes | Five Google operations save/read back, allowed operations execute, excluded operations fail, and a later revision cannot broaden an old approval. |
| Readiness UX | Key-qualified 12900 shows administrator/rollout/unsaved copy plus safe trace ID; draft stays selected and dirty; generic 503/OAuth 12900 copy is unchanged. |
| Existing behavior | Normal approvals, non-Lark web ingress, activation readiness, audit, delegated/live-turn authority, credential epoch and execution-generation fences remain green. |
| Automation boundary | Trigger-bound fields and continuation ownership remain untouched; combined Opus review records no authority broadening at the interaction boundary. |
| Checks | `cargo fmt --check`; focused Rust tests; frontend Vitest tests for assistant HTTP/request error and operation editor; relevant lint/typecheck; bounded full checks with the shared Cargo target. |

## Astra plan sign-off questions

Before implementation, Astra must explicitly verify: (a) the two-leg
distinction and the absence of a marker across the callback HTTP hop; (b) that
the exact seven-key, scalar-string `client_metadata` shape and 2,048-byte bound
are sufficient for the captured callback and cannot admit stateful/provider-hosted
effects; (c) that exact body/header/authority bindings remain intact; (d) that
denial and automation continuations retain their original channel/run authority;
(e) that the frontend mapping is key-qualified; and (f) that the production
callback/deployment gate remains open until evidence exists even if local
classifier fixtures pass. This assessment is recorded as
`ASTRA_PLAN_APPROVED` for the exact plan SHA in the review ledger and status
record.

## Implementation order

1. Astra records `ASTRA_PLAN_APPROVED` for this exact revision and launches the
   one persistent GPT-6.1-Sol implementer in this worktree. No Opus session is
   resumed or prompted under the amended workflow.
2. Implement the minimal callback allowlist change and backend regressions.
3. Add denial/continuation regressions and run focused Rust checks.
4. Implement structured readiness error mapping and operation-editor tests.
5. Astra personally reviews the complete diff, relevant surrounding code, test
   output, security boundaries, and acceptance matrix. Every valid finding,
   including nits, is sent to the same Sol session and re-tested until resolved.
6. Astra records `ASTRA_IMPLEMENTATION_APPROVED` only for the exact final
   commit after all available checks and acceptance evidence pass. Any later
   source change requires another personal review of the affected paths.

