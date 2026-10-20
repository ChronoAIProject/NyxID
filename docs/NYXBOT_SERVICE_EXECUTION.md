# NyxBot service execution

NyxBot enforces the acting owner's saved connection order at concrete execution,
after resolution and before provider or node dispatch. This covers direct MCP,
`nyx__call_tool`, raw proxy, LLM gateway, SSH execution and session issuance,
Oracle submissions, machine service requests and the actual member selected by a
Service Pool. Ordinary external clients keep advisory discovery ordering and their existing routing.

## Order and exact targets

Preference positions compare connections within one catalog service group.
Eligible personal connections precede eligible organization connections; each
tier follows saved order. Without saved positions, stable discovery order is
personal newest first, then organization membership order and newest within
each organization. Custom `connection:<id>` groups are singletons. Unrelated
providers/models never compete.

An exact named call is never silently moved to another credential. An out-of-order
proposal receives an exact eligible preferred candidate. A candidate must permit
the requested operation and inputs. Same catalog and URL do not establish
account/resource equivalence. Unknown account equivalence requires the owner to
choose the exact target, rather than making a model proposal count as selection.

The composer's **Next message connection** and a leading
`/service <connection UUID or unambiguous slug>` are authenticated owner choices
for one turn. Channel selectors require a verified owner sender. Guests, webhooks,
delegated credentials, service accounts and conversation keys cannot promote
text/flags into human selection. New ordinary turns reset the choice. An allowed
card continuation inherits only its exact bound target/operation; denied, expired
or consumed cards cannot install a choice. Selection is separate from consent.

## Existing permission modes

| Mode | Organization connection use | Other actions |
| --- | --- | --- |
| Full access (`autopilot`) | No prompt; exact selection and priority still enforced | Existing bypass behavior and successful-effect receipts |
| Auto (`nyxmode`) | Existing action card before use, including reads | Existing sensitive/destructive classification |
| Ask every time (`ask_every_time`) | Existing action card before use, including reads | Existing change/sensitive/destructive classification |

Organization ownership comes from the resolved effective owner and authoritative
User type, never a model flag or an unrelated catalog creator. Public Person-owned
Oracle pools are not organization resources. Platform/BYO credential-source
labels do not prove org ownership. Existing guest grants remain authoritative;
guests cannot raise/consume owner cards or widen their read/use permissions.

The existing `assistant:live-turn-gate` rollout remains off by default. With it
off, personal requests first authenticated without a live turn retain ordinary
automatic execution under the existing permission mode. A mode-required action
card or any organization connection use requires a real live turn. Idle calls
still enforce personal saved order, exact grants, ACLs, operation scopes and
final key/configuration/mode checks, and cannot reuse turn selections, consent
or outage evidence. Live admissions always stop on Stop or a replaced turn.
The public MCP transport retains its existing live activity requirement even
with this rollout off; direct executor compatibility does not remove that gate.

Model transport scope and account equivalence are separate. The closed supported
generation routes and NyxAgent's `responses/compact` callback retain their existing
model scope exception, including omitted/true `store`, previous response/file
handles and provider extensions. Personal/platform calls keep live and idle
compatibility; organization calls require a live turn and the existing consent
gate. An organization-owned specialist may use an otherwise ungranted thinking
model through that same implicit scope exception. Explicit grants still use the
organization resource boundary, and every path retains live membership, key,
connection ACL, operation and execution-configuration checks.

Every nonstateless model request binds its exact account and complete semantic
request, uses the existing change-action permission gate, and never inherits
stateless model consent or account fallback. Its card pauses the runtime visibly,
and Allow resumes only the exact bound request. No exception extends to arbitrary
provider file, response-management or other resource routes.

## Model bootstrap and turn consent

Raw stateless inference on a conversation key has one honest bounded contract:
an organization card says **Use connection / model for this turn**. Allow consumes
the card once and installs a server-persisted grant for the current resumed turn,
exact connection/owner/provider/model/endpoint, semantic headers, configuration,
credential generation and original server-admitted input. Model/tool/model rounds
may change prompts and transport tracing within that grant. A new turn, model,
semantic target/header or authority/config change requires fresh admission.
Personal stateless model calls preserve existing no-extra-prompt behavior.

NyxAgent uses the same caller key for callbacks; NyxID does not claim to recognize
an authenticated model-only marker. All raw stateless inference on that key uses
this contract. MCP inference tools retain exact semantic payload consent.
Stateful provider operations also retain exact action consent. Supported stateless
generation requires an object/model and known request semantics; Responses must
set `store:false`. Client function definitions and public `web_search` are
supported. Provider account handles, reusable prompts, containers, stored or
background responses, remote MCP and provider mutation tools are exact actions.

If the initial model callback needs org consent, NyxID persists/renders the normal
card and pauses visibly before provider egress. Server-owned current-attempt
correlation identifies the refusal that caused the failed callback. Unrelated
pending cards cannot hide upstream failures, and Stop wins in both cancellation
result forms. Allow starts a visible continuation with the card's bound input
admission. Deny never grants or triggers another organization attempt.

Automatic title, learning and voice metadata inference has no live consent turn.
It retains configured platform utility routing or eligible personal-only fallback
in saved order, and returns the existing optional absence when unavailable.

## Availability and pools

Fallback evidence is generated by the server, scoped to turn, operation and exact
connection/credential authority. Offline nodes qualify only when no configured
allowed fallback route works. Verified pre-dispatch transport failure, unusable
credentials after refresh, and bounded temporary read failures can qualify.
Evidence expires after 60 seconds, deduplicates atomically and is bounded to 32
entries per turn. Replacement credentials cannot inherit prior evidence.

Invalid arguments, missing resources, empty results, ACL/policy denial and declined
or invalid consent are never availability evidence. An uncertain dispatched write
must not retry/switch without reconciliation or an applicable idempotency guarantee.
Refusal does not initiate attempts against other orgs.

Service Pools keep configured priority/rotation/weight behavior. Consent pins the
actual member durably across HTTP requests, replicas and balancing ticks. An
approved member never silently changes. A configured safe pre-dispatch failover
may propose the next member with fresh admission; outside bypass, org consent is
required for that exact next member. Pool ordering is distinct from connection
preferences within each catalog group.

## Execution identity and lifecycle

Exact actions bind connection UUID, effective owner, endpoint/operation contract,
method, canonical path, query, actual bounded body bytes and sanitized forwarded
semantic headers. Acknowledgement control metadata is never forwarded.
Transport tracing and correlation values do not participate in semantic identity;
account/project and idempotency headers do.
The held resolved target and configured primary/fallback routes are compared with
fresh authoritative resolution immediately before dispatch, after approval,
billing and other awaits. Deterministic gateway destination/header translations
are reproduced in that comparison. Revoked membership, stopped/replaced turns,
elapsed keys and credential/config drift stop egress and release unforwarded
billing reservations. Successful effects alone produce automatic receipts. A
service approval receipt takes precedence when both gates allow an action
automatically. HTTP streams must finish cleanly; HEAD and bodyless successful
responses complete without polling a body.

Managed Telnyx HTTP and MCP calls retain the same final authority check inside
the worker that owns their funded execution. A proven refusal before the provider
send releases that execution's reservation and produces no success receipt.
After dispatch, Stop or caller disconnect does not cancel settlement or turn an
uncertain outcome into a retryable operation. See
[Telnyx platform billing](TELNYX_PLATFORM_BILLING.md) for its funding and recovery
contract.

`assistant_service_execution` is private server state with absolute TTL and an
owner/conversation/key/turn/kind index. Input admissions, selections, model grants,
attempt markers, job plans, card bindings and outage buckets expire within their
bounded turn/card lifetime. Expiry is checked at use; TTL cleanup is not authority.
Conversation deletion, agent destruction and owner purge remove associated rows.
Machine command-use grants live on the existing job row and expire with that job.
Externally authorized Remote Machine jobs retain their own source and machine
grant authority. They do not require NyxBot chat consent. Their signed gateway
requests keep exact service declarations and live source, grant, runtime and
lease checks before dispatch.
Approved async submissions through user-managed connections persist their exact
execution-authority digest on the existing `assistant_async_operations` watch.
The private watch capability permits only that operation ID's declared
status/result GETs and optional cancel POST,
within its deadline and fenced lease. For a cancelled owner invocation, the
capability permits only one durably recorded cancellation attempt and a declared
status read to confirm the outcome. That status read may follow the normal watch
deadline, but must remain within the watch expiry and live lease; result reads
are refused. The original invocation and delivery binding are checked before
each send. Ordinary active watches remain usable while the conversation
is idle or running a later turn; it never supplies a synthetic live turn. Each
send rechecks the live key, membership, service scope, connection/configuration
and credential generation, contract and lease. Revocation stops further polling.
Legacy watches admitted before this feature keep their bounded result/cancel
lifecycle under live authority checks. New versioned user-managed submissions
require both connection and endpoint-contract snapshots. Organization approval
of the submission covers this bounded continuation; it does not authorize a
second submission or arbitrary background requests.
Digests/control metadata are never forwarded or exposed as credential material.

SSH certificate, tunnel and terminal requests use existing modes and the existing
action card. Their admission binds the exact connection or existing catalog-only
authority, public key/principal or initial terminal geometry, SSH configuration,
and maximum lifetime (at most one hour). Consent authorizes one session's dynamic
traffic. Final open checks and live session supervision enforce expiry, Stop,
key/node scope and revocation. Existing direct TCP tunnels and catalog-only
creator/public access remain available; an exact selected connection cannot
escape through catalog fallback. Certificates already delivered remain valid
until their signed expiry, as in the existing SSH certificate contract.

Private execution guards run after token exchange, MCP adapter initialization and
WebSocket slot/signing preparation, immediately before the operation send/open.
A refused guard prevents that attempt from sending and creates neither
availability evidence nor a success receipt. When the entire request has no
prior dispatched operation, it releases billing/durable bookkeeping. A prior
possibly dispatched read retry retains its existing uncertain accounting.
This is a practical final authority check, not an atomic transaction with the
external provider's network receive.
