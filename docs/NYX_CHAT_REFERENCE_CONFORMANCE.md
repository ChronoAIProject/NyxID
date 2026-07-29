# `eanz17/nyxid-chat` — FE Conformance Review vs the v8 Chat Contract

**Purpose:** verify the claim that the foreign repo
[`eanz17/nyxid-chat`](https://github.com/eanz17/nyxid-chat) implements "the correct structure
we need", where *correct* is defined by the spec, and record the alignment delta plus proposed
changes.

| | |
|---|---|
| **Spec (SSOT)** | `~/Desktop/aelf-frontend-work/docs/nyx-chat-prd.md` — *NyxID Assistant Chat — PRD & NyxID ↔ Aevatar Contract*, Draft v8 (2026-07-17). The repo copy `docs/assistant-chat-prd.md` is a stale v7 — do not use it. |
| **Implementation reviewed** | `eanz17/nyxid-chat` @ `5bf4c38` (*feat(assistant): LLM-triggered in-chat connect cards from the NyxID service catalog*), pulled 2026-07-23. Local clone: `~/Desktop/aelf-frontend-work/nyx-chat`. |
| **Scope** | The frontend chat implementation (`public/app.js`, `public/protocol.js`, `public/blocks.js`), plus `server.mjs` where the BFF shapes what the FE receives. |
| **Related** | `docs/ASSISTANT_STREAM_ALIGNMENT.md` (NyxID vs the reference at `819dc0d` — frame taxonomy, verified 2026-07-20), `docs/CHAT_REWORK_SPEC.md` (the config-fix + revert plan). This doc does **not** restate the frame-by-frame comparison in the first; it answers the structural question the first one deliberately scoped out. |
| **Reviewed** | 2026-07-23 |

---

## 0. Verdict

**`eanz17/nyxid-chat` is a faithful reference for the *deployed* Aevatar surface and for the
NyxID session/proxy identity topology. It is not an implementation of the v8 C1 contract, and
on the contract's single load-bearing rule it does the exact opposite of what the spec requires.**

Two different things have both been called "the contract" in this project, and conflating them
is what makes the premise of this review ambiguous:

| | **Contract L — live** | **Contract T — target** |
|---|---|---|
| Defined by | What prod Aevatar actually speaks today | `nyx-chat-prd.md` Draft v8 |
| Transport | AG-UI frames (`RUN_*`, `TEXT_MESSAGE_*`, `TOOL_*`, `CUSTOM aevatar.*`) | `turn.*` / `message.*` / `block.*` event catalog (§3.7) |
| Message model | opaque markdown string | typed block array, both directions (§3.0) |
| Endpoints | ~7 scope-prefixed Aevatar routes | exactly 3 (§3.1–3.3) |
| Approvals | Aevatar `:approve` / `runs/{id}:resume` | NyxID `POST /approvals/requests/{id}/decide` only (§4.3) |
| Implemented by | `nyxid-chat` ✅, NyxID FE ✅ | **nobody — neither side** |

`nyxid-chat` is an excellent Contract-L client. The PRD's own Appendix C already says the shipped
NyxID branch is also a Contract-L client and lists G1–G6 as the expected-fail gaps to Contract T.
So the honest finding is not "the foreign repo is wrong" — it is that **the foreign repo is not
the thing that closes G1–G6, and adopting its structure would deepen the gaps rather than close
them.** Specifically, its newest feature (`5bf4c38`) closes G5 (in-chat connect wiring) by a
mechanism that hard-violates §3.0.

**The single most important finding:** the connect cards that make this repo look
contract-shaped are produced by **prompting the LLM to emit a fenced markdown code block and
regex-parsing it out of the assistant's prose** (`server.mjs:602-606` teaches the LLM the
format; `public/blocks.js:7,28-98` parses it back out). PRD §3.0 names this the load-bearing
rule and forbids it in both directions:

> **Content boundary (the load-bearing rule):** every message, in both directions, is a **typed
> block array** … Structure never rides inside markdown; **the FE never parses text to derive an
> action, card, or identifier**, and Aevatar never derives an action from user prose when a typed
> block exists for it.

This is not a stylistic difference. A model that hallucinates a slug, emits two fences, or drops
the closing fence changes what the UI offers to connect. `blocks.js:85-88` does validate the slug
shape and `buildConnectCardBlock` re-resolves display data from the real catalog snapshot — so the
blast radius is bounded — but the *existence, count and target of a connect card* is still model
output parsed out of prose.

---

## 1. What the repo actually is

A standalone Node BFF (`server.mjs`, 1290 lines) plus a vanilla-JS single-page client
(`public/`, ~3.5k lines). It is a demo/reference harness, not a component library — there is no
React, no shared types with NyxID, and nothing importable.

**FE → BFF calls** (`public/app.js`): `/api/demo/config` `:373` · `/api/auth/session` `:521` ·
`/api/auth/services` `:551` · `/api/nyxid/connectors` `:577` · `/api/nyxid/keys` `:656` ·
`/api/auth/logout` `:1079` · `/api/demo/health` `:1205` · `/api/demo/conversations[/{id}]`
`:1305-1306,1328,1435,1544` · `/api/demo/chat` `:1636` · `/api/demo/approve` `:2358`.

**BFF → Aevatar** (`server.mjs`): list `/api/scopes/{scope}/chat-history` `:771` · detail
`/api/scopes/{scope}/chat-history/conversations/{actor}` `:805` · create
`…/nyxid-chat/conversations` `:979,993` · stream `…/nyxid-chat/conversations/{actor}:stream`
`:1035-1036` · approve `…:approve` `:1075-1076` · workflow resume `…/runs/{runId}:resume`
`:1100-1101` · delete `…/nyxid-chat/conversations/{actor}` `:804` · workflow surface `/api/chat`
`:1018`.

Where it is genuinely authoritative — and where `docs/CHAT_REWORK_SPEC.md` correctly treats it as
the reference — is the **identity topology**: HttpOnly `nyx_session` → BFF → NyxID proxy, with the
proxy (never the BFF, never the browser) injecting `X-NyxID-Identity-Token` /
`X-NyxID-Delegation-Token` (`README.md:19-30,169-186`). The required `aevatar` catalog row
(`identity_propagation_mode: "jwt"`, `identity_jwt_audience: "urn:aevatar:api"`,
`inject_delegation_token: true`, `forward_access_token: false`) is stated at `README.md:89-101`
and matches the prod config fix already recorded in `CHAT_REWORK_SPEC.md`. That part stands.

---

## 2. Conformance matrix

| Spec ref | Requirement | Status | Evidence |
|---|---|---|---|
| §3.1–3.3 | Exactly three C1 endpoints: list, detail, message POST | **DIVERGES** | 7 upstream Aevatar routes + a separate `:approve` and `runs:resume` (`server.mjs:771,805,979,1035,1075,1100,804`) |
| §3.0 | Typed block array in **both** directions; FE never parses text for structure | **DIVERGES (blocker)** | Cards parsed from a ` ```nyxid:connect ` markdown fence (`blocks.js:7,28-98`); the LLM is *prompted* to emit it (`server.mjs:602-606`) |
| §3.0 | Client blocks (`card_action`, `control`) posted as typed blocks | **ABSENT** | Message POST body is `{prompt, sessionId, actorId, attachment}` (`app.js:1640-1646`) — a string, no blocks |
| §3.0 | `client_msg_id` idempotency | **ABSENT** | no such field anywhere in `app.js` / `server.mjs` |
| §3.0 | `schema_version` on every envelope | **ABSENT** | not present |
| §3.0 | Unknown block types → neutral "unsupported content" shell | **PARTIAL** | unknown *frames* are skipped (`app.js:1865-1866`) — silently, with no shell; no block model to be unknown about |
| §3.0 | Error envelope `{error:{code,message}}` + the named cases | **PARTIAL** | ad-hoc `{code,message,serviceId,…}` mapping (`app.js:1729-1744`); no `turn_active` / `client_msg_id_conflict` / `message_too_large` |
| §3.5 `text` | Prose block | **PARTIAL** | prose exists, but as an opaque string, not a block (`protocol.js:321-333`) |
| §3.5 `connect_card` | Typed server block | **DIVERGES** | correct *shape* is built — but client-side, from a parsed fence (`blocks.js:104-133`) |
| §3.5 `run` | Step ledger block | **ABSENT** | steps are inspector-panel rows derived from `STEP_*`/`TOOL_*` frames (`app.js:2211-2240,2661-2700`), not a transcript block |
| §3.5 `approval_card` | `approval_request_id`, `approval_mode`, `expires_at`, `decision_channel` | **DIVERGES** | card built from `TOOL_APPROVAL_REQUEST` with `toolName`/`prompt`/`argumentsJson` (`app.js:2302-2330`); no NyxID approval id, no expiry countdown, no channel |
| §3.5 `artifact` | Typed artifact block | **ABSENT** | only `MEDIA_CONTENT` → an `<img>` (`app.js:2399-2415`) |
| §3.5 | `requested_scopes` drives a scoped OAuth initiate | **ABSENT** | parsed (`blocks.js:94-96`), copied onto the block (`:119`), then **never read** — dead field |
| §3.7 | `turn.*`/`message.*`/`block.*` event catalog | **DIVERGES** | AG-UI vocabulary (`protocol.js:61-126`; dispatch `app.js:1746-1867`) |
| §3.7 | Per-turn monotonic `cursor`, dedup, whole-field patch merge | **ABSENT** | no cursor, no dedup, no patch semantics; text is append-only (`app.js:2255-2265`) |
| §3.8 | No raw workflow telemetry in the browser | **DIVERGES** | `aevatar.raw.observed` normalized and retained (`protocol.js:178-211`) and rendered verbatim into a DOM event log (`app.js:2701-2721`, `safeJson(event.raw)`) |
| §3.4 | Reconnect: detail GET + poll while `active_turn` | **ABSENT** | history is fetched only on explicit conversation switch (`app.js:1435`); a dropped stream ends the turn client-side (`app.js:1652-1661`) |
| §3.4 / A1 | Reload re-renders cards in final state | **PARTIAL (accidental)** | works *only because* cards are text — history is re-parsed through the same fence splitter (`app.js:1520` → `renderAssistantSegments`); live card state (`connected`, error, key id) is lost, `blocks.js:124` recomputes from the connectors snapshot |
| §3.4 / F11 / A4 | Cancel = `control: cancel`; open blocks terminalize | **DIVERGES** | Stop is a client-side `AbortController.abort()` only (`app.js:2766-2777`); the upstream turn keeps running — the UI says so (`app.js:1679`) |
| §4.3 / G3 | Exactly one decision plane: NyxID `/approvals/requests/{id}/decide` | **DIVERGES** | decides via Aevatar `:approve` / `runs:resume` (`app.js:2358` → `server.mjs:1075,1100`); NyxID's decide endpoint is never called |
| §4.3 F3 | OAuth: placeholder key → scoped initiate → `card_action` | **ABSENT** | deep-links to NyxID `/keys?slug=…` in a popup, then manual "refresh" (`app.js:619-643,678-695`) |
| §4.3 F4 | api_key paste; secret browser→NyxID only | **PARTIAL** | secret routing conforms (`app.js:645-676` → `server.mjs:541-568`, never enters chat content) — but the returned `KeyResponse` is discarded (`app.js:665-669`), so no `key_id` is ever attached |
| §4.3 F5 | device_code: show `user_code` + URL | **ABSENT** | `device_user_code`/`device_verification_url` hardcoded `null` (`blocks.js:122-123`); device_code falls into the generic deep-link branch (`app.js:885-897`) |
| §4.3 | `requested_scopes ⊆ catalog scope set`, else refuse | **ABSENT** | no scope validation anywhere |
| §4.3 | Extend agent-key `allowed_service_ids` on connect | **ABSENT** | not implemented (n/a to this architecture — no per-user agent key) |
| §3.6 | Markdown subset, **no raw HTML** | **DIVERGES** | `DOMPurify.sanitize(…, {USE_PROFILES:{html:true}})` (`app.js:2289-2292`) permits the full sanitized-HTML profile |
| §3.6 | Links `https:`/`mailto:` only | **PARTIAL** | `http:` also gets `target=_blank` (`app.js:2295`); other schemes left to DOMPurify's defaults |
| §3.6 | No autoloaded remote images except catalog `icon_url` | **PARTIAL** | `icon_url` gated on `^https://` (`app.js:768`) — but `renderMedia` autoloads any `https:` image (`app.js:2399-2409`) and the CSP permits `img-src 'self' data: https:` (`server.mjs:1185-1193`), so markdown-embedded remote images load too |
| §3.3 | v1 rejects reserved `image`/`file` input blocks (400) | **DIVERGES** | composer accepts any file ≤ 5 MiB (`app.js:20,2823-2842`); BFF maps it to `image`/`audio`/`video`/`file` input parts (`server.mjs:946-972`) |
| §3.0 | `?before=&limit=` pagination on both GETs | **ABSENT** | history URL carries only `surface`/`workflow` (`app.js:1299-1307`); BFF forwards no pagination (`server.mjs:766-795`) |
| §3.0 | 32 KiB serialized message cap → `413 message_too_large` | **DIVERGES** | generic 10 MiB BFF cap (`server.mjs:13-14,301-317`); composer capped at 12 000 chars (`index.html:172`) |
| §8 | DELETE conversation is v1.1 backlog, not v1 | **DIVERGES** | implemented on both sides (`app.js:1535-1556`, `server.mjs:797-826`) |
| §4.1 / F12 | Provider readiness via `GET /api/v1/llm/status` | **ABSENT** | health probes Aevatar `/api/capabilities` + Ornn instead (`server.mjs:1119-1153`) |
| §7 A8 | All missing connections reported at once | **DIVERGES** | delegated to prompt compliance (`server.mjs:602-606`); catalog injection is best-effort and silently skipped on failure (`server.mjs:1025-1033`) |
| §3.6 | Display strings never become action inputs | **PARTIAL** | actionable slug comes from model prose, but is shape-validated (`blocks.js:85-88`) and re-resolved against the catalog (`blocks.js:105-108`) |
| §3.6 | Artifact MIME allowlist, size cap, `Content-Disposition` | **N/A** | no artifact plane |
| §1 | NyxID persists/logs no chat content | **CONFORMS** | BFF streams through; secrets + reasoning redacted (`protocol.js:223-255`) |

---

## 3. Findings

### BLOCKER

**F1 — Structure is carried inside markdown prose, in both directions.**
Spec §3.0 forbids exactly this. The BFF appends a `[[NYXID_CONTEXT]]` block to the user's prompt
that teaches the model to emit ` ```nyxid:connect ` with a JSON body (`server.mjs:580-609`); the
FE regex-splits assistant text on that fence and turns matches into cards
(`blocks.js:7,28-76,78-98`). Consequences: a card's existence and target are model output; a
dropped closing fence renders a placeholder forever (`blocks.js:56-60`); prompt-injected content
in a tool result can synthesize a connect card; and the whole mechanism costs prompt tokens on
every single turn (up to 60 catalog rows, `server.mjs:574`).
*Do not port this to NyxID.* If a stopgap is needed before Aevatar emits typed blocks, it belongs
behind an explicitly-named adapter (see P2), not in the renderer.

**F2 — Two decision planes for approvals (PRD Appendix C, G3).**
Spec §4.3 binds in-chat Approve/Deny to `POST /api/v1/approvals/requests/{id}/decide`, with the
`approval_decided` `card_action` as an optional latency nudge only. The repo decides through
Aevatar (`app.js:2332-2397` → `server.mjs:1075-1076` `:approve`, or `:1100-1101` `runs:resume`).
NyxID's approval row is never touched, so an in-chat decision and a Telegram/mobile decision are
not the same decision, `decided_via` never converges, and the NyxID audit chain does not record
the in-chat outcome. This is a correctness and auditability defect, not just a shape mismatch.
The repo's own README concedes the boundary: *"Aevatar approval 卡片不是最终安全边界，proxy
policy 必须独立执行"* (`README.md:186`).

### MAJOR

**F3 — Cancel does not cancel.** `cancelRun` aborts the browser's `fetch` and nothing else
(`app.js:2766-2777`). No `control: cancel` reaches Aevatar; the turn continues server-side; open
cards never terminalize (violates §3.4 and guarantee A4). The UI is at least honest about it
(`app.js:1679`), but "Stop" that doesn't stop is a user-trust problem, and it means a run can
still be posting to a downstream after the user believed they stopped it.

**F4 — No reconnect path.** §3.4 makes the detail GET the reconnect mechanism, polled at 1–3 s
while `active_turn` is non-null. Here, stream drop terminalizes the run client-side with an
advisory message (`app.js:1652-1661`); reload loses everything not persisted in the Aevatar
history string. Combined with F3, a mid-turn reload leaves the user with no view of a run that is
still executing.

**F5 — Raw engine telemetry reaches the browser (§3.8).** `aevatar.raw.observed` envelopes are
normalized and kept (`protocol.js:178-211`), then dumped verbatim into a DOM event log
(`app.js:2701-2721`). `redact()` (`protocol.js:223-255`) strips secret-keyed fields and
`reasoningContent`, but workflow YAML, system prompts, actor ids and lease/kernel state are not
secret-keyed and pass straight through — which is precisely what §3.8 was written to prohibit.
Acceptable in a debug harness; not acceptable in the product surface.

**F6 — The `run` ledger and `artifact` blocks do not exist as transcript content.** Step state
lives in a side inspector panel (`app.js:2211-2240,2661-2700`) and is lost on reload; artifacts
are only inline images (`app.js:2399-2415`). F7 (execute-with-steps) and F9 (artifacts) are
therefore unimplemented, and B1/B2/B3 in Appendix B cannot be asserted against this client.

**F7 — OAuth and device_code connect are not in-chat.** §4.3's OAuth flow (placeholder key →
scoped initiate with `scope_override` → `card_action`) is replaced by a deep link to NyxID
`/keys?slug=…` plus a manual "I've connected, refresh" button (`app.js:619-643,678-695`).
`requested_scopes` is parsed and then never used (`blocks.js:94-96,119`), so scope narrowing (P3
in Appendix B) cannot happen. `device_user_code` / `device_verification_url` are hardcoded `null`
(`blocks.js:122-123`), so F5 (device code) has no card. Only the api_key modality (F4) is
genuinely in-chat.

### MINOR

**F8 — `USE_PROFILES: {html: true}` permits raw HTML.** §3.6 says the text block is a markdown
subset with *no* raw HTML. `app.js:2289-2292` sanitizes but does not restrict to a subset;
`FORBID_ATTR: ["style"]` is a partial mitigation. Tighten to an explicit tag/attr allowlist.

**F9 — Link scheme policy is looser than the spec.** §3.6 limits links to `https:`/`mailto:`;
`app.js:2293-2299` treats `http:` the same as `https:` and defers everything else to DOMPurify.

**F10 — Auto-retry after connect contradicts the repo's own stated rule.** The README says a
request is only re-submitted on an explicit user click, precisely because the run may have
partially executed (`README.md:46-49`), and `docs/ASSISTANT_STREAM_ALIGNMENT.md` G1 records that
as the *desired* behavior. But `scheduleConnectCardRetry` (`app.js:710-727`) auto-re-sends the
original prompt 900 ms after a card flips to connected. The `AUTHORIZATION_REQUIRED` path is
explicit-retry; the new connect-card path is not. One of the two is wrong; per the stated
rationale, the auto-retry is.

**F11 — No `client_msg_id` / `schema_version`.** No post idempotency and no forward-compat
version marker (§3.0). Low impact at demo scale; both are cheap to add and both are contract
surface.

**F12 — The whole assistant turn is one string, re-parsed and re-rendered on every delta.**
`appendText` (`app.js:2255-2263`) appends to a single `assistantText` accumulator and then calls
`renderAssistantSegments` over the *entire* message — full fence split plus full markdown
re-render — for every chunk. Quadratic in message length, and it clears and rebuilds the DOM each
time (`blocks.js:28`, `app.js:922`); card DOM survives only via the `cardElements` registry keyed
by `connect:{slug}:{ordinal}` (`blocks.js:91`). §3.7's `block.delta` (append to one block) and
`block.updated` (whole-field patch keyed by a stable server-assigned `block_id`) exist precisely
to avoid this.

**F13 — The fence splitter has no code-fence nesting state.** `splitMessageSegments`
(`blocks.js:41-73`) scans line-by-line for `^```\s*nyxid:connect\s*$` and never tracks whether it
is already inside an ordinary fenced code block. So a ` ```nyxid:connect ` line appearing *inside*
a quoted markdown block — a tool result, a pasted log, the model echoing its own instructions —
is parsed as a live connect card. The BFF asks the model not to repeat the context block
(`server.mjs:606`), but that is an instruction, not an enforcement. This is the concrete
prompt-injection path implied by F1.
*Blast radius, stated precisely:* not credential exfiltration — the pasted secret still goes
browser→NyxID (`app.js:656`) and an unknown slug degrades to a generic `/keys` link
(`blocks.js:105-108`, `app.js:621`). But for any of the ~60 catalog slugs an injected fence
renders a legitimate-looking card with an attacker-chosen justification string and, for `api_key`
services, an inline credential input — i.e. it can induce a user to connect a service they never
intended to. That is a real phishing surface, and it exists *only* because structure is parsed
out of prose.

---

### Added after reconciliation (§6)

**F14 — MAJOR. The 120 s progress watchdog kills exactly the waits the contract requires.**
When 120 s pass with only keepalives, the BFF synthesizes a `RUN_ERROR /
UPSTREAM_PROGRESS_TIMEOUT` frame and aborts the upstream stream
(`server.mjs:895-910`). §3.4 says the opposite: *"Aevatar MAY close an idle stream during a long
`waiting` period (external gates can take hours); the FE falls back to polling."* A user who
takes three minutes to fetch a Lark bot token, or an approver who decides on Telegram after
lunch, gets their run aborted. The watchdog is the right idea for a *dead* stream and the wrong
one for a *waiting* one — the two are only distinguishable if the FE has a `turn.status:
"waiting"` signal and a detail-poll fallback, which it does not (F4). This qualifies adoption
item 4 below.

**F15 — MINOR. v1-reserved multimodal input is accepted.** §3.3 reserves `image`/`file` client
blocks for v1.1 and has v1 reject them 400. The composer accepts any file up to 5 MiB
(`app.js:20,2823-2842`) and the BFF maps it to `image`/`audio`/`video`/`file` input parts
(`server.mjs:946-972`). Fine as a product feature; wrong as a reference for a v1 client.
(Tracked on the NyxID side as G4 in `ASSISTANT_STREAM_ALIGNMENT.md`, correctly deferred there.)

**F16 — MINOR. The catalog payload cannot support scope validation even if it were attempted.**
`deriveConnectors` projects catalog entries down to slug/name/description/icon/authKind/apiKey
fields and drops scope sets and provider config ids (`server.mjs:498-510`). So §4.3's
`requested_scopes ⊆ catalog scope set` check and the `provider_config_id` needed for a scoped
OAuth initiate are not merely unimplemented — they are unavailable to the FE. Any adapter built
on this shape (P2) must widen the BFF projection first.

**F17 — MINOR. Connect-card UI implements four of six card states.** `connectCardPill`
(`app.js:749-757`) labels `needs_connection`, `waiting_for_user`, `connected`, `error`. There is
no `waiting_for_provider` (the observable-gate-in-progress state) and no `timed_out` — so the
waiting-deadline behavior in §3.5 and Appendix B's partial-outcome assertion have no rendering.

---

## 4. Worth adopting

1. **The identity topology, verbatim.** Browser HttpOnly session → BFF → NyxID proxy, with the
   proxy alone minting `X-NyxID-Identity-Token` / `X-NyxID-Delegation-Token`, and the BFF
   explicitly refusing to forward browser-supplied copies of those headers
   (`README.md:169-186`). This is already NyxID's prod config and should stay pinned.
2. **`redact()`'s recursive field-level walk** (`protocol.js:223-255`), including
   `reasoningContent` → `[not displayed]` and JSON-in-string re-serialization. NyxID redacts the
   serialized display string instead; the two are complementary and both are worth having
   (already noted as divergence 3 in `ASSISTANT_STREAM_ALIGNMENT.md`).
3. **`buildConnectCardBlock` re-resolving display data from a live catalog snapshot**
   (`blocks.js:104-133`) rather than trusting the payload. Right instinct, applied to the wrong
   input source. Keep the instinct; change the input to a typed block.
4. **The 120 s no-progress watchdog — the idea, not the rule as written** (`README.md:123-124`,
   `server.mjs:895-910`). NyxID enforces the same budget client-side and the open `G-hang` item
   wants it. **But see F14:** as implemented it aborts legitimate long waits. Adopt it only
   alongside a `waiting` turn state that suspends the timer, or scope it to "no frames at all"
   rather than "no frames other than keepalives".
5. **`POST /api/demo/health` as a per-component route probe.** Answers "is chat down or just
   slow?" in-product — still open as G5 in `ASSISTANT_STREAM_ALIGNMENT.md`.
6. **Per-conversation `AbortController` sets with view isolation** (`app.js:246-334`,
   `test/app-concurrency.test.mjs`) — clean multi-conversation concurrency model.
7. **The connect-card visual design** (`design/nyxid-assistant-shell.html`, `styles.css`): brand
   header + status pill + three-step wizard + inline action zone. The *rendering* is good; only
   the data source is wrong.

---

## 5. Proposed changes

Ordered. **P0–P1 are decisions, not code** — the rest is wasted work without them.

**P0 — Name which contract we are building against, in writing.**
The v8 C1 contract requires Aevatar to emit typed blocks over the 3-endpoint surface. Nobody
implements it. Until Aevatar commits to a date, "align NyxID to the spec" is unbuildable and
"align NyxID to the reference" means shipping F1–F7 into the product. Pick one:
(a) drive Contract T with Aevatar and treat `nyxid-chat` as a Contract-L artifact only;
(b) formally re-baseline the PRD onto the AG-UI transport and rewrite §3 accordingly;
(c) run an explicit adapter (P2) while (a) is negotiated. *Recommendation: (a) + (c).*

**P1 — Reject the fence mechanism as a target architecture (F1).**
Record it as a demo-only stopgap. Do not port `blocks.js` into `frontend/src/`. If Aevatar cannot
emit `connect_card` blocks soon, get it to emit them as an **SSE `CUSTOM` frame with a named
payload** — same effort upstream, no prose parsing downstream, and it satisfies §3.0's "typed
block" intent without waiting for the full 3-endpoint rework.

**P2 — If a stopgap is needed, isolate it behind an adapter boundary (F1).**
One module (`lib/assistant/legacy-fence-adapter.ts`) whose only job is
`assistantText → ContentBlock[]`, feature-flagged, deleted when Aevatar emits typed blocks. The
renderer keeps consuming `ContentBlock[]` only. Carry over `blocks.js:85-88` slug validation and
`blocks.js:105-108` catalog re-resolution, and add the §4.3 `requested_scopes ⊆ catalog scopes`
check that the repo omits.

**P3 — Keep NyxID's approval decisions on the NyxID plane (F2).**
Do not copy `/api/demo/approve`. In-chat Approve/Deny calls
`POST /api/v1/approvals/requests/{id}/decide` with an `idempotency-key`; Aevatar learns the
outcome from its own status poll. This is Appendix C G3 and it is already the NyxID side's
stated target — this review is evidence *not* to regress toward the reference here.

**P4 — Make Stop real (F3).** Post a cancel to Aevatar (a `control: cancel` block under Contract
T; whatever the `:stream` surface accepts under Contract L) and terminalize every open card
locally. If Aevatar has no cancel verb, that is an upstream ask — file it; do not ship a Stop
button that only stops the browser.

**P5 — Add the reconnect loop (F4).** On stream drop or reload, fetch conversation detail and
poll while a turn is active. Under Contract L, `chat-history/conversations/{actor}` is the
closest available detail GET; the gap to close is that it returns text, so live card/run state
is not recoverable — which is itself the argument for P0(a).

**P6 — Gate the raw event log (F5).** Behind a dev-only flag, never in the production bundle.
Keep the existing `redact()` on top of that, and add workflow/system-prompt keys to the
redaction set so a leaked debug build still doesn't emit system prompts.

**P7 — Tighten the markdown renderer (F8, F9).** Explicit tag/attr allowlist instead of
`USE_PROFILES: {html: true}`; restrict link schemes to `https:`/`mailto:`.

**P8 — Resolve the retry contradiction (F10).** Make connect-card retry explicit, matching the
`AUTHORIZATION_REQUIRED` path and the repo's own stated rationale. Applies to NyxID's G1 work
too — when we close G1, close it as explicit re-send, not auto-retry.

**P9 — Add `client_msg_id` + `schema_version` to the post envelope (F11).** Cheap, and both are
contract surface we will need regardless of which contract wins P0.

---

## 6. Reconciliation — independent Codex review

A second audit of the same repo against the same spec was run in parallel by Codex CLI
(`codex-cli 0.144.6`, `gpt-5.1-codex-max`), given the same inputs and explicitly instructed not
to look for this document. Its raw output is preserved at `nyx-chat-codex-audit-raw.md`.
Everything below is reconciled: each Codex-only claim was re-verified against the source before
being folded in.

### 6.1 Agreement

Both passes reached the same verdict independently — **not a conformant v8 implementation; a
different architecture** — and independently flagged the same blockers:

| Area | Both found |
|---|---|
| Endpoint surface | ~7 upstream Aevatar routes, not 3 (identical route maps) |
| Content boundary | markdown-fence card parsing = the §3.0 violation; both rate it BLOCKER |
| Stream events | AG-UI vocabulary, no `cursor`, no block reducer |
| Telemetry | `aevatar.raw.observed` rendered into the browser (§3.8) |
| Approvals | decided via Aevatar `:approve`, not NyxID decide — G3 split-brain |
| Connect | OAuth/device_code absent; only api_key is in-chat |
| History / reconnect | detail returns strings; no `active_turn` poll |
| Cancel | client-side abort only; blocks never terminalize |
| §3.0 fields | no `client_msg_id`, no `schema_version`, wrong error envelope |
| Rendering | `USE_PROFILES:{html:true}` + `http:` links accepted |
| Missing blocks | no `run`, no `artifact` |
| Worth adopting | session/BFF identity boundary, `redact()`, catalog enrichment, secret routing, card visual design |

Convergence on the blockers from two independent passes is the main reason to treat §0's verdict
as settled rather than as one reviewer's read.

### 6.2 Codex-only findings — verified and adopted

Each was re-checked against source before inclusion; all six confirmed.

| Codex finding | Verified at | Folded in as |
|---|---|---|
| The `KeyResponse` from the api_key POST is **discarded** — no `key_id` ever attached | `app.js:665-669` | matrix row §4.3 F4 downgraded **CONFORMS → PARTIAL** |
| 120 s keepalive watchdog emits `RUN_ERROR` and aborts upstream, killing long legitimate waits | `server.mjs:895-910` | **F14** + adoption item 4 qualified |
| v1-reserved multimodal input is accepted and mapped to input parts | `app.js:20,2823-2842`; `server.mjs:946-972` | **F15** + matrix row §3.3 |
| BFF catalog projection drops scope sets and provider ids, so scope validation is *impossible*, not just absent | `server.mjs:498-510` | **F16**, and P2 amended |
| CSP `img-src 'self' data: https:` permits remote images; `renderMedia` autoloads them | `server.mjs:1185-1193`; `app.js:2399-2409` | matrix row §3.6 images **CONFORMS → PARTIAL** |
| No pagination; 10 MiB cap vs the binding 32 KiB; DELETE shipped despite being v1.1; health probes `/api/capabilities` not `/llm/status` | `app.js:1299-1307`; `server.mjs:13-14,301-317,797-826,1119-1153` | four new matrix rows |
| Only 4 of 6 connect-card states are rendered | `app.js:749-757` | **F17** |

### 6.3 This pass only — not in Codex's report

| Finding | Why it matters |
|---|---|
| **F13** — the fence splitter tracks no code-fence nesting state (`blocks.js:41-73`), so a ` ```nyxid:connect ` inside a quoted block or tool result renders a live card | turns F1 from a design objection into a concrete prompt-injection → connect-phishing path |
| **F10** — auto-retry (`app.js:710-727`) contradicts the repo's own explicit-retry rule (`README.md:46-49`) and our G1 target | an internal inconsistency in the reference; tells us which behavior to copy when we close G1 |
| **F12** — the full message is re-split and re-rendered on every delta (`app.js:2255-2263`) | quadratic render; also the concrete argument for why `block_id` + `block.delta` exist |
| **Contract L vs Contract T framing** (§0) | Codex graded purely against v8 and concluded "rewrite everything" — correct, but it does not surface that *nobody* implements v8, including NyxID. Without that, the review reads as "the foreign repo is broken" instead of "we have an unresolved baseline decision" (**P0**) |
| Cross-links to `ASSISTANT_STREAM_ALIGNMENT.md` and `CHAT_REWORK_SPEC.md` | keeps this from re-litigating the frame taxonomy already settled on 2026-07-20 |

### 6.4 Disagreements

**None material.** Two grading differences, both resolved in Codex's favour and corrected above
(§4.3 F4 and the §3.6 image policy — this pass had graded both CONFORMS on FE-side evidence and
missed the discarded `key_id` and the server CSP respectively).

One difference of *emphasis* is worth recording rather than resolving: Codex's proposed changes
(its §5, items 1–12) are a complete v8 build-out — replace the transport, the message model, the
reducer, the approval plane, connect choreography, reconnect, cancel, artifacts, and the protocol
guards. That list is correct and is effectively the work behind P0(a)+(b). This document's P0–P9
deliberately stops short of scheduling it, because the ordering question — *which contract are we
building* — is unanswered, and Codex's list is only actionable after P0 resolves. **Read Codex's
§5 as the P0(a) work-breakdown**; it is reproduced verbatim in `nyx-chat-codex-audit-raw.md`
and should be lifted into a tracking issue if and when P0 lands on (a).

### 6.5 Verification limits both passes share

Neither pass ran an authed session against prod NyxID/Aevatar; both are static reviews. Codex
additionally attempted the repo's test suite: 14 dependency-free `blocks`/`protocol` tests pass
and `node --check` passes on `server.mjs`, `public/app.js`, `public/protocol.js`, but the `jsdom`
and server-integration suites could not run in its sandbox (`jsdom` not installed; `EPERM` binding
`127.0.0.1`). So no behavioral test evidence exists on either side — see §7.5.

---

## 7. Open questions

1. **Does prod Aevatar have a cancel verb on `:stream`?** Neither client uses one. If it does not,
   P4 is blocked upstream (relevant to F3 and to NyxID's own Stop button).
2. **Is `5bf4c38`'s fence mechanism intended as a demo or as the product direction?** The commit
   message ("LLM-triggered in-chat connect cards from the NyxID service catalog") reads as a
   feature, not a stopgap. P0/P1 need this answered by whoever owns that repo.
3. **What is the Aevatar-side timeline for typed blocks?** Determines whether P2's adapter is
   worth building at all.
4. **Unverified live behavior.** This review is static — no authed run against prod Aevatar was
   performed, so claims about which frames actually arrive rest on the code's handling and on the
   fixtures noted in `ASSISTANT_STREAM_ALIGNMENT.md` G3 (a text-only capture). Whether
   `TOOL_APPROVAL_REQUEST` / `AUTHORIZATION_REQUIRED` ever fire on the deployed path is still
   unconfirmed on both sides.
5. **No behavioral test evidence.** The Codex pass ran what it could: 14 dependency-free
   `blocks`/`protocol` tests pass, `node --check` passes on all three JS entry points, but the
   `jsdom` and server-integration suites did not run (missing dep / sandbox `EPERM`). Both passes
   are therefore static reads of source. Worth a clean local `npm test` before anyone cites the
   repo's test coverage as assurance.
6. **P2's adapter may be more expensive than it looks.** F16 shows the BFF's catalog projection
   drops the scope sets and provider ids that §4.3's validation needs, so "just port the card
   renderer" also means widening the connectors payload. Scope P2 with that included.
