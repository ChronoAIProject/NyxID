# In-Chat Connector Connection — FE Plan

**Goal:** the user connects a missing connector **from inside the chat transcript**. Assistant says
"I need GitHub" → an actionable card renders in the thread → user connects there → the task
continues.

**Shape derived from:** `eanz17/nyxid-chat` @ `5bf4c38` (`~/Desktop/aelf-frontend-work/nyx-chat`),
which has this working today.

**Status:** in progress. **W2, W3, W4, W4b, W4c, W5, W6, W8 landed** (2026-07-23) — see §5.
Remaining: W0/W1 (Aevatar), W4d, W7.

**Option B was built** (the in-text marker), structured so Option A drops in without touching the
renderer — see §1. W0 is still worth asking, but is no longer blocking: if Aevatar can emit typed
blocks, `textToBlocks` gains a branch and `connect-fence.ts` is deleted.

**Interim decision (2026-07-23):** no browser popups. Anything that would need one is built as a
NyxID modal presenting the pending task, with live status streamed into the connect card.

### Given constraints (not up for redesign)

1. **NyxID's FE calls Aevatar for two things only: send-message (+ its stream), and get-history.**
   Nothing else.
2. **Aevatar reads NyxID `GET /api/v1/keys` itself** to know which slugs are connected and which
   are missing. The FE does not tell it, and does not inject a catalog into the prompt.
3. All connect actions are **FE → NyxID's own API** (`/keys`, `/catalog`, `/providers/…`).
   Unrestricted by (1).
4. Skills/tools discovery is out of scope — backend concern.

---

## 1. The recommendation, and why

> **Revised after the independent Codex pass (§8), which disagreed on the wire encoding. The
> reconciled position is below; the disagreement is recorded rather than smoothed over.**

**The FE consumes a typed `ConnectCardContentBlock` — the type we already ship. That is settled and
both passes agree on it. The only open decision is how Aevatar encodes the card on the wire, and it
is gated on one question about Aevatar's storage:**

| | **Option A — typed block** (preferred end state) | **Option B — Aevatar-authored marker in text** (fallback) |
|---|---|---|
| Aevatar emits | a `connect_card` block in the message's block array, on the SSE **and** in history | its normal text, with a `nyxid:connect` fence it writes **programmatically** |
| Aevatar change | message storage becomes a block array + history envelope | text only — no storage change |
| Survives reload | yes, natively | yes, because the card *is* the text |
| FE cost | transport hydrates typed blocks | transport parses the fence once, emits the same typed block |
| Risk | larger upstream lift, unknown timeline | in-text encoding; must never be LLM-authored |

**Take Option A if Aevatar can change history storage in this cycle. Otherwise ship Option B now
and migrate — the two are forward-compatible, because in both cases the transport emits the same
typed block and the renderer never learns which encoding was used.**

**The critical amendment to Ean's design:** in his prototype the *LLM* authors the fence, because
the BFF prompts it to (`server.mjs:602-606`). That is the brittle part — and it is separable. Under
constraint 2 Aevatar already knows the missing slugs from its own `/api/v1/keys` read, so **Aevatar
writes the marker programmatically**. It is then a transport encoding for a structured payload
inside a text-only channel, not model output being parsed for structure. Codex's objection to the
fence is correct as aimed at Ean's version and mostly dissolves against this one.

### Why the encoding question is gated on storage

Constraint 1 is what makes this the deciding factor. Our history projection is:

```
historyEntryToMessage()  →  blocks: [{ type: "text", text }]      aevatar-transport.ts:382-402
```

Aevatar's history returns **one `content` string per message** (`aevatar-transport.ts:233-241`).
Under the two-endpoint rule we cannot add a typed side-channel to fetch card state on reload —
there is no third endpoint to call. So:

> **Anything not carried inside the message text does not survive a reload, unless Aevatar changes
> what history returns.**

That is why the encoding choice reduces to a storage question. Ean's fence round-trips for free —
his `renderStoredMessage` (`app.js:1514-1526`) re-parses stored content and the card comes back.
Our current connect card is built from a live SSE frame (`aevatar-transport.ts:1593`, `:2002-2037`)
and is therefore **destroyed on reload** — fatal for this feature specifically, because "user
reloads while going to fetch their API key" is the single most likely moment in the entire flow.

Either option resolves the content-boundary objection from `NYX_CHAT_REFERENCE_CONFORMANCE.md` §F1:
that objection was that *the renderer* parses prose. Under B the parse happens once at the
transport seam and the renderer stays typed-only — same boundary, enforced one layer earlier, ~40
lines plus the nesting fix.

**Not proposed under either option:** the prompt-injection half of Ean's design
(`server.mjs:580-609`). Constraint 2 says Aevatar already knows what is connected. Injecting a
60-row catalog into every prompt is pure cost with no benefit. Drop it. Both passes agree.

---

## 2. How Ean's prototype does it, stage by stage

| Stage | Mechanism | Where |
|---|---|---|
| **Signal** | Assistant emits a fenced block in its normal text: ` ```nyxid:connect ` + JSON body `{catalog_slug, reason, requested_scopes?}` | `blocks.js:7` |
| **Detect** | `splitMessageSegments` line-scans assistant text → `{kind:"text"}` / `{kind:"connect_card"}` / `{kind:"pending_card"}`. Unterminated fence while streaming → held back as `pending_card` so half-written JSON never flashes | `blocks.js:28-76` |
| **Validate** | Slug must match `^[a-z0-9][a-z0-9_-]{0,80}$`; bad JSON or bad slug degrades to a plain code block | `blocks.js:78-88` |
| **Resolve** | `buildConnectCardBlock(segment, connectors)` joins the slug against a live connectors snapshot to get `service_name`, `icon_url`, `auth_kind`, `api_key_url`, `api_key_instructions`, `docs_url`, and current connected state. **Card display data comes from the catalog, not from the model** | `blocks.js:104-133` |
| **Render** | Brand header + status pill + 3-step wizard + action zone + broker footer | `app.js:760-817` |
| **Act — api_key** | Inline password field *in the card* → `POST /api/nyxid/keys` → NyxID `POST /api/v1/keys`. Secret goes browser→NyxID only, never into chat | `app.js:645-676`, `server.mjs:541-568` |
| **Act — oauth / device_code** | Popup deep-link to NyxID `/keys?slug=…`, card → `waiting_for_user` | `app.js:619-643` |
| **Observe** | Explicit "I've connected, refresh status" → refetch connectors → if connected, flip card | `app.js:678-695` |
| **Resume** | On connect, re-send the original prompt after 900 ms | `app.js:710-727` |
| **Reload** | Stored assistant text re-parsed through the same splitter; cards return | `app.js:1514-1526` |
| **Reactive fallback** | On `AUTHORIZATION_REQUIRED`, if the slug resolves in catalog, render the same rich card | `app.js:2490-2509` |

Two things in his prototype we should **not** copy:

- **Auto-retry** (`app.js:710-727`) contradicts his own stated rule — *"只有用户明确点击「重试请求」才会
  再次提交，避免重复已经部分执行的生产操作"* (`README.md:46-49`). The run may have partially
  executed. Keep retry explicit. (Same conclusion as `ASSISTANT_STREAM_ALIGNMENT.md` G1.)
- **The fence splitter has no code-fence nesting state** (`blocks.js:41-73`) — a
  ` ```nyxid:connect ` inside a quoted block or tool output renders a live card. Must be fixed in
  our port; see W2.

---

## 3. What NyxID already has

More than expected. This is mostly a wiring job, not a build.

| Piece | Status | Where |
|---|---|---|
| `ConnectCardContentBlock` type — full v8 field set, all six states | ✅ exists | `types/assistant.ts:33-56` |
| `ConnectCard` component — icon, status badge, guidance, action button | ✅ exists | `blocks/connect-card.tsx` |
| In-place connect via `AddKeyDialog` with `prefillSlug` | ✅ **already wired** | `connect-card.tsx:104-125` |
| Live connected-state detection (scans `useKeys()` for the slug) | ✅ exists | `connect-card.tsx:60-66` |
| OAuth reconnect path for expired credentials | ✅ exists | `connect-card.tsx:47-58` |
| Keys + catalog derivation | ✅ exists | `lib/assistant/plugins.ts:132-159` |
| `useKeys` / `useCatalog` / `useCatalogEntry` / `useCreateKey` | ✅ exists | `hooks/use-keys.ts` |
| Card emitted from `nyxid.authorization.required` | ✅ exists (reactive only) | `aevatar-transport.ts:1593`, `:2002-2037` |
| Per-slug dedupe so replayed frames update one card | ✅ exists | `aevatar-transport.ts:1979-2000` |
| Unknown block types → neutral shell | ✅ exists | `chat-thread.tsx` |

**We are ahead of Ean on the connect modalities themselves.** His card only deep-links for OAuth
and device code — it never displays a device code or polls (`app.js:619-643,885-897`, and
`blocks.js:122-123` hardcodes the device fields `null`). Our `AddKeyDialog` already implements
device code properly — placeholder key → NyxID initiate → shows `user_code` + verification URL →
polls to completion (`add-key-dialog.tsx:1694-1819,1936-2059`) — and already does the OAuth
placeholder + scoped initiate (`add-key-dialog.tsx:1441-1468`, `use-providers.ts:105-175`). So:
**take Ean's message shape, not his connect actions.** Ours are better.

**The four real gaps:**

- **G-a — No proactive signal.** A card appears only *after* a call fails
  (`aevatar-transport.ts:1593`). The assistant cannot say "these two need connecting" up front.
- **G-b — Cards die on reload.** `historyEntryToMessage` coerces everything to one text block
  (`aevatar-transport.ts:382-402`).
- **G-c — Card display data is hardcoded, not catalog-resolved.** `auth_kind: "api_key"` is a
  literal default and `icon_url` is `""` (`aevatar-transport.ts:2010-2013`), with a comment
  conceding the live frame doesn't carry it. So the card can offer the wrong affordance for an
  OAuth or device-code service.
- **G-d — No explicit resume.** Copy says "send your request again"
  (`connect-card.tsx:68`); the user retypes.
- **G-e — OAuth navigates the user out of the chat.** ⚠️ **The blocker.** `handleConnect` ends in
  `hardRedirect(response.authorization_url)` (`add-key-dialog.tsx:1468`), and `hardRedirect` is
  `window.location.href = url` (`lib/navigation.ts:12-14`). The whole tab leaves the transcript, and
  `redirectPath` sends the user to `/keys/{id}` afterwards (`add-key-dialog.tsx:1456`) — not back to
  the conversation. **In-chat OAuth connect cannot work until this becomes a popup.** Found by the
  Codex pass; I had assumed the dialog was drop-in.
- **G-f — Device-code completion doesn't repaint the card.** `usePollDeviceCode` invalidates
  `provider-tokens`, `providers`, `llm-status` — but **not `["keys"]`**
  (`use-providers.ts:233-237`), which is exactly the query `ConnectCard` reads its connected state
  from (`connect-card.tsx:59-65`). One-line fix.

---

## 4. The contract

### 4.1 Aevatar → FE (inside the message text)

The assistant embeds, in its normal message content, one fenced block per service it needs:

````
```nyxid:connect
{"catalog_slug":"api-github","reason":"read your merged PRs"}
```
````

- `catalog_slug` **required** — must be a slug Aevatar saw on `GET /api/v1/keys` / the catalog.
- `reason` **required** — one short human sentence. Display only.
- `requested_scopes` optional — display only in v1; not an action input.
- All services needed for the task should be emitted **in the same message**, before the assistant
  waits. Serial "oh, and also…" discovery is the thing users hate.
- Everything outside the fences stays ordinary markdown.

**Binding rules for the FE side:** the fence is parsed **once, in the transport**, into a
`ConnectCardContentBlock`. `catalog_slug` is treated as an untrusted string — shape-validated, then
resolved against NyxID's catalog. **No other field from the model is ever an action input.** If the
slug does not resolve, the card renders in a degraded "not in catalog" state; it never invents a
connect target.

### 4.2 FE → Aevatar

Unchanged and closed: **send-message (+stream)** and **get-history**. The connect flow adds
**nothing**. Aevatar observes completion by polling `GET /api/v1/keys` itself (constraint 2).

### 4.3 FE → NyxID (all connect actions)

| Action | Endpoint | Note |
|---|---|---|
| Resolve card display data | `GET /api/v1/catalog` + `GET /api/v1/keys` | already via `useCatalog` / `useKeys` |
| Connect (api_key) | `POST /api/v1/keys` | via existing `AddKeyDialog`; secret never enters chat |
| Connect (oauth / device_code) | existing `AddKeyDialog` / `/keys?slug=` deep link | already wired |
| Observe completion | `useKeys()` refetch | card flips when the slug appears active |

---

## 5. Work plan

Ordered. Everything is **FE-only** unless marked. ✅ = landed 2026-07-23.

### Landed

| # | What shipped | Where |
|---|---|---|
| ✅ **W4b** | OAuth no longer navigates the tab away. `OAuthStep` keeps the dialog mounted, renders the authorization URL as an explicit user-clicked link (real user gesture — no popup, nothing to block), and polls the placeholder key in place via the new `useKeyAuthorizationStatus`. Success / denied / retry all render in the modal. `hardRedirect` import dropped (still used by `provider-grid`/`sa-connected-providers`, so the util stays). | `add-key-dialog.tsx:1441-1560`, `hooks/use-keys.ts` |
| ✅ **W4c** | `usePollDeviceCode` now invalidates `["keys"]` on completion — the query the card actually reads. | `use-providers.ts:233-241` |
| ✅ **W5** | Card resolves the real modality + name from the catalog via `useCatalogEntry` + new exported `catalogAuthKind`, instead of trusting `block.auth_kind` (which the transport hardcodes to `api_key`). An OAuth service no longer gets offered a paste-your-key button. | `connect-card.tsx`, `lib/assistant/plugins.ts` |
| ✅ **W6** | Slug missing from the catalog → card renders with an explanation and **no connect affordance**. Never invents a connect target. | `connect-card.tsx` |
| ✅ **W8** | Card streams live authorization status: placeholder key in `pending_auth` → spinner + "Waiting for {service}…" + `waiting_for_provider` badge, in an `aria-live` region. | `connect-card.tsx` |

| ✅ **W2** | `lib/assistant/connect-fence.ts` — the marker parser, **with** the ordinary-code-fence nesting state Ean's lacks, so a `nyxid:connect` inside a quoted block or tool output stays literal text. Slug shape-validated; malformed markers degrade to literal text. Plus `renderableText` for the streaming path. | new + tests |
| ✅ **W3** | Transport splits assistant text into `text` + `connect_card` blocks on **both** paths — `historyEntryToMessage` (reload) and `closeOpenMessage` (live). Same splitter, so reload converges on the live shape. Marker-free messages keep byte-identical single-text-block output. User messages are never split (markers are Aevatar-authored; a user pasting the encoding can't mint a card). | `aevatar-transport.ts` |
| ✅ **W4** | `TEXT_MESSAGE_CONTENT` streams only `renderableText(accumulated)`, forwarding the newly-safe suffix — a half-written marker never reaches the transcript. | `aevatar-transport.ts` |

Gate: `tsc -b` clean, **1950 tests / 173 files pass**, lint 0 errors on changed files. 30 new tests.

**A property test earned its keep.** `renderableText` feeds a suffix diff
(`safeText.slice(emittedText.length)`), so any shrink between two prefixes corrupts every later
delta. Exhaustively checking every chunk boundary over 9 samples found a real one: the newline
before a marker belongs to the text run until the opening fence is recognised, then gets absorbed —
a one-character shrink. First fix (blanket `trimEnd`) then broke a cancel-flow test that asserts a
partial `"Hello, "` keeps its trailing space. Final fix trims **trailing newlines only**, which is
the exact class of character fence recognition can absorb. Both tests are in the suite.

### Remaining

| # | Work | File | Size |
|---|---|---|---|
| **W0** | **Ask Aevatar: can message history return a typed block array this cycle?** No longer blocking — Option B shipped. Yes → `textToBlocks` gains a typed-block branch and `connect-fence.ts` is deleted; the renderer is untouched either way. | — | — |
| **W1** | **[Aevatar]** Emit a connect card for every service the task needs, in one message, before waiting — **A:** as a typed `connect_card` block on the SSE *and* in history; **B:** as a programmatically-written `nyxid:connect` fence in the message text. **Never LLM-authored.** The only upstream ask; unblocks everything below. | Aevatar | — |
| **W4d** | Add an assistant mode to `AddKeyDialog`: optional `onConnected({blockId, catalogSlug, keyId})`, and skip the post-connect Agent Key onboarding step (`connect-verify-step.tsx:65-88`) — that step mints a separate key for external AI tools and is not needed when Aevatar already has identity. Preserve existing non-assistant callers (`pages/keys.tsx:979`, `pages/key-detail.tsx:2441`, `plugins-view.tsx:415`). | `add-key-dialog.tsx` | M |
| **W7** | Explicit resume: once every card in the gate is satisfied, show a **"Retry that request"** button that re-sends the originating prompt. Explicit, never automatic (§2). Fixes **G-d**. **Two constraints from §8.3:** (a) the completion predicate is reason-specific — `NYXID_SERVICE_NOT_CONNECTED` accepts any active same-slug key, but `NYXID_UNAUTHORIZED` requires **the exact `key_id`**, or an unrelated same-slug key silently clears a reauthorization gate; (b) exactly-once *across a reload* needs **Aevatar** to accept a caller-supplied stable idempotency id and dedupe it durably — today `clientRequestId` is a fresh `crypto.randomUUID()` per run (`aevatar-transport.ts:1046`) and history carries no client id. Without that, scope resume to best-effort live behavior and keep the manual button. | `connect-card.tsx`, `use-assistant.ts`, `pages/assistant.tsx` (+ **Aevatar** for durable dedupe) | L |
| **W9** | Remaining tests: connect → flip → retry (needs W7). Fence parsing, history round-trip, unresolvable slug, and streaming hold-back all landed with W2/W3. | `*.test.ts(x)` | S |

**Sequencing (updated after the first tranche landed):** the FE side is now ahead of Aevatar. **W1
is the only thing standing between this and the full flow** — until Aevatar emits connect markers,
cards appear only *reactively*, after a call has already failed on
`CUSTOM nyxid.authorization.required`. Everything downstream of the marker is built, tested, and
exercised by that reactive path today.

W7 next on the FE side (it needs no Aevatar work in its explicit form — the user clicks, so no
durable idempotency is required; only *automatic* resume needs that). W4d after. W0 whenever
Aevatar can answer it.

**Rollback order:** disable resume (W7) first, then assistant dialog mode (W4d). Keep W3's hydration
— it is backward-compatible with plain text messages.

---

## 6. Defects found

**D1 — Constraint-1 violation: the FE calls Aevatar's approve endpoint.**
`aevatar-transport.ts:840` posts to `/api/v1/assistant/conversations/{id}/approve`, reached from the
in-chat approval card via `useDecideApproval` (`hooks/use-assistant.ts:400-414`). That is a third
Aevatar endpoint, and it is the wrong decision plane — the same finding as
`NYX_CHAT_REFERENCE_CONFORMANCE.md` F2/G3. In-chat Approve/Deny should call NyxID
`POST /api/v1/approvals/requests/{id}/decide`. Out of scope for this plan, but it is the other
thing standing between us and "chat + history only".

**D2 — Also present: create and delete conversation.** `aevatar-transport.ts:633-637` (create,
called before every first message via `use-assistant.ts:311-347`) and `:656-670` (delete, exposed
through the sidebar at `pages/assistant.tsx:66-83`). Both are further Aevatar endpoints. The Codex
pass reads constraint 1 strictly and says remove both: mint the conversation id client-side and
create lazily on first send; drop delete, since deleting locally would be misleading when history
brings the conversation back. I had left this as an open question; the strict reading is
defensible and is now §7 Q1.

**D4 — The transport interface codifies the violations.** `AssistantTransport`
(`types/assistant.ts:222-250`) *requires* remote create/delete/decide. Narrowing the production
transport so only history reads and message-send+stream can reach Aevatar is the structural fix, not
just deleting call sites.

**D6 — The existing adapter's card identity is FE-random and per-turn.** `block_id` comes from
`newId()` = `crypto.randomUUID()` (`aevatar-transport.ts:313-315`), and the slug dedupe map
`promptedConnectSlugs` is allocated per run (`:1063`, used `:1979,2036`). So the current
`nyxid.authorization.required` path cannot produce a stable card identity across reload, and cannot
dedupe across turns. It is a live-only compatibility adapter — fine as the fallback, never the
identity source. Under Option A, `block_id` becomes Aevatar-owned and opaque.

**D5 — Connect cards are marked completed while still unconnected.** On a `blocked` outcome,
`finalizeActivity` emits `block.completed` for every open card, but `toTerminalBlock` is applied
only for `approval` kind (`aevatar-transport.ts:2274-2287`). So a connect card is announced complete
while its state is still `needs_connection`. Since this plan makes that card an actionable, persisted
gate, the lifecycle needs an explicit "turn blocked, card open" state rather than a false terminal.

**D3 — History coercion loses everything typed.** `historyEntryToMessage:382-402` → one text block.
Today that silently discards run cards and approval cards on reload, not just connect cards. W3
fixes it for connect cards; the rest stay lost until their state is likewise text-derivable or
history gains structure.

---

## 7. Open questions

1. **Do create/delete conversation count against "chat + history only"?** (D2)
   *Recommended default (revised after §8):* **yes, remove both.** Mint the conversation id
   client-side and create lazily on first send; drop delete rather than fake it locally. Confirm,
   because this removes a visible button.
2. **Does Aevatar's history return assistant text verbatim, including fences?** The whole plan rests
   on it. Ean's BFF strips only its own injected `[[NYXID_CONTEXT]]` block from *user* messages
   (`server.mjs:611-613`) and leaves assistant content untouched, which suggests yes — but it should
   be confirmed against our history endpoint before W3 is built.
   *If no:* cards cannot survive reload under the two-endpoint constraint, and that becomes an
   Aevatar ask.
3. **Fence keyword and payload — ours or Ean's verbatim?** Recommend adopting `nyxid:connect` and
   his field names exactly, so his prototype stays a valid reference and Aevatar implements one
   thing for both clients.
4. **Should the reactive `AUTHORIZATION_REQUIRED` path stay** once W1 lands? *Recommended:* yes,
   as the fallback for gaps the assistant did not predict.

---

## 8. Convergence — independent Codex pass

A second pass ran the same brief over the same two codebases (Codex CLI, no access to this doc).
Raw output: `assistant-in-chat-connect-codex-plan.md`.

### 8.1 Agreed

Both passes independently reached the same conclusions on everything except the wire encoding:

- The FE must consume the **existing typed `ConnectCardContentBlock`** (`types/assistant.ts:34-57`).
  Do not invent a second card model.
- **Drop Ean's prompt injection** (`server.mjs:580-609`) — Aevatar already reads `/api/v1/keys`.
- **`catalog_slug` is the only trusted field.** Everything else the model or Aevatar sends is
  display data; auth kind, provider config and scopes are re-resolved from NyxID's catalog.
- **Reuse, don't rebuild:** `ConnectCard`, `AddKeyDialog`, `useKeys`/`useCatalog`/`useCreateKey`,
  and `plugins.ts`'s `catalog_service_slug` join.
- **History coercion to one text block is the reload defect** (`aevatar-transport.ts:382-401`).
- **Resume must be an ordinary chat send of the original text, once, explicitly** — no new Aevatar
  call, and no auto-retry copied from Ean.
- **Constraint-1 violations:** approve (`:840`), create (`:635`), delete (`:665`).
- Keep `nyxid.authorization.required` as the compatibility/fallback path.

### 8.2 The one real disagreement — wire encoding

| | This pass | Codex |
|---|---|---|
| Recommends | Ean's in-text marker, parsed at the transport seam | first-class typed `connect_card` block in stream + history |
| Because | history is text-only, so nothing else survives reload without an Aevatar storage change | model-authored JSON, ordinal ids and prose parsing are brittle for production |

**Resolution (§1):** both are right about different things, and the disagreement is smaller than it
looks — *both* require an Aevatar change, and *both* land the same typed block on the FE side. The
split is which Aevatar change: text-only (mine, smaller) versus block-array storage (Codex's,
cleaner). So it becomes **W0, a single question to Aevatar**, with A preferred and B as the
forward-compatible fallback.

Codex's brittleness objection is aimed at Ean's version, where the **LLM** authors the fence. Under
constraint 2 Aevatar authors it programmatically from its own `/keys` read — which removes the
hallucination surface and reduces the fence to a transport encoding. That amendment is the actual
reconciliation and is now folded into §1.

### 8.3 Codex-only findings — verified and adopted

All re-checked against source before inclusion.

| Finding | Verified at | Folded in as |
|---|---|---|
| ⚠️ **OAuth hard-redirects the whole tab**, leaving the transcript and returning to `/keys/{id}` | `add-key-dialog.tsx:1456,1468`; `lib/navigation.ts:12-14` | **G-e** + **W4b** — the true critical path; I had assumed the dialog was drop-in |
| Device-code poll never invalidates `["keys"]`, so the card can't repaint | `use-providers.ts:233-237` vs `connect-card.tsx:59-65` | **G-f** + **W4c** |
| `AddKeyDialog` success continues into Agent Key onboarding — unwanted in chat | `add-key-dialog.tsx:2899-2906`; `connect-verify-step.tsx:65-88` | **W4d** |
| Connect cards get `block.completed` while still `needs_connection` | `aevatar-transport.ts:2274-2287` | **D5** |
| The transport *interface* mandates create/delete/decide | `types/assistant.ts:222-250` | **D4** |
| Create/delete should be removed, not just questioned | `:633-637`, `:656-670` | **D2** sharpened |
| Ean's device_code doesn't actually implement device code — same deep link as OAuth | `app.js:825-826,885-897` | §3 "ahead of Ean on modalities" |
| Reloaded cards in Ean's prototype lose retry context, so they can't auto-resume | `app.js:1518-1521,710-713` | supports explicit-resume (W7) |

### 8.4 This pass only

- **The code-fence nesting hole** in Ean's splitter (`blocks.js:41-73`) — a `nyxid:connect` inside a
  quoted block or tool output renders a live card. Carried into W2 as a required fix.
- **Ean's auto-retry contradicts his own README rule** (`app.js:710-727` vs `README.md:46-49`).
  Codex independently recommended explicit resume, on different grounds.
- **The Aevatar-authored (not LLM-authored) amendment** to the marker, which is what makes Option B
  defensible at all.

### 8.5 Codex revised its own draft — second round folded in

Codex ran an adversarial self-review after its first draft and materially revised it. Re-verified
and adopted:

| Revision | Verified at | Effect |
|---|---|---|
| **Reason-specific completion predicate.** `NYXID_UNAUTHORIZED` must match the **exact `key_id`**; any-active-same-slug would let an unrelated key clear a reauthorization gate | `connect-card.tsx:35-46` (today's matcher is exact-key-first, and `connectedNow` already excludes reauth at `:59-61`) | constraint (a) on **W7** — it is the *new* resume predicate that must not regress this |
| **Resume needs Aevatar for durable exactly-once.** `clientRequestId` is a fresh UUID per run; history carries no client id | `aevatar-transport.ts:1046`, `:233-241` | **W7 re-sized FE-only/M → L, needs-Aevatar**, with best-effort live fallback |
| **`block_id` is FE-random, dedupe is per-turn** | `:313-315`, `:1063,1979,2036` | new **D6** |
| **OAuth popup detail:** retain the popup handle, poll that exact placeholder key, keep `/keys/{id}` as the landing page — no new router/callback file | `add-key-dialog.tsx:1441-1469` | **W4b** refined |
| **Softened its fence verdict** — now states the trade-off explicitly ("works without a backend message-schema change and reloads from flat history… the typed choice requires an Aevatar producer/storage change") rather than a flat "do not adopt" | — | **converges with §1's A/B gate.** My earlier claim that Codex had missed the storage trade-off applied to its first draft only, and is withdrawn |

The net effect is that the two passes ended closer than they started: same A/B trade-off, same
preference for A, same fallback logic for B.

### 8.6 Shared limits

Neither pass ran against live Aevatar. §7 Q2 — *does Aevatar history return assistant text
verbatim?* — is unverified by both and is a precondition for Option B. Codex additionally notes the
Aevatar producer repo was not in the workspace, so all upstream file paths in W1 are unfilled.
