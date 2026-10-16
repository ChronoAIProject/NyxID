# NyxID public runtime deployment contract v1

Status: NyxID implementation prepared for review; upstream implementation and
deployment are pending authorization. No deployed acceptance is claimed.
Inspected upstream main: `57ab8be89618a61b98e2ebc76331d121f96d731b`.

The configured `llm-nyx` catalog transport is the sole upstream authority. It has
no personal credential and follows no redirects. Public discovery is separate
from ordinary `/v1/models` and image/steer capabilities.

## Signed control plane

NyxID signs RS256 assertions with its existing JWT signing key and publishes the
verification key through existing JWKS. Assertions use `typ=nyxid-public+jwt`,
`aud=nyxagent-public`, an exact issuer, a unique `jti`, strict expiration and a
single scope: `discovery`, `readiness`, `execution`, `probe_model`, or `cleanup`.
They are never ordinary access tokens. Upstream must explicitly allow the NyxID
issuer/origin and validate the assertion before generic caller authentication.
There is no new shared secret or probing personal API key.

Each assertion binds `nyxagent_service_id`, `nyxagent_catalog_digest`, manager
profile/bot/Telegram identity/epoch, public policy revision, sponsor and candidate
digest. Request body hashes bind exact request bytes. Execution additionally
binds the public key SHA256, public session, turn, receipt and claim, immutable
runtime/profile digests, exact model operations, loop limits and absolute session
expiry. Upstream must reject profile/build/runtime drift on every execution.

Requests carry the assertion as `X-NyxID-Public-Authority`. Only execution also
carries the public key as Authorization Bearer. Upstream must verify its hash,
preserve the original tuple in every MCP call (including initialize), and pass
all four `x-nyx-public-{session,turn,receipt,claim}-id` headers. MCP initialize
returns trusted public purpose/authority metadata. Request fields and ordinary
profile labels never establish purpose. Ordinary personal authentication stays
unchanged.

Model callbacks also carry a unique UUID `x-nyx-public-call-id`. Callback retries
must reuse that call ID; a new model attempt uses a new call ID and consumes the
next call allowance. MCP `tools/call` uses its own exact idempotency identity.
The public key alone never permits generic REST, proxy or personal MCP access.
The execution request still uses the Responses transport at `/v1/responses`;
upstream must establish signed public authority before entering personal auth,
profile selection, session initialization or MCP initialization.

## Discovery and readiness

`POST /v1/public/profiles` returns a strict, nonce/binding-echoed manifest. It
contains an explicit public default, profile digests, immutable build/runtime
digest, callback-only model operations and limits, and exact semantic contracts
`nyxid-public-execution.v1`, `nyxid-public-model.v1`,
`nyxid-public-tools.v1`, `nyxid-public-cleanup.v1`. Every containment flag must be
true. No public contract / 404 / 501 means `public_contract_unsupported`, not a
missing selected profile. Malformed responses are infrastructure/protocol errors.
HTTP 401/403 means `public_authority_rejected`: the runtime must trust the configured
NyxID issuer, existing JWKS and dedicated audience. HTTP 402/429/503 communicate
funding/capacity/runtime unavailability. None of these statuses creates evidence.

`POST /v1/public/readiness` receives the exact discovered manifest and a short
probe-model assertion. Upstream performs one bounded text call through the dedicated
NyxID probe callback. NyxID stores the observed exact model, nonempty response,
usage, meter settlement and total-cap admission before accepting Ready. Runtime
readiness must also establish its immutable runtime/profile and public enforcement
semantics. Discovery never establishes Ready.

Proof is persisted per manager/candidate/evidence, never process-global. Stable
policy identity excludes nonces and check times. Each turn pins an immutable
proof; callbacks never load a newer proof after turnover. New admissions refresh
expired proof through bounded sponsor-funded readiness, outside transactions.
A MongoDB lease keyed by the activated manager binding and stable digest coordinates
automatic refresh across replicas. The holder rechecks for an adequate proof before
funding one probe; contenders reuse its persisted, fully fenced result. Work and
waiting are bounded by the admitted ingress claim and a 95-second ceiling, shorter
than the 100-second lease. Automatic probe signatures and funding also use this
final claim-bound deadline. A failed or cancelled holder retains its lease through
its bounded expiry, so a delayed callback cannot overlap a replacement refresh.
A failed or expired holder leaves readiness unavailable; contenders do not
initiate a second paid attempt. Waiting polls persisted candidate identity and
expiry read-only every 500 ms. Each sufficiently fresh candidate receives one
transactional assessment per waiter; unchanged near-expiry evidence causes no
repeated proof/catalog/sponsor writes. Final admission fences authority again.
The lease coordinates work only
and never establishes execution authority.
Final admission, callbacks and replies recheck local expiry/catalog/grant/control
fences transactionally. Settlement and cleanup remain legal after expiry.

The maximum proof lifetime is five minutes. New ingress refreshes before the
remaining proof lifetime becomes shorter than the complete bounded turn plus a
five-second margin. Refresh compares the entire stable policy digest with the
durable human-activated deployment and rejects a changed contract. Probes are limited
to one attempt per manager per 30 seconds. Their deadline is at most 90 seconds,
clamped to the next UTC cap-day boundary and enforced by MongoDB at preparation,
funding and dispatch. They settle through
the ordinary exact billing transaction and recovery path. Their cost shares the
manager/day total cap; no fake sender quota is used. Explicit human validation
records short-lived candidate consent, fences its original control revisions,
and rechecks the administrator and sponsor organization membership before each
funding or proof effect.

Wire request and response shapes are closed (`deny_unknown_fields`):

| Route | Request JSON | Response JSON |
| --- | --- | --- |
| `POST /v1/public/profiles` | `contract`, UUID `nonce`, `binding`, `catalog_bindings` | echoed `nonce`, `binding`, `manifest` |
| `POST /v1/public/readiness` | `contract`, UUID `nonce`, `binding`, exact `manifest`, `selected_profile`, UUID `probe_id`, `probe_callback`, `probe_authorization`, `probe_body_utf8` | `contract`, echoed `nonce`, `binding`, exact `manifest`, `selected_profile`, `probe_id` |
| `POST /v1/public/cleanup` | `contract`, `public_session_id`, `policy_digest` | echoed `contract`, `public_session_id`, `policy_digest`, boolean `erased` |

`probe_callback` is relative to the configured NyxID API origin; never accept an
arbitrary callback host. Send `probe_body_utf8` as the exact UTF-8 request bytes,
with `Authorization: Bearer <probe_authorization>`. The assertion binds those
bytes, the stored one-use probe ID and exact catalog/manager/sponsor tuple. JSON
reserialization may change the body hash and is forbidden for this callback.
The probe body is fixed short text with no tools. Its output-token limit is the
advertised profile per-call bound; NyxID reserves that exact bound. A reasoning
model must have enough of its declared budget to produce visible text. Local mocks
prove the bound and accounting agreement, not actual model support.
NyxID observes nonempty text, the exact requested model and settled usage itself;
the upstream readiness response cannot substitute for that observation.

The normative wire fixture is [platform-public-v1.json](../fixtures/platform-public-v1.json).
Its build, identities, model and digests are local examples. A real manifest must
identify the running immutable binary/container digest and effective public profile
digest. The strict Rust wire structures live in `models/platform_public_contract.rs`
and `services/platform_public_contract_service.rs`. The fixture's execution assertion
includes the entire manifest, service/catalog digest, stable policy digest, proof
record ID, credential hash, original tuple and both absolute deadlines. The other
scopes accept only their own typed authority schema; no ordinary JWT interchange.
Manifest array ordering is part of the contract identity and must stay stable
across refreshes of the same deployed contract.

Catalog authority digests (`nyxagent_catalog_digest` and each model operation's
`execution_authority_digest`) and the manager `candidate_digest` use NyxID's
canonical JSON digest format: `sha256:` followed by exactly 64 lowercase hex
characters. Copy the catalog authority digest from trusted discovery without
removing its prefix. Runtime/profile digests, endpoint-set digests, stable policy
and evidence digests, request byte hashes and credential hashes are exactly 64
lowercase hex characters without a prefix. These representations are strict;
uppercase, alternate algorithms and truncated values are rejected.

## Model protocol and cost

The only dynamic public tools are exactly `public_faq_search`, `public_faq_read`,
`public_signup_link`, with schemas identical to NyxID public MCP `tools/list` and
model callback `tools`. Do not declare the six ordinary NyxID wrappers. No shell,
computer, owner/native tool, web search or provider builtin is allowed.

Upstream normalizes Codex wire fields to the closed public model schema before
callbacks. All model work, retries, continuations and compaction count against
one cumulative budget. Compaction uses the same model callback with a bounded
text summarization request; it cannot call a direct provider or unbounded compact
endpoint. Never silently drop unsupported Codex fields; fail the public turn.

Model paths are relative to the catalog API root. For `chrono-llm-public` base
`https://llm.aelf.dev/v1`, the Responses operation is `responses`. Trusted inference
metadata can declare protocol operations for `AiService` only when its durable
endpoint set is empty. The whole catalog digest, endpoint-set digest and conditional
parent write fence provide authority. Endpoint mutations already write the parent
through the ordinary catalog mutation service, so additions conflict with an
admission that observed absence. A selected durable endpoint additionally requires
its actual operation name, positive generation and publication. The complete
endpoint snapshot binds IDs, paths, methods, target, active/publication state and
generation; addition, removal, replacement or disable rejects execution. Tool
publication and arbitrary proxy paths retain their existing gates. Operation
prices use the same `BillingService::http_operation`/active-operation resolver as
ordinary HTTP billing, including a durable endpoint's real name.

Text-only native bounds assign zero to images/voice. Active input/output/cache
components and operation overrides still use existing billing estimates and exact
settlement. A readiness probe checks affordability of the conservative full loop
against both configured caps, including 10/day and 0.50/person. Probe calls use
only the total public cap, no fabricated sender. NyxID never debits a second wallet.

At the observed prices (input 0.000005, output 0.00002, cache read 0.0000008,
cache write 0.000006 and images 0.2), the local regression profile's 4096 input /
256 output token bound costs at most 0.0534528 credits per call, including
conservative cache estimates and zero images. Two calls cost 0.1069056, below
0.50/person and 10/day. These figures validate the local billing shape; each real
runtime's advertised bounds and current prices are checked afresh before readiness.
Explicit saved caps are preserved.

## Expiry and cleanup

Public sessions have an absolute maximum 24-hour lifetime, fixed at creation and
never extended. Turns also have a hard deadline and bounded model/tool calls,
bytes, input/output tokens and continuations. Upstream enforces the signed bounds
and the callback boundary enforces sponsor funding before every provider effect.

`POST /v1/public/cleanup` accepts only a cleanup assertion, after execution-key
revocation. It erases the exact NyxID public-session index plus all associated
upstream sessions/responses, including a dispatch with no observed upstream
handle. Response echoes the exact public session and scope digest and confirms
erasure. Do not restore execution keys, infer deletion from TTL, or delete newer
sessions after a stale cleanup. Keep encrypted handles/tombstones until confirmed.
The typed cleanup assertion carries immutable `session_expires_at` even after that
absolute deadline has elapsed. Ambiguous failure revokes access without shortening
this deadline. The upstream keeps an exact public-session revocation tombstone
through the signed deadline, including when cleanup arrives before any execution
has created a session record, rejecting delayed execution requests after cleanup. Local erasure commits only while the cleanup worker's exact claim and
lease remain current. Failure retains evidence and retries; an elapsed TTL never
means cleanup succeeded. Bounded seven-day evidence retention preserves only the
newest proof for the current live activation, every retained turn's pinned proof,
and the latest authority needed by each unfinished cleanup policy. Associated probe evidence cannot expire
before its retained proof.
Cleanup must accept retained public-session authority from an earlier runtime
build after an upgrade; it grants deletion of that exact index only. Execution
continues to require the current binary/profile contract. A changed catalog
destination is never followed to perform cleanup of the old destination: that
case retains its tombstone and needs a separately reviewed transport recovery.


## Asynchronous public failure delivery

A failed public turn saves only sanitized failure text, settles its original
receipt and closes its budget loop. A possibly dispatched upstream context is
revoked and queued for exact cleanup while retaining its original absolute
deadline. Repeated settlement of the same saved reply returns the original
reference without replacing its body, digest or outcome, or touching a successor.

Revoked transcript delivery stays forbidden. Event consumption and saved-reply
recovery route a saved failed turn through a separate server-fixed notice:
“The public guide could not finish this turn. Please try again to start fresh,
or send /link to use your own NyxBot.” The notice never decrypts model content or
restores a credential. It uses the existing fixed delivery record and dedupe,
with a deadline of two minutes after persisted receipt settlement, bounded by
the original absolute session deadline. Retrying cannot extend that deadline.

The delivery transaction binds the original failed turn/reply, receipt and claim,
manager epoch, public policy revision, sender, sponsor and active deployment.
Only the exact session's automatic failure revocation can coexist with the
notice; manager stop, deployment replacement, sponsor or sender revocation,
changed receipt and elapsed notice/session lifetime suppress it. The durable
fixed delivery owns recovery after enqueue. A later message may start a fresh
public session through ordinary activation, readiness and budget gates.

## Review and proof limits

Protocol fixtures exercise local validation, not upstream deployment or external
acceptance. Required runtime tests include trusted purpose before generic MCP auth,
exact three-tool agreement, normalized real Codex payloads, compaction/retries,
model and binary/profile drift, hard budgets, absolute expiry, revoked-key cleanup
with and without handles, and unchanged personal flows. Transaction tests require
a transaction-capable MongoDB and must not skip silently.

Run targeted backend tests after review on the existing CI replica-set MongoDB:

```sh
NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27017/?replicaSet=rs0&directConnection=true' \
  cargo nextest run -p nyxid --profile ci \
  -E 'test(platform_public_) | test(platform_personal_) | test(platform_manager_) | test(platform_bot_runtime_control_service) | test(platform_link_intent_service) | test(public_credentials_never_resolve_ordinary_inventory_or_session_fallback) | test(billing_route_coverage_smoke)'
```

The normal Backend Test CI job also runs these cases and the full backend suite.
No test may silently fall back to standalone MongoDB or skip a transaction gate.
Local `cargo check --tests` establishes type correctness only. Fixtures and mocked
transports cannot establish the upstream's actual containment or deployment.

## Deployment validation and activation

`platform_public_deployment_proofs` holds immutable verification evidence separately
from legacy `platform_public_readiness_records`. The legacy unique candidate index
is retained unchanged; concurrent new/old startups never drop it. Legacy unavailable
records cannot authorize execution and need no privileged migration.

`platform_public_deployment_activations` stores one durable manager-scoped activation
binding and stable policy digest. Validation alone creates proof, never activation.
Explicit admin enable/apply rechecks human and sponsor consent, fences the exact
validated proof and atomically writes activation with revocation/cleanup of existing
public sessions when the stable deployment changes. The UI sends the validated
`public_deployment_digest`; a different proof cannot be substituted at apply time.
Explicit validation always performs current runtime discovery and a paid readiness
handshake, even while an older proof remains unexpired. Only the apply adapter
reuses sufficiently fresh validated evidence to avoid a second immediate paid
probe. A proof with less than the full turn duration plus five seconds remaining
cannot be reused for apply. Validation of a changed build does not activate it.

An unchanged public profile ID can still name a changed runtime, catalog or bound
policy. The enabled settings page therefore offers **Validate and apply public
deployment** even with unchanged form fields. This action adopts the new deployment;
existing senders safely start fresh after old sessions are revoked. Ordinary refresh
keeps activation and sessions unchanged, and each in-flight turn retains its original
proof record. Execution always selects the activated digest and rejects drift;
settlement and exact cleanup remain legal after revocation or expiry.

After seven days, unused drafts, historical candidates and retired activations lose
unreferenced proofs and probes. The current live activation's latest refresh,
retained turns and unfinished cleanup retain the authority they still require.
