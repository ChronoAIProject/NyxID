# Owner NyxBot CLI and invocation API

`nyxid nyxbot` runs owner work through the existing NyxAgent engine and keeps a
receipt that can be inspected after a disconnected client or lost response.
Use a first-party account login (`nyxid login`, account access) and the usual
CLI profile/server selection. The HTTP endpoints accept the same first-party
owner cookie or access token. Developer OAuth clients, delegated tokens,
relay tokens, service accounts, and API keys, including Agent Key login, are
rejected. An explicit credential never falls back to a saved owner session.

First-party provenance identifies how the credential was issued. It does not
prove a person is present for each request. This surface carries the owner's
existing authority; it is not a restricted unattended credential. Specialist
grants, billing, engine availability, and owner confirmations still apply.
The CLI neither changes permission modes nor approves cards.

## Local terminal workflow

The `nyxid` executable runs on your computer and calls your configured NyxID
API. NyxBot execution, conversation history, memory, service credentials,
permissions, and billing remain on that server. This is a supported way to
give your existing NyxBot work from a terminal or script. It uses the same
agent and conversation engine as the web interface; it does not create a
separate local chatbot or change how that agent is configured.

Log into a dedicated owner profile and choose **full account access** on the
approval page:

```sh
nyxid login --profile nyxbot-owner --base-url https://nyx-api.chrono-ai.fun
nyxid nyxbot run --profile nyxbot-owner \
  --text 'Summarize my pending work' --output json
```

The instance must have upgraded replicas and enabled owner invocations as
described under [Rollout and rollback](#rollout-and-rollback). Installing a new
CLI does not enable server admission. Explicit credentials in the environment
still take precedence over the saved profile; a rejected Agent Key does not
fall back to the owner session.

For an existing self-hosted instance, use its API URL, such as
`http://localhost:3001`, at login. The CLI starts no listener or local server.
Self-hosting NyxID still requires its configured NyxAgent and model services;
this command does not install an offline engine.

`--file` reads only the named prompt file, and `--file -` reads stdin. Their
contents are sent to the server as prompt text. Running the CLI in a directory
does not give NyxBot access to that directory, shell, or desktop. Local machine
work uses separately configured [machine nodes](../MACHINE_NODES.md) and their
existing grants and confirmations. Specialist configuration and sensitive
owner decisions remain in the existing NyxID owner interface.

MCP serves a different client workflow: `nyxid mcp` configures external AI
clients to discover and call connected services. It does not expose this
owner invocation resource as a delegated tool. No new MCP setup is needed for
`nyxid nyxbot`; NyxBot's own MCP calls retain the shared authorization,
approval, and cancellation checks.

## Commands

```sh
nyxid nyxbot agents --output json
nyxid nyxbot models --output json
nyxid nyxbot conversations --limit 50 --output json
nyxid nyxbot history nyxa-CONVERSATION --limit 50 --output json

nyxid nyxbot run --text 'Summarize my pending work' --output json
nyxid nyxbot run --file prompt.txt --agent SPECIALIST_ID --output json
cat prompt.txt | nyxid nyxbot run --file - --no-wait --output json
nyxid nyxbot run --text 'Follow up on the previous answer' \
  --conversation nyxa-CONVERSATION --timeout 120 --output json

nyxid nyxbot show INVOCATION_UUID --output json
nyxid nyxbot wait INVOCATION_UUID --timeout 300 --output json
nyxid nyxbot cancel INVOCATION_UUID --output json
```

`run` waits by default. `--no-wait` returns after durable acceptance. An
optional `--model` selects an available model for a new conversation. An
existing conversation cannot be combined with `--agent` or `--model`.
Only private owner conversations of the owner’s NyxBot or owned specialist are
accepted. Hidden group, voice, automation, channel, and platform threads use
separate delivery protocols and are rejected before admission.
Discovery commands return the existing API's JSON objects, including pagination
metadata. Conversations accept `--agent`, `--cursor`, and `--limit`; history
accepts `--before-seq` and `--limit`.
Discovery HTTP failures emit a safe error object (or text equivalent) with
status, bounded error codes, and credential guidance. They never include the
raw response body or an invocation ID. Login and renewal failures retain exits
3 and 4. Successful discovery JSON keeps the API's original object shape.

Choose exactly one input: `--text`, `--file PATH`, or `--file -` for stdin.
Prompts must contain non-whitespace text and fit 32,768 Unicode scalar values
and 131,072 UTF-8 bytes. Do not put credentials in prompts. Text display strips
terminal control and Unicode bidi control characters from untrusted replies;
JSON escapes terminal control and bidi characters while preserving the exact
decoded content, including for discovery results.

Before submitting, `run` emits a small JSON recovery notice to stderr with the
invocation UUID. Supply `--idempotency-key UUID` to choose it explicitly. Keep
that UUID and the exact original request. On a lost response, use `show` or
`wait` first. Repeating `run` with the same UUID and the same request returns
its receipt; changing the request conflicts. No transport timeout or unknown
outcome causes an automatic second submission. The same receipt survives
conversation deletion.

The local timeout covers response reads and polling sleeps. It does not stop
server work. `show` and `cancel` have a bounded HTTP timeout as well. Explicit
cancellation requests a stop for this invocation family only. It does not stop
a later ordinary turn or an unrelated invocation on the same conversation.
Source-image handles and attachment commits also recheck this invocation, so a
download that finishes after cancellation cannot attach its image. Queued
invocation continuations and accepted channel questions retain their separate
delivery identities and cannot evict one another. A repeated channel question
queues separately instead of joining an invocation's reply destinations.

## JSON and process outcomes

Invocation commands write exactly one result object to stdout. Progress and
the early recovery notice use stderr. The result includes `cli_status`,
`invocation_id`, `invocation` (or null), `error`, `http_status`, `error_code`,
`reason`, `guidance`, and `exit_code`. Error reasons are bounded server codes;
raw HTTP error bodies and credential material are not included.

| Exit | Meaning |
| --- | --- |
| 0 | Completed, or accepted and still active for `run --no-wait`, `show`, or `cancel` |
| 1 | HTTP request rejected; inspect safe error code/reason |
| 3 | Login required |
| 4 | Session renewal temporarily uncertain |
| 20 | Execution failed |
| 21 | Invocation cancelled or its root conversation deleted |
| 22 | Owner action or review required |
| 23 | Local wait deadline reached; server work may continue |
| 24 | Transport outcome unknown; inspect the same invocation UUID |
| 25 | Invalid protocol or malformed/mismatched invocation response |

Argument/input errors and local authentication or profile setup errors use the
existing CLI error behavior rather than the invocation report contract.
Rejected Agent Key or developer credentials receive guidance to choose
first-party account access.
Credentials are never switched automatically.

## HTTP resource

All paths are relative to `/api/v1/assistant/nyxagent`:

| Method and path | Behavior |
| --- | --- |
| `POST /invocations` | Atomically admit work, or replay its existing receipt |
| `GET /invocations/{invocation_id}` | Read a bounded durable snapshot |
| `POST /invocations/{invocation_id}/cancel` | Fence the exact invocation family and request cancellation |

Create accepts only these fields:

```json
{
  "idempotency_key": "e104bf09-0d68-4f2e-a47a-81e352cc3e66",
  "text": "Summarize my pending work",
  "conversation_id": null,
  "agent_id": null,
  "model": null
}
```

The public invocation ID is the caller's canonical UUID v4 idempotency key,
scoped to the authenticated owner. Unknown fields are rejected. Nullable
optional fields and omitted optional fields canonicalize identically. The
retained request fingerprint is keyed and bound to the owner and public key;
it is not an unkeyed prompt digest. A new admission returns HTTP 202. A
matching replay returns HTTP 200 and never starts a new worker. A changed
request under the same key returns HTTP 409.

Responses include invocation/conversation/root-turn IDs, `status`, `terminal`,
creation time, cancellation state, turn/running/background/continuation counts,
`required_actions`, `replies`, truncation markers, `error_code`, and
`retry_after_ms`. Poll no faster than that interval. Each snapshot contains at
most 50 required actions and 16 assistant replies; each reply is capped at
4,096 UTF-8 bytes with a truncation marker. Read full retained history through
the existing conversation messages endpoint or `nyxid nyxbot history`.

| Status | Terminal | Meaning |
| --- | --- | --- |
| `running` | false | An associated turn is active |
| `waiting` | false | Background work, a durable continuation, or settlement hook remains |
| `requires_action` | false | Review owner cards or finish a setup flow in NyxID |
| `requires_review` | false | Progress cannot safely continue automatically; inspect `error_code` |
| `cancelling` | false | Stop fence committed; associated work is still settling |
| `completed` | true | Tracked turns, hooks, decisions, producers, and background work have settled |
| `failed` | true | Tracked execution settled with an error |
| `cancelled` | true | Tracked local work and supported cancellation have settled |
| `deleted` | true | The deleted root's tracked cancellation has settled |

A reply alone does not complete an invocation. Owner decisions queue a durable
continuation in the decision transaction. Expired decisions, lost settlement
hooks, unresolved or expired orchestrator decisions, paused event streaks,
failed setup/publication, and continuation beyond the 128-turn bound require
review. These review states remain nonterminal;
late work cannot mutate a receipt that has already been declared terminal.

The cancellation fence prevents new descendant admissions, tool authorization,
action-card consumption, watch creation, and event production. Machine stops
use exact turn/runtime scopes. An existing bounded sweep resumes cleanup after
a crash between the durable fence and node dispatch. It does not reexecute
turns or provider effects. External cancellation is attempted once per tracked
async operation. Missing cancellation support, a missing operation ID, HTTP
success without a declared terminal status read (including `200` with a
`running`/`cancelling` state), failure, timeout, or a crash around the attempt
leaves a durable `async_cancellation_unconfirmed` review outcome. It never
silently promises that the external job stopped, and never repeats an uncertain
cancel RPC.
Already dispatched external effects cannot be undone by this API.

A conversation's raw upstream key is retired at invocation-family boundaries,
including a later browser/ordinary turn, so an old invocation worker cannot
acquire successor authority. Same-family continuations retain their key and
card/watch bindings. If a different family still has pending cards, async or
setup watches, or machine jobs, admission conflicts with guidance to finish or
cancel them or choose a new conversation. Ordinary untouched conversations
retain their existing credential behavior.

Group execution through `post_to_group` is refused for owner invocations:
personal group member scheduling coalesces several origins into shared turns
and cannot supply this resource's exact cancellation contract. Assign owned
specialists directly, or post from the owner's group interface. Other group
management/discovery retains the existing permissions. No synthetic platform
origin or widened grant is used.

Admission and execution share the browser engine's transaction, limiter,
credential encryption, billing inventory, observability, and audit path.
Request identity and execution claims prevent duplicate admission. They do
not provide exactly-once downstream effects. A process lost after admission
is settled by existing orphan recovery instead of replaying an uncertain turn.

## Rollout and rollback

`assistant:owner-invocations` is a separate, default-off admission flag. Upgrade
**every replica** before enabling it. Older replicas can discard the additive
invocation fields when replacing conversations/cards/events and cannot enforce
the family cancellation contract. Leave the ordinary
`assistant:live-turn-gate` deployment flag unchanged. Invocation-controlled
conversations enforce their live/cancellation fence independently of that flag.

Disabling the admission flag stops new submissions while authenticated
inspection and cancellation remain available (with the engine enabled).
Before the first invocation admission, disabling this flag permits rollback to
the previous binaries. **After the first admission, the supported recovery
contract is forward-fix-only:** do not return older binaries to service against
this database and its conversation credentials. Disabling admission and
cancelling or draining work does not make an older binary safe. Invocation
conversation credentials can remain active after completion or cancellation,
and older binaries ignore their mandatory live-turn fence. Some unconfirmed
external effects also intentionally retain a nonterminal review receipt.

If an incident requires stopping new work, disable admission and use inspection
and exact cancellation on upgraded replicas while preparing a forward fix.
Returning older binaries would require a separately verified migration retiring
every affected conversation credential and session binding and proving no
runnable descendant work remains. That migration is outside this feature's
supported rollback procedure; cancelling receipts alone is insufficient.
