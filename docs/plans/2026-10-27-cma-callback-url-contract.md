# CMA agent-page callback URL contract

Date: 2026-10-27. Upstream repository: `ChronoAIProject/cma`, base branch
`develop`. This document is the implementation handoff for a separate CMA
session. The CMA change described here has not been implemented or deployed by
this NyxID worktree.

## Required outcome

CMA supplies an optional `callback_url` in authenticated operation receipts and
status/response objects. Its value opens the actual CMA agent page. NyxBot
renders a prominent **View in CMA** link only when it has received a valid
`callback_url`; an agent ID, successful status, or catalog homepage alone must
never cause NyxID to invent a destination.

Despite the field name, this is a **browser navigation URL**. NyxID does not
POST to it, poll it, fetch it, or register a webhook. Existing authenticated
status polling and Responses streaming continue to carry status. This change
adds navigation metadata and does not change the meaning of a CMA status.

## Verified starting points in CMA

The following files were read from `ChronoAIProject/cma:develop` on 2026-10-27.
Recheck their current definitions before editing:

| Concern | Source | Observed behavior |
| --- | --- | --- |
| Public CMA origin | `api/crates/cma/src/config.rs` | `Config.public_url` reads `CHRONO_SERVER_PUBLIC_URL` and calls `validate_origin`; the existing development default is `http://localhost:8080`. |
| Origin validation | `api/crates/cma/src/config_validation.rs` | `validate_origin` already rejects non-HTTP(S) schemes, missing hosts, userinfo, queries, fragments, and non-root paths. Preserve those rules. |
| Lifecycle projections | `api/crates/layer1-public/src/rest/agent_lifecycle.rs` | `AgentLifecycleCreateReceipt` currently returns `id` and `status`; `AgentLifecycleOperationView` can also return `agent_id`, `phase`, and other lifecycle metadata. |
| V2 Responses projection | `api/crates/layer1-public/src/responses/v2/projection.rs` | Response `id` identifies the response; `conversation.id` identifies the agent. |
| Web route | `web/src/routing/routes.ts` | The agent page is `/agents/{agent_id}`. |
| Event gateway configuration | `api/crates/cmaeg/src/config.rs` | `CMAEG_PUBLIC_URL` belongs to the gateway and is not the CMA agent-page origin. |

The checked lifecycle and V2 Responses contracts do not currently provide this
browser URL. A diagnostic session against the user's configured NyxID/CMA
services also returned lifecycle/response IDs and an agent ID without a
browser URL. That establishes the missing contract; it does not validate the
new producer or consumer in a deployed environment.

Existing `nyxid_callback_url`, `/v1/callbacks/{channel_id}`, custom event URLs,
signed CloudEvents delivery, OAuth `/auth/callback`, and `/connection-return`
serve different protocols. Do not reuse or rename them for this field.

## Field definition and ownership

| Property | Requirement |
| --- | --- |
| JSON name | Exactly `callback_url`, in snake case. |
| Type | Optional, nullable absolute URL string. |
| Meaning | Browser URL for the real agent associated with this operation or response. |
| Authority | CMA computes it from validated server configuration and the resolved agent identity. |
| Route | Root-relative `/agents/{actual_agent_id}` on the configured public CMA origin. |
| Availability | Supply it as soon as the corresponding agent page is addressable under normal CMA authentication. Do not wait for work completion if the page already exists. |
| Unknown agent | Return `null` or omit the field until a real, addressable agent exists. |
| Unavailable destination | Return explicit `null` to withdraw a previously supplied link. |
| Stability | The same agent and public-origin configuration produce the same destination across receipts, GETs, streams, and idempotent retries. |

The public URL is display metadata, not a capability or access grant. Following
it uses ordinary CMA login, authorization, and agent ownership checks. Never
put credentials, API keys, OAuth codes, bearer tokens, signed access grants,
user information, query strings, or fragments into this URL.

Do not accept this response metadata from the caller's request body, tool
arguments, assistant output, `Host`, `Forwarded`, or `X-Forwarded-*` headers.
An incoming field with the same spelling must not override the server-generated
value. Use the persisted/resolved agent identity, never a lifecycle operation
ID, response ID, sandbox ID, or an unverified caller-supplied string.

## Environment and URL construction

1. Reuse `Config.public_url`, loaded from **`CHRONO_SERVER_PUBLIC_URL`**. Pass
   the parsed/validated public origin through the normal application state and
   service/projection layers; do not read environment variables per response.
2. Build the destination with a shared helper and a URL library. Join the
   root-relative route `/agents/{agent_id}` against the configured origin.
   Validate/encode the agent identifier as one path segment using the existing
   canonical CMA agent ID type, so it cannot introduce another path, query, or
   fragment.
3. Preserve configured scheme, hostname, and nondefault port. Origins with or
   without a trailing slash must produce the same link and no double slash in
   the route. Existing public-origin validation should continue to reject
   configuration containing userinfo, a query, fragment, or non-root base path.
4. Do not hardcode a production hostname or add a second competing public URL
   variable for this task. Do not use the gateway origin, API proxy origin,
   NyxID origin, catalog homepage, or an inbound request hostname as a fallback.
5. Check deployment manifests and environment documentation to ensure each
   environment sets `CHRONO_SERVER_PUBLIC_URL` to its browser-facing CMA
   origin. Keep the existing local default; deployed environments must supply
   their own origin. HTTPS is required for deployed public environments; HTTP
   remains usable for local/self-hosted development.
6. The output must be at most 2,048 UTF-8 bytes, use `http://` or `https://`,
   include a hostname, and contain no literal whitespace, controls, backslash,
   userinfo, query, or fragment. Cloud metadata endpoints are invalid. Failure
   to produce a usable navigation URL must not turn otherwise successful
   provisioning or execution into a failure; expose no link and diagnose the
   configuration through existing logging/configuration mechanisms without
   logging secrets.

Examples use test domains, not deployment defaults:

| `CHRONO_SERVER_PUBLIC_URL` | Agent | Expected `callback_url` |
| --- | --- | --- |
| `https://cma.example` | `agt_example` | `https://cma.example/agents/agt_example` |
| `https://staging.example:8443/` | `agt_example` | `https://staging.example:8443/agents/agt_example` |
| `http://localhost:8080` | `agt_example` | `http://localhost:8080/agents/agt_example` |

The checked `validate_origin` already rejects non-root paths. Reuse that
validation and add coverage for the callback helper; never carry an API prefix
into a browser route. The helper must also reject malformed raw input that a
URL parser might otherwise normalize, such as literal whitespace or controls.

## Response surfaces

Add the field through shared projections so all representations agree. Locate
the current V1 and AG-UI definitions rather than assuming they share V2 structs.

| Operation/object | Required placement and timing |
| --- | --- |
| `POST /api/v1/agents` lifecycle creation receipt | Top-level `callback_url`. Null/omitted if the receipt only identifies a lifecycle operation and an agent page does not exist yet. If an idempotent replay already resolves to an addressable agent, return its current link. |
| `GET /api/v1/agent-lifecycle/{id}` lifecycle status | Top-level `callback_url` next to `agent_id`/`status`; populate once the real agent page exists, including while provisioning continues. Return it on later observations whenever still valid. |
| `GET /api/v1/agents/{id}` | Top-level `callback_url` for the returned agent. This is the status resource NyxID uses for agent-input/AG-UI watches. |
| `POST /api/v1/agents/{id}/input` | Include the server-resolved agent link in the accepted receipt where compatible with that receipt type. Its GET status resource must also supply the field. |
| `POST /api/v1/agui/run` | Include it in a structured response object if this protocol has a compatible metadata location. Do not inject ad hoc SSE event types or assistant prose. At minimum, the corresponding agent GET must supply it. |
| V1 Responses: `POST /api/v1/responses`, `GET /api/v1/responses/{response_id}` | Put `callback_url` on the response object using that response's resolved agent. Preserve existing response/status semantics. |
| V2 Responses: `POST /api/v2/responses`, `POST /api/v2/agents/{agent_id}/responses`, `GET /api/v2/responses/{response_id}` | Put `callback_url` on the response object for `conversation.id`. Response `id` remains the work identity and must not become the browser route. |
| Responses SSE (`response.created`, status transitions, terminal events) | Put the field **inside `response`**, alongside `response.id` and `response.status`. Emit it in every full response projection once available; GET must return the same destination. |

For surfaces where the page cannot exist yet, null is correct. Do not fabricate
an `agt_` ID from a lifecycle receipt. A provisioning link can appear before
`succeeded` only if the real page already resolves. If a deleted agent page no
longer resolves, send explicit null; if CMA intentionally retains an authorized
history page for that agent, it can retain that valid destination.

## Payload examples

Illustrative partial payloads below omit unrelated existing fields; preserve
their current schema and values in the implementation.

Creation accepted, agent page not available yet:

```json
{
  "id": "lifecycle-example",
  "status": "queued",
  "callback_url": null
}
```

Lifecycle observation after the actual agent page becomes addressable:

```json
{
  "id": "lifecycle-example",
  "status": "running",
  "phase": "agent_startup",
  "agent_id": "agt_example",
  "callback_url": "https://cma.example/agents/agt_example"
}
```

V2 response object, whether returned by POST or GET:

```json
{
  "id": "resp_example",
  "status": "in_progress",
  "conversation": { "id": "agt_example" },
  "callback_url": "https://cma.example/agents/agt_example"
}
```

The same field in an SSE frame:

```text
event: response.completed
data: {"response":{"id":"resp_example","status":"completed","conversation":{"id":"agt_example"},"callback_url":"https://cma.example/agents/agt_example"}}

```

## NyxID consumer contract

NyxID's durable CMA watch stores only a validated callback alongside bounded
identity and status metadata. The chat history API projects this as optional
`cma_url`; the upstream CMA field remains exactly `callback_url`.

| Incoming observation | NyxID behavior |
| --- | --- |
| Valid nonempty `callback_url` string | Store/replace the destination and show **View in CMA**. Preserve the supplied URL without constructing or rewriting its host/path. |
| Field omitted | Preserve the last valid link for this watch; if none has arrived, keep the link hidden. |
| Explicit `null` | Clear the stored destination and hide the link. |
| Empty, malformed, unsafe, overlong, or non-string value | Clear the stored destination and hide the link; continue processing the CMA status. |
| Agent ID or catalog homepage without a callback | Show status without a link. No fallback. |
| Request arguments or assistant prose containing a callback | Ignore for card metadata. Only authenticated CMA responses supply it. |
| Reload/history of an existing watch | Restore its stored valid callback. Rows predating this field deserialize with no callback and no link. |

For a Responses stream, NyxID carries an earlier supplied callback forward
through later frames that omit it, only within the same response ID. Explicit
null or invalid values supersede an earlier URL. CMA should still include the
field consistently in its full projections, because clients may reconnect and
read only GET or a later event.

This is a browser link only: rendering history makes no upstream request or
DNS lookup. It opens a new tab with `noopener noreferrer`. Link availability is
independent of status: a valid link can accompany working, ready, completed,
paused, or failed work. The card records the last observed status and date/time;
it does not claim a historical observation is a live status check.

## Compatibility and implementation boundaries

- Make the JSON field additive and optional in shared types, OpenAPI schemas,
  generated SDKs, fixtures, and examples. Follow the repository's generated-file
  workflow. Old clients must continue to accept responses and old producers
  must continue to work with NyxID with the link hidden.
- Centralize URL construction in an appropriate shared service/projection
  helper. Do not duplicate environment handling in each endpoint. Preserve
  CMA's layering and existing authorization gates.
- Idempotent retries return the real agent's link when the resource exists;
  they do not create another agent to obtain a callback. Returning the link
  must not perform extra provisioning or change billing.
- Existing lifecycle IDs, response IDs, statuses, phases, approvals, resource
  ownership, errors, cancellation, polling, and streaming protocols keep their
  current meanings. This task adds no webhook transport or callback receiver.
- Access to `ChronoAIProject/cma` has been verified with read/push/maintain
  permissions. Use a separate CMA checkout/worktree and a feature branch based
  on `develop`; preserve the local NyxID work in this worktree. Open a focused
  PR when the CMA change and required checks are ready. Do not merge/deploy as
  part of this handoff.

## Required tests and acceptance checks

| Scenario | Acceptance |
| --- | --- |
| Production/staging/local origins | Changing only `CHRONO_SERVER_PUBLIC_URL` changes the callback origin; no production hostname appears in runtime URL-building code. Nondefault ports work. |
| Trailing slash | Origin with/without `/` yields the same single-slash `/agents/{id}` route. |
| Invalid origin | Userinfo, query, fragment, non-root base path, invalid schemes, whitespace/control characters and metadata hosts do not produce callbacks. Follow existing config validation policy. |
| Unknown/unallocated agent | Creation receipt has null/omitted callback; no operation/response ID is substituted. |
| Agent allocated during provisioning | Next lifecycle observation supplies its actual page link as soon as addressable. |
| Completed provisioning | Final lifecycle status includes the correct link; it identifies the same agent returned in `agent_id`. |
| V1/V2 Responses | POST/GET/SSE destinations agree with the actual agent, including nested `conversation.id` in V2. |
| Streaming | `callback_url` is inside `response`, survives terminal frames, and is available through GET after reconnect. |
| Queued input/AG-UI | Agent GET supplies the callback even when initial acceptance lacks one. |
| Idempotency/replay | Repeating the same request preserves the real agent identity and correct configured-origin destination without new side effects. |
| Injection attempts | Caller-provided callback, `Host`, and forwarded headers cannot change the generated URL. Identifier syntax cannot escape its path segment. |
| Missing/later/null callback | NyxID hides initially, shows on later valid observation, preserves on omission, and hides on explicit null. |
| Invalid callback | NyxID stores/emits no unsafe URL and continues to show observed status. Test JavaScript/data schemes, relative URLs, userinfo, query/fragment, controls, backslash, non-string and >2,048-byte strings. |
| Reload and old data | Persisted valid callback restores after reload; old rows and old producers show no link. No catalog/agent-ID fallback. |
| Authentication/ACL | An authorized user reaches the actual agent page; an unauthorized user receives ordinary CMA authorization behavior. Link grants no extra access. |
| Status independence | Working, failed, and completed operations may all show a supplied valid link. Missing callback never changes their status label. |

Before the CMA PR is considered complete, run its required formatting, type,
unit/integration, schema generation and API compatibility checks. Report exactly
which checks ran and any that could not run. Use a test/staging session to
capture the actual POST, status GET, SSE and response GET payloads; verify the
link against the real authenticated agent page without logging credentials or
private prompts/output.

Deploying the NyxID consumer first is compatible: older CMA emits no callback
and NyxID shows status without a link. Deploying the additive CMA field first
also supports older clients that tolerate unknown fields. After both sides are
deployed, verify a creation that initially has no page, a later observation
that supplies the link, a completed Responses operation, and a history reload.
Historical settled watches with no supplied callback stay without a link; this
task does not invent links or backfill old operations.

## Prompt for the separate CMA session

The user also requested convergence with Opus 5.5. No Opus review/signoff has
been obtained: the local Claude CLI is unauthenticated, and earlier provider
attempts were blocked by authentication or insufficient credits. If Opus is
available in the separate session, have it review this contract and the current
CMA projections before implementation. Record its actual findings; do not treat
this handoff as an Opus-approved design.

```text
Implement the CMA producer side of the callback_url contract in:
docs/plans/2026-10-27-cma-callback-url-contract.md
in the NyxID fresh-dove worktree.

Read that document first. Work in ChronoAIProject/cma on a feature branch from
develop using a separate checkout/worktree; preserve the current NyxID changes.
Recheck CMA's repository instructions and current source definitions.

callback_url is the browser link to the actual CMA agent page, not a webhook.
Generate it from Config.public_url / CHRONO_SERVER_PUBLIC_URL and the resolved
agent identity. Do not hardcode a hostname. Add it consistently to lifecycle,
agent, V1/V2 Responses and Responses SSE projections as specified. Null means
the destination is unavailable; omitted means no update. Keep it additive,
credential-free, and independent of operation status.

Implement shared construction, schema/SDK/docs updates and meaningful tests.
Validate actual receipt, polling and streaming payloads and an authorized
browser clickthrough where the test environment permits. Prepare a focused PR
against develop with validation results; do not merge or deploy.
```
