# Travel Booking Through NyxID and the AI Layer

**What this is:** the design for ChronoAI's travel assistant — the AI plans and books real flights in conversation, and the traveller pays the airline directly with their own card, inside the chat. NyxID brokers the travel API; the AI layer (Aevatar) owns the booking process; nobody in between touches the money.

**Fact bases:** `DUFFEL_API_VERIFICATION.md` (Duffel API semantics, verified against live docs) and `DUFFEL_PAYMENT_MODEL.md` (payment rails and custody, verified). Aevatar workflow primitives were verified against the live deployment (`GET /api/capabilities`, `capabilities.v1`, 2026-08-13). Everything below marked **ASSUMPTION** is not yet verified; everything else is either verified there or cited against this repository with `file:line`.

Part I is the narrative, readable without knowing the codebase. Appendix A is the implementation contract.

---

# Part I — The design

## 1. What we are building

You tell the chat where you want to go. The AI searches real flights through Duffel (a flights API and accredited travel network), holds the one you pick, and a payment card appears in the conversation: itinerary, price, deadline. You enter your card into Duffel's own secure form — rendered by Duffel inside the chat, including your bank's 3-D Secure challenge — and the money goes card → airline. ChronoAI never holds the fare; the travel supplier is merchant of record; our PCI exposure is the lightest tier (SAQ-A) because card data never touches our servers.

Three owner decisions anchor everything: **the customer bears the transaction** (no ChronoAI float, no fare in NyxID's credit system); **NyxID stores no booking data** (no order rows, no passenger profiles, no state machine — Duffel is the system of record); and **the AI layer owns orchestration and durability** (booking is a long-running task, and the workflow engine, not the chat transcript, is what makes it survive hours and restarts). A fourth principle governs the build itself: **reuse existing machinery wherever it exists** — §12 lists, item by item, what is reused versus what is genuinely new, and the new list is short.

## 2. Where the Duffel endpoint lives

Concretely, because this is the first question anyone asks:

- **The catalog row.** Duffel is one `DownstreamService` document — slug `duffel`, `base_url: https://api.duffel.com`, holding the encrypted platform API token. No new collection, no new service; it sits in the same class as the other admin-managed, platform-credentialed catalog rows (the Aevatar row among them).
- **What the agent calls.** `POST /api/v1/proxy/s/duffel/air/offer_requests` (slug proxy route, `routes.rs:76-77`; UUID form at `:94-95`). NyxID resolves the slug, injects the decrypted credential and the mandatory `Duffel-Version: v2` header, and forwards to `https://api.duffel.com/air/offer_requests`. The AI never sees the token.
- **The spec.** `backend/specs/catalog/duffel.openapi.json`, served publicly at `/api/v1/catalog-specs/duffel/openapi.json` (`routes.rs:1261`), synced at startup into `ServiceEndpoint` rows and published as typed MCP operations — the existing catalog machinery (CLAUDE.md Critical Rule 10), nothing bespoke.
- **The skill.** `nyxid/duffel-travel` in the Ornn registry, referenced from the catalog row's `recommended_skills` field (`downstream_service.rs:286-288`): it teaches the flow, the paid-proof rule, hold eligibility, and the resource-token lifecycle (§7).
- **The payment action** is declared in the existing **assistant-actions manifest** (§3), not on a bespoke surface.
- **The only genuinely new NyxID routes** are two small endpoints under `/api/v1/resource-tokens` — a generic namespace, because the signed-token primitive they serve (§7) is a general answer to "authorize access to an external resource without storing attribution," and Duffel is merely its first user. `POST /api/v1/resource-tokens/exchange` trades a token for a provider-side artifact (v1: the Duffel card-form client key); `POST /api/v1/resource-tokens/reissue` recovers a lost token. Both are human-session-only; both dispatch on the resource type named inside the token (v1 supports `duffel:order` only). The namespace is deliberately **not** called "capabilities" — that word means platform abilities (what NyxID knows how to do, as in the assistant-actions manifest and Aevatar's capability listing), and the token primitive must not squat it.

Everything else NyxID contributes is enforcement on the path that already exists.

## 3. How the AI learns what it can do: two declaration mechanisms, deliberately

NyxID already has two ways of telling the AI layer about itself, and travel uses both — each for the shape it was built for:

- **The assistant-actions manifest declares human-in-the-loop actions.** `GET /api/v1/assistant/actions` (`handlers/assistant_actions.rs`, route `routes.rs:1253-1254`) is a public, versioned, static manifest (`schema_version: 4`, revision `nyxid-assistant-actions.v4`, `assistant_actions.rs:8-9`) consumed by Aevatar at startup. Its existing entry, `service.connect`, is exactly the shape of the payment step: *the AI asks the user's browser to complete something NyxID owns end to end, and gets back only completion or decline.* Payment becomes the **second entry in that manifest** — not a new surface.
- **MCP declares API operations.** The Duffel endpoints are proxy calls, not browser actions; they are published as typed MCP operations from the catalog overlay, like every other catalog service.

The split in one sentence: **the manifest is for things a human must finish in a browser; MCP is for things an agent calls over HTTP.** That distinction is why travel reuses two existing mechanisms instead of inventing a third.

**The `payment.complete` manifest entry**, with each field chosen against the manifest's actual validation contract (`assistant_actions.rs:276-344`):

- `action: "payment.complete"` — must also be added to the supported-actions contract list mirrored in the manifest's own tests (`assistant_actions.rs:119-134`) and to Aevatar's parser; until Aevatar's side lands, clients render the deliberately decline-able unsupported-action fallback (`frontend/src/schemas/assistant-actions.ts:74-80`), which is safe by design.
- `description`: model-facing, in the style of `service.connect`'s (`assistant_actions.rs:11`): *"Ask the user's browser to complete a card payment for a held external booking. NyxID owns the entire journey — the provider's PCI-compliant card form, the bank's 3-D Secure challenge, and verification — and reports back only completion or decline. Never ask the user for card details in chat."*
- `params_schema`: all-string properties (`serviceSlug`, `resourceRef` — the `ord_…` id, `resourceToken`, `amount`, `currency`, `payBy`) — strings because the schema validator admits only object/array/string types (`assistant_actions.rs:246-272`), which suits Duffel's decimal-string amounts anyway; `resourceToken` passes the manifest's forbidden-secret-name normalizer (`token` alone is forbidden, `assistant_actions.rs:135-154`) and is acceptable to carry here because the action envelope already transits the workflow → chat trust path that holds the token in run state.
- `risk: "destructive"` — the closed set is `low | grant | destructive` (`assistant_actions.rs:328`), and an irreversible money movement is destructive in exactly this taxonomy's sense: `grant` marks durable access grants, `destructive` marks one-shot irreversible effects. Choosing it also makes the next property mechanical rather than conventional:
- `remember_eligible: false` — a payment consent is never remembered, and the manifest contract *enforces* `risk == "destructive" → remember_eligible == false` (`assistant_actions.rs:338-340`). The right value falls out of the right risk class.
- `tier: "v1"` — not actually a free choice today: the parser contract requires exactly `"v1"` (`assistant_actions.rs:329-333`). Noted for later: `risk` and `tier` are plausible hooks for entitlement gating (§13, open decision 5) — that is an observation, not a design.

**Re-examination of the standalone token routes, since the manifest now declares the action:** the manifest is declaration; execution still needs a server. The browser card must exchange the token for a Duffel component key server-side (the platform credential mints it), and lost tokens need reissue — so both routes survive. What the manifest made redundant is any bespoke *discovery* surface for payment, and none is built; what died is the "capabilities" name.

## 4. What users see — and what they don't

The placement question at the product layer: is travel part of AI Services, or something else entirely? **Something else — and deliberately almost nothing.**

- **Not AI Services.** The `/keys` page exists for services a user connects with *their own* credentials — paste a key, manage a connection. Duffel runs on a platform credential: there is nothing to connect, no key to paste, no connection to manage. Listing it there would offer users a button that does nothing.
- **Admins** manage the Duffel row in the existing admin catalog UI, like any platform-credentialed service — that is where the token is set and rotated (service create/update is admin-gated, `handlers/services.rs:32`, and the admin `/services` UI renders catalog rows, `handlers/services.rs:208`; the Services page is admin-only in the sidebar per the streamlined-services design).
- **Users** get no catalog entry, no settings page, no toggle. Travel appears where it happens: in conversation, plus the payment card when it is time to pay. That is the entire user-visible surface.
- **Agents** discover travel where discovery actually matters: the MCP operations and the assistant-actions manifest (§3), guided by the skill.

Two consequences follow from the no-booking-data rule — consequences, not defects:

- **There is no "my trips" page, and there cannot be one.** NyxID stores no bookings, so there is nothing to list. A trip lives in three places: the conversation that booked it, Duffel's records, and the airline's confirmation email. Anyone tempted to design a trips surface later should read this paragraph first — the data does not exist, by decision. The closest thing to recovery is the token-reissue path (§7), which re-establishes access to a specific order from Duffel's own data plus the logged-in session.
- **Entitlement is currently ungated.** A public platform-credentialed row means *every* NyxID account can book flights through the assistant — no opt-in, no beta flag, no plan tier, no per-org enablement. That may be exactly the intended launch posture, but no gating mechanism exists today, and the choice has not been made. It is listed as an open decision in §13 with the build cost of each option.

## 5. The division of responsibility

**The AI layer owns the process.** Orchestration, durability, retries, deadlines, timeout branches, user prompting, and the single human gate before inventory is touched. A booking is a durable workflow run in Aevatar's engine — it suspends for hours, wakes on signals or deadlines, checkpoints its state, and decides what to do when a hold lapses or a user disappears for a day.

**NyxID owns the door.** The credential, the operation allowlist enforced at execution time, the signed resource token that proves whose order is whose, the recovery path when a token is lost, and the guarantees that nothing booking-shaped is stored, logged, or routed outside the NyxID process. NyxID's contract is identical whether the caller is a durable workflow, a plain chat turn, or the CLI — it neither knows nor cares that the caller can suspend.

Neither layer reaches into the other. Aevatar never sees a Duffel token; NyxID never holds workflow state. Exactly two artifacts cross the boundary: the signed resource token (NyxID → workflow state) and the payment-completed signal (chat card → workflow).

## 6. How a booking works, end to end

In workflow terms — every primitive name below is verified from the live capability listing:

1. **Search & choose.** Read-only `connector_call`s through the proxy (allowlisted operations; no token, no approval).
2. **Hold.** A `secure_connector_call` creates a *hold order*: seats reserved, **no money moved**, and the hold lapses on its own at `payment_required_by` if never paid. This step carries the one human gate (§8). NyxID stamps the response with the signed resource token; the workflow stores token, order id, and deadline in run state and checkpoints. Fares that cannot be held (`requires_instant_payment`) are declined conversationally — v1 books holdable fares only. (**Measured, 2026-08-14:** 78% of offers were holdable — 495 of 635 on a real SIN→NRT return; `fixtures/duffel-offer-request-sample.json`. The restriction is mild.)
3. **Await payment.** The workflow emits the `payment.complete` action (§3) into the chat and suspends on `wait_signal("payment_completed:<order_id>")`. The card renders Duffel's own card-input iframe and 3DS challenge (both are Duffel's; the host page supplies a container and callbacks), and on success fires the signal. **The signal is the fast path, never the authority:** on every wake the workflow does a token-bearing read-back and treats the order as paid only when `awaiting_payment == false` **and** ticket documents are issued — Duffel's own documented proof; either condition alone is insufficient (an expired unpaid hold also shows `awaiting_payment: false`).
4. **Deadlines.** `wait_signal` times out or the deadline nears: read back the order. Paid but the signal was lost → proceed. Unpaid, deadline close → `notify` the user once. Lapsed → say so plainly and offer to rebook. Nothing is orphaned; Duffel released the seats itself.
5. **Long holds — the normal case, not the exception.** Real hold windows run **1–3 days** (measured 2026-08-14: `payment_required_by` of Aug 15 and Aug 17 from an Aug 14 search; `fixtures/duffel-offer-request-sample.json`), while `wait_signal`'s timeout caps at exactly 24 h (86,400,000 ms, verified) — so a single wait can never span a real hold. The wait is a **deadline-driven re-entrant loop**: each leg waits `min(payment_required_by − now, 24 h)`, wakes on signal or timeout, re-reads truth, checkpoints, re-enters — at most 3–4 legs for measured windows — with `self_reschedule` as the fallback if looped waits prove awkward.
6. **The price can move while the seat is still held.** On the measured holdable offer, `price_guarantee_expires_at` (Aug 16) falls **inside** `payment_required_by` (Aug 17): the seat is held longer than the price is guaranteed, so a re-price inside the hold window is a live case, not a formality. Behavior: the payment card re-reads the order's current totals when opened (the amount carried in the action params is display-only); a `price_changed` rejection on payment submit refreshes the display for re-confirmation (3DS shows the true amount regardless); and the workflow notifies the user once when the guarantee lapses before payment.

If the user walks away mid-payment, the hold quietly lapses at the airline. There is nothing to clean up, because nothing was stored.

## 7. The signed resource token: attribution without a database

NyxID must answer one question safely — *when someone asks to pay for or read order X, is it theirs?* — while being forbidden to keep a table of who booked what. You cannot authorize what you cannot attribute, so the attribution travels with the user instead of sitting in a database: **when an order is created through NyxID, NyxID stamps the response with a signed resource token** — a short-lived, cryptographically signed statement that order `ord_…` belongs to this account, valid roughly as long as the hold. The workflow keeps it in run state. From then on:

- Reading the order's status: token required.
- Paying the order, or cancelling it: token required.
- Exchanging for the card-form client key: token required, **and** the caller must be the logged-in human the token names — agent and automation credentials are refused at that door.

The order-list endpoint is simply not callable through NyxID (§9), so there is nothing to enumerate; per-order access needs the token. Token custody is deliberately **per account, not per credential**: any agent or session belonging to the same effective owner can use it, because the design *is* "the agent books, the human pays." For org-owned services, the token names the same "effective owner" identity at stamping and at checking — one shared function computes it in both places, so the two can never disagree.

The token is now precious, which creates three obligations, each met (§10): key rotation must not kill live tokens, a failed stamping must not orphan a payable hold, and a lost conversation must have a recovery path.

## 8. The gates: one per concern

Aevatar's connector primitives carry an approval seam built for exactly this (`secure_connector_call` with `approval.policy: "required"` — *"suspend before connector execution"* — and NyxID-aware parameters: `approval.service_ref`, `http_verb`, `resource`, `permission_scope`, `expiration_seconds`, `status_check_interval_seconds`). Meanwhile the bank's 3DS challenge is itself a consent mechanism. Two gates configured carelessly would double-prompt the user; the design assigns one gate per concern:

- **Booking consent — the AI layer's gate.** The hold step runs behind the approval seam (or `human_approval`): a human confirms *"reserve these seats at this price"* before Duffel is called. This lives in workflow state, not in any NyxID record, and it is also the throttle on a runaway or prompt-injected agent: no inventory is touched without a human tap.
- **Payment consent — the bank's gate.** 3-D Secure by the actual cardholder, inside Duffel's iframe, showing the true amount. Nothing wraps it; a second prompt on top of Strong Customer Authentication would be pure friction.
- **NyxID adds no third gate.** Its contributions are the allowlist and the resource token; no NyxID approval machinery is configured for Duffel operations.

**One integration wrinkle, flagged rather than hidden:** the capability schema documents `approval.node_id` as *"Required NyxID node routing identity"*, but the Duffel row forbids node routing (§9). If the seam genuinely requires a node identity, the hold gate uses `human_approval` before a plain `secure_connector_call` instead. This is a question for the Aevatar owner, to resolve before the workflow is built.

## 9. What NyxID enforces, and where

Publishing a curated operation list is discovery, not enforcement — the proxy is otherwise a passthrough that forwards whatever path it is given. And NyxID has **two independent request executors** (the REST/WebSocket path and a separate executor inside the MCP tool layer); a check present in only one of them is not a control. The enforcement design follows from those two facts:

- **One shared, side-effect-free authorization check, called by both executors** after the final method, path, and body exist and before anything is approved, billed, node-routed, or forwarded. It answers: is this method+path on the service's allowlist, and where the rule demands a resource token, is a valid one presented for this exact order? For Duffel, the allowlist covers search, offer reads, order create (which mints the token), per-order reads, payment, and cancellation; the order-*list* endpoint and the raw component-key endpoint are absent, hence unreachable.
- **A canonical path form** used for matching *and* forwarding: decoded exactly once, encoded separators, dot-segments, duplicate and trailing slashes rejected outright, query strings excluded from matching — so a path that looks different to NyxID but identical to Duffel cannot slip past.
- **One gate in front of every master-credential decrypt.** The catalog master credential is unreachable outside `proxy_service`: its ciphertext is namable only inside the two authorization functions, and the only decrypt path for it goes through `AuthorizedMasterCredential`, whose constructor is private to those functions — so within the proxy layer, authorization is the only way to produce a decryptable credential. (This is a module-boundary guarantee, not a whole-crate type proof: the general `decrypt` API remains public, and one admin-gated OIDC client-secret read in `handlers/services.rs` sits outside the newtype — it returns a secret to its owning admin and injects nothing into a proxy request.) Authorization is evaluated against the real effective actor, with consent checks for private rows that work even when no per-user service record exists. The one resolution path with no real user behind it — the server-chosen/admin path — is handled by prohibition: it may never serve a private credentialed row, and no synthetic actor can be constructed outside the module to pretend otherwise.
- **No response bodies in logs, for any service.** Proxied error bodies can contain passenger data; the proxy logs status, size, and correlation id only.
- **No-node as a hard invariant.** Passenger-bearing request bodies must never leave the NyxID process for a user's node agent. That is a persisted flag on the catalog row, enforced at key creation, at binding creation, and by a fail-closed guard at the final moment of routing — in both executors.

The first, fourth, and the credential gate are **generic NyxID platform improvements, independent of Duffel** — two of them close exposures that exist today. Only the row, the overlay, the skill, the manifest entry, and the payment card are travel-specific. The PR sequence keeps that split (Appendix A).

## 10. When things go wrong

- **Key rotation.** The resource token is the only proof of ownership, so rotating NyxID's signing key must not strand a live, still-payable hold. Tokens carry a key id and are verified against the current **and** previous public key, with the previous key retired only after an overlap longer than the longest hold window — reusing NyxID's existing JWT/JWKS conventions. **Residual, for Calvin:** an *emergency* rotation (compromised key, no overlap) still strands live tokens; the reissue path below is the mitigation, and accepting that tradeoff is an explicit decision.
- **The stamp fails after the order succeeds.** Duffel may return a response NyxID cannot inspect — oversized, malformed, streamed. The rule is fail *closed and loudly*: the caller receives a controlled error naming the recovery route, never a silent success without a token. Minting happens only on an inspectable 2xx JSON body with a strictly validated `ord_…` id.
- **Recovery without a database.** A logged-in human can ask NyxID to reissue a token: NyxID fetches the order from Duffel server-side and mints a fresh token **only if the order's passenger email matches the requester's verified account email**. Attribution is recovered from Duffel's data plus the session — still zero NyxID storage. The skill therefore instructs the AI to use the account email as the booking contact; when they differ, recovery falls back to the airline's own confirmation email. A stated limitation, not a hidden one.
- **A spoofed "paid" signal.** Harmless by construction: the workflow's read-back is the authority, and the payment card itself renders success only after its own token-bearing read-back satisfies both paid conditions. Browser callbacks are progress indicators, nothing more.

## 11. What we deliberately do not build, and what data still exists

No wallet holds, no settlement, no booking records, no traveler profiles, no NyxID approval machinery for Duffel, no NyxID-hosted payment page (the card lives in chat, on Duffel's iframes), no bespoke action-discovery surface (the manifest already exists). An earlier experimental transaction subsystem in this repository is removed — **code and data**: a migration scrubs the approval documents that carry booking ids, amounts, and itinerary summaries, and drops the transaction collections, before the Duffel row is ever enabled.

Two honest exceptions, both explicit decisions rather than fine print:

- **Audit history.** Historical transaction audit events are metadata-only (ids, amounts, statuses — never passengers), but they live in a tamper-evident hash chain that deletion would break. Recommendation: retain them as a documented exception; the alternative sacrifices the audit chain's integrity guarantees.
- **Backups and replicas.** Dropping a collection does not erase it from point-in-time backups or replicas. The deletion runbook is a release gate with an owner, verified counts, and an explicit statement of how long backup copies persist — a privacy/retention decision, not a code task.

NyxID's ordinary operational records remain: proxy audit and metering rows are metadata (caller, path, status, sizes — `handlers/proxy.rs:272-319`), and request bodies transit in memory only (`handlers/proxy.rs:47`). Nothing booking- or passenger-shaped is persisted by NyxID. Booking references, resource tokens, and passenger details entered in conversation live in **Aevatar workflow run state and chat history** — a real record with a lifecycle, whose retention and access model is an open item below.

## 12. What is reused, and what is genuinely new

Reused — existing machinery, no reinvention:

- The **catalog row + proxy credential injection** (`DownstreamService`, slug proxy routes `routes.rs:76-77, 94-95`).
- The **overlay → `ServiceEndpoint` sync → MCP publication** path (`catalog_spec_registry`/`catalog_spec_sync`, CLAUDE.md Rule 10).
- The **assistant-actions manifest** (`handlers/assistant_actions.rs`, `routes.rs:1253-1254`) — payment is its second entry.
- The **chat action-card blocks and the managed-popup + broadcast completion machinery** shipped in `21297220` (#1349): `frontend/src/components/assistant/blocks/`, `schemas/assistant-actions.ts` with its decline-able unknown-action fallback (`assistant-actions.ts:74-80`), `lib/oauth-popup.ts` and its store/hooks — the payment card is another consumer of the same envelope and completion path.
- The **Ornn skill registry** via `recommended_skills` (`downstream_service.rs:286-288`).
- **Aevatar's durable workflow primitives** (`wait_signal`, `secure_connector_call` + approval seam, `checkpoint`, `lease`, `self_reschedule` — verified `capabilities.v1`).
- The **JWT/JWKS key conventions** (`crypto/jwt.rs:544-560`) for the resource token.
- The **admin catalog UI** for row management (`handlers/services.rs:32, 208`).

Genuinely new — the whole list:

1. The shared **operation-authorization check** called by both executors, with the allowlist policy on the catalog row and the canonical path form (generic).
2. The **resource token**: mint-on-create response stamping, verification, rotation overlap, and the two `/api/v1/resource-tokens/*` routes (generic).
3. The **master-credential authorization gate** and the **no-node invariant flag** (generic; close existing exposures).
4. Proxy **error-body log redaction** (generic; removes an existing leak).
5. The **`payment.complete` manifest entry** and the **payment card block** that renders Duffel's iframes (travel-specific frontend).
6. The **Duffel overlay + row + skill** (travel-specific content, not machinery).

Items 1–4 are platform work any future provider inherits; 5–6 are the only travel-shaped artifacts.

## 13. Decided and open

**Decided (owner):** customer pays via Duffel Cards; card + 3DS in-chat on Duffel's iframes; NyxID stores no booking data; the AI layer owns orchestration and durable task running; NyxID is a credential broker with the catalog as its integration surface; the assistant-actions manifest is the declaration mechanism for human-in-the-loop actions.

**Decided in this design:** one gate per concern (§8); token custody per effective owner (§7); the manifest/MCP declaration split (§3); `risk: "destructive"` + `remember_eligible: false` + `tier: "v1"` for `payment.complete` (§3); the generic `/api/v1/resource-tokens` namespace (§2); holdable-fares-only for v1; fail-closed minting with loud errors (§10).

**Open — each needs a named owner or a Calvin decision:**
1. **Aevatar workflow run state retention and ACLs** — resource tokens, order references, and passenger details entered in conversation now live there; the question is sharp ("what is the retention and access model for run state?") but unanswered.
2. **The `approval.node_id` wrinkle** (§8) — Aevatar owner, before the workflow is built.
3. **Emergency key-rotation residual** (§10) — accept, with reissue as mitigation?
4. **Audit-chain retention exception and backup/replica retention wording** (§11).
5. **Who may book — the entitlement posture** (§4). The row as designed is open to every NyxID account. Options and their build cost: **(a) open to all** — zero build, the current design; **(b) beta cohort** — small: gate the Duffel row behind `developer_app_ids` consent (the mechanism the credential gate already honors for private rows) or an existing feature-flag check in the operation-authorization call, days not weeks; **(c) per-org / plan-tier entitlement** — moderate: the operation-authorization check consults the caller's plan entitlements the way billing's gate already does for metered services, reusing that lookup rather than inventing one — real but bounded work, and the only option that touches billing code. The manifest's `risk`/`tier` fields are a plausible future hook for expressing entitlement to the AI layer (`tier` is currently pinned to `"v1"` by the parser contract, `assistant_actions.rs:329-333`) — noted, not designed. No answer is assumed; (a) is what ships if nothing is chosen, and that should be a choice, not a default.
6. **External work:** Duffel sandbox registration and **Duffel Cards approval** (approval-gated but open to new customers — the one commercial ask; test mode works today); the Aevatar-side booking workflow implementation; Aevatar's parser accepting `payment.complete` (until then, the decline-able fallback renders); Aevatar exposing MCP operation tools in chat; the payment card's signal-delivery surface into the workflow engine.

---

# Appendix A — Implementation contract

Citations verified against this worktree; **ASSUMPTION** marks what is not. PR-A/B are generic platform security, PR-C is deletion, PR-D/E are travel-specific, A.6 is the Aevatar-side workflow.

## A.1 PR-A — master-credential gate + log redaction (generic; closes live exposures)

**The gate.** Today several paths decrypt catalog master credentials directly: strict resolver (`proxy_service.rs:603-620`), lenient (`:745-760`), server-chosen (`:489-498`, resolver `:434-438`, call site `handlers/proxy.rs:1535-1543`), auto-provision (`:1867-1885`), plus catalog reads in `handlers/services.rs` and `catalog_service.rs`. The existing consent helper cannot gate them — it returns `Ok(())` for anything not auto-provisioned (`proxy_service.rs:1689-1710`). Replace with:

```rust
pub async fn authorize_master_credential(db, service: &DownstreamService, actor: &EffectiveActor)
    -> AppResult<AuthorizedMasterCredential>;          // newtype; catalog decrypt APIs accept ONLY this
pub async fn authorize_master_credential_server_chosen(db, service: &DownstreamService)
    -> AppResult<AuthorizedMasterCredential>;          // public rows ONLY; private always denied
```

Predicate: active + http + internal + `!requires_user_credential` + non-empty credential + no `provider_config_id` (the `proxy_service.rs:1654-1664` predicate minus visibility); `public` → allow; `private` → require actor consent for a `developer_app_ids` app, evaluated **without** needing a UserService. `EffectiveActor` has no default/system constructor — a synthetic actor cannot compile. Deny → `NotFound`. Audit: repo-wide `credential_encrypted` read census, every site through the newtype. Tests: private-row denial on UUID/slug/lenient/WS/MCP/server-chosen; allowed with consent; public and auto-provision regressions.

**Redaction.** Remove the 1024-byte error-body preview at `handlers/proxy.rs:2963-2981` for all services; log status, content length, upstream request-id only. Test: a sentinel passenger name in a stubbed 422 body never appears in captured tracing.

## A.2 PR-B — operation authorization + resource tokens (generic primitive)

**One function, two executors.** `services/proxy_authorization.rs`:

```rust
/// Side-effect-free. Called from BOTH executors after the final method/path/body
/// exist and before approval, billing, node transport, or forwarding.
pub fn authorize_proxy_operation(
    policy: Option<&ProxyOperationPolicy>, actor: &EffectiveActor,
    method: &str, canonical_path: &CanonicalPath, token: Option<&VerifiedResourceToken>,
) -> AppResult<OperationDecision>;    // Allowed { mint: Option<ResourceTokenMint> } | denial
```

Call sites: the REST/WS/node executor (`handlers/proxy.rs`, incl. WS `:478-491, 2008-2060` and node forwarding `:2063-2151`) **and** the MCP executor, which builds and forwards requests independently and never enters the REST path (`mcp_transport.rs:1675-1725`; `mcp_service.rs:2829-2861`, node `:3244-3329`, direct `:3333-3368`, generic path `:3371-3410`) — it receives the resolved policy and actor explicitly. Tests: typed-MCP order read/payment without token, generic-MCP `/air/orders`, REST slug/UUID/`_nyxid_via`/WS/node — each asserting **no downstream request is made**.

**Canonical path** (matching *and* forwarding, all entry points): method uppercased; percent-decoded exactly once; encoded separators, dot segments, fragments, duplicate and trailing slashes **rejected** (the existing validator at `proxy_service.rs:250-315` validates but does not canonicalize — extend it); query excluded from matching; case-sensitive. Bypass tests per rejected variant.

**Policy model** on `DownstreamService`, in the idiom of `AnonymousEndpointRule` (`downstream_service.rs:114-123`): `ProxyOperationPolicy { rules: Vec<ProxyOperationRule { method, path_pattern, mints_resource_token: Option<ResourceTokenMint>, requires_resource_token: bool, resource_token_id_path: Option<String> }> }`; `None` = passthrough (today's behavior for every other service).

**Resource-token format** (house JWT conventions — `crypto/jwt.rs:544-560` shows the `kid`/`iat`/`jti` precedent): claims `{iss, aud: "nyxid:resource-token", token_type: "resource_token", sub: <effective_owner_id>, res: "duffel:order:<id>", iat, exp, jti, kid}`. Response header: `X-NyxID-Resource-Token`. `sub` is the proxy-resolution effective owner (org member → org owner, `proxy_service.rs:1331-1361`; subject otherwise, `mw/auth.rs:134-149`) computed by one shared function at mint and verify. **Rotation:** verify against current + previous public keys (new `JWT_PUBLIC_KEY_PREVIOUS_PATH`, JWKS conventions); previous-key retirement ≥ the maximum token TTL (Duffel: 86 400 s → retire no sooner than 7 days); documented rotation runbook; rotation-during-unpaid-hold test.

**Minting contract, fail-closed at runtime:** mint only on 2xx + `application/json` + identity encoding + body ≤ 1 MiB + parsed JSON whose `id_path` yields a string matching `^ord_[A-Za-z0-9]+$`. Anything else — including runtime streaming decisions (`handlers/proxy.rs:2622-2634`), chunked/compressed/oversized/malformed bodies — is **not forwarded as success**: controlled 502-class error naming the reissue route (the provider write may exist). Admin write additionally rejects minting rules on streaming-marked operations. Fixtures: chunked, compressed, oversized, malformed, id-mismatch.

**The two new routes**, both on the human-session-only router, both under the generic namespace:
- `POST /api/v1/resource-tokens/exchange` `{resource_token}` — verify signature/expiry/`sub == session user`, dispatch on the token's `res` type (v1: `duffel:order` → mint the Duffel component client key server-side via the credential gate), return `{component_client_key, expires_at}`; never logged. (**ASSUMPTION:** exact Duffel component-key request/response and scope semantics — fixture-verified in PR-D.)
- `POST /api/v1/resource-tokens/reissue` `{service: "duffel", resource_id}` — fetch the order server-side; mint iff the order's passenger email equals the session user's verified email (**ASSUMPTION:** email field path). Tests: match/mismatch/expired/nonexistent; agent, delegated, and service-account callers rejected on both routes.

**No-node invariant:** `disallow_node_routing` on the row; enforced at `POST /keys` create/update (the existing rejection is conditional on `provider_config_id`, `unified_key_service.rs:801-809` — insufficient), at `NodeServiceBinding` creation, and by fail-closed final guards before REST (`handlers/proxy.rs:2063-2151`) and MCP (`mcp_service.rs:2973-3037, 3244-3329`) node forwarding. Tests include a stale pre-existing binding.

**Credential-write centralization:** every catalog credential write (create `handlers/services.rs:1012-1033`, update `:2236-2246`, rotation, seed/backfill `catalog_service.rs:228-252`) goes through one `catalog_credential_service::write_credential` validating pre-encryption — for the `duffel` slug: reject non-`duffel_test_` prefixes until an explicit audited live-enable flag (live enablement additionally gates on Duffel Cards approval and Calvin's go). Startup audit deactivates violating rows (covers Mongo-smuggled writes). Tests per write path + smuggled-row resolution refusal.

## A.3 PR-C — remove the experimental transaction subsystem: code and data

**Code:** workers spawned at `main.rs:715-727`; routes `routes.rs:660-676, 1317-1340`; exports `services/mod.rs:97`, `models/mod.rs:66-67`, `handlers/mod.rs`; indexes `db.rs:1959-2021`; models; scopes `key_service.rs:67-69`; error variants `errors/mod.rs:767-776, 949-960` (numeric block stays reserved with a tombstone note in CLAUDE.md); config `config.rs:303-310, 1079-1112`; ENV rows `docs/ENV.md:371-384`; deny-list entry + tests; approval spend fields (`models/approval_request.rs:137-146`, written by `approval_service.rs:696-743`, exposed by `handlers/approvals.rs:168-188`). Gate: clean `cargo check`, full suite, repo-wide `rg -i "transaction_order|transaction_provider|transactions::"`.

**Data (release gate before the Duffel row is enabled):** migration deletes `approval_requests` documents where `transaction_order_id` exists and `$unset`s spend fields elsewhere; drops `transaction_orders`/`transaction_providers`; records counts, owner, completion; post-migration assertion query proves no transaction identifiers or spend fields remain. Audit-chain rows retained as the documented metadata-only exception pending sign-off (Part I §11); backup/replica retention window stated in the runbook with an owner.

## A.4 PR-D — Duffel row, overlay, skill, manifest entry

Row: slug `duffel`, `service_category: "internal"`, `requires_user_credential: false`, `visibility: "public"`, `auth_method: "bearer"`, `disallow_node_routing: true`, `default_request_headers`: `Duffel-Version: v2` non-overridable + `Accept: application/json`. Operation policy: allow `POST /air/offer_requests`, `GET /air/offers`, `GET /air/offers/{id}`, `POST /air/orders` (mints, `id_path: "data.id"`), `GET /air/orders/{id}` (requires token), `POST /air/payments` (requires token; `resource_token_id_path: "data.payment.order_id"` — **ASSUMPTION**, fixture-verified), `POST /air/order_cancellations` + confirm (require token). List-orders and `/identity/component_client_keys` absent → unreachable through both executors. Overlay registered in `catalog_spec_registry`, added to the weekly drift-guard workflow.

**Manifest entry:** add `payment.complete` to `handlers/assistant_actions.rs` per §3 — description, all-string `params_schema` (`serviceSlug`, `resourceRef`, `resourceToken`, `amount`, `currency`, `payBy`), `risk: "destructive"`, `tier: "v1"`, `remember_eligible: false`; extend `SUPPORTED_ACTIONS` (`assistant_actions.rs:119-134`) and the golden-manifest tests; the contract's destructive→not-remembered rule (`:338-340`) and secret-name normalizer (`:135-154`) must pass unchanged. Coordinate the Aevatar parser addition (Part I §13.6); until it lands, clients render the decline-able fallback.

Skill `nyxid/duffel-travel`: the flow, resource-token lifecycle (arrives in `X-NyxID-Resource-Token` on order create; store in run state; present on order reads, payments, cancellations; reissue via email match), paid-proof, hold eligibility, booking-contact-equals-account-email, cardholder-must-match-traveller. Mandatory sandbox fixtures before enablement: hold eligibility, component-key scope/TTL, 3DS event states, payment body shape, passenger-email path. Regression test: a passenger-laden order body produces no passenger data in any `ApprovalRequest` (the description extractor is allowlist-only, `action_description.rs:16-99` — pinned for Duffel shapes).

## A.5 PR-E — the payment card (frontend)

The `payment.complete` action kind lands in `frontend/src/schemas/assistant-actions.ts` — unknown kinds already render a decline-able fallback on older clients by design (`assistant-actions.ts:74-80`) — with a `payment-card.tsx` block beside the existing card blocks (`frontend/src/components/assistant/blocks/`), reusing the #1349 envelope and completion machinery: itinerary, amount, `payment_required_by` countdown, cardholder-match notice; exchanges the resource token for the component key via `/api/v1/resource-tokens/exchange`; mounts `DuffelCardForm` + Duffel-managed 3DS inline; submits `POST /air/payments` via the proxy with the token; renders success **only** after its own token-bearing read-back satisfies `awaiting_payment == false` and non-empty `documents`; on verified success fires the workflow signal `payment_completed:<order_id>` (order id only, no secrets; spoofing is harmless — the workflow re-verifies). Token-expired and stamp-failed states link the reissue flow. New dependency `@duffel/components` → **wizard bundle rebuild required** (`npm --prefix frontend run build:wizard` + commit `cli/src/wizard/`). Tests: schema fallback, card state machine on a stubbed proxy, callback spoofing, broadcast-before-readback ordering. Gate adds `npm --prefix frontend run build`. (**ASSUMPTION:** the signal-delivery surface from a web client into the workflow engine — Aevatar owner, resolve before this PR.)

## A.6 The booking workflow (Aevatar-side deliverable; specified here, implemented by the Aevatar owner)

Primitive names and parameters verified from `capabilities.v1`; composition to be validated against the engine:

```
search:    connector_call (search + offer reads; no approval, no token)
hold:      secure_connector_call POST /air/orders
             approval.policy: "required"            // the ONE human gate; suspends pre-execution
             approval.service_ref: <duffel>, approval.http_verb: POST,
             approval.resource: redacted itinerary+price, approval.expiration_seconds ≤ offer TTL
             [OPEN: approval.node_id documented as required, but Duffel forbids node routing;
              fallback = human_approval before a plain secure_connector_call]
           → store order_id, payment_required_by, resource token in run state; checkpoint
pay-wait:  emit payment.complete action; loop until payment_required_by:
             wait_signal("payment_completed:<order_id>", timeout_ms ≤ 86_400_000)   // 24 h cap
             on signal OR timeout → token-bearing GET /air/orders/{id}
               paid-proof holds → confirm with reference; end
               unpaid, deadline near → notify once
               lapsed → inform + offer rebook; end
             checkpoint; re-enter wait                                              // covers >24 h holds
guard:     lease/mutex — one workflow run per booking intent (duplicate-booking guard)
fallback:  self_reschedule if looped waits prove awkward
```

## A.7 Gates, assumptions, owners

**Verification gate:** standard CI set (`cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `bash scripts/check-rci-backend-boundary.sh`, `cargo test -p nyxid billing_route_coverage_smoke -- --nocapture`, `cargo build -p nyxid`, `cargo nextest run -p nyxid --profile ci`, CLI build+test) + PR-C's code-and-data gates + the sentinel redaction test + the type-enforced credential audit + PR-E's frontend build and wizard freshness + the manifest golden/contract tests (`assistant_actions.rs:346-409`).

**Assumptions to retire:** Duffel component-key/3DS/`@duffel/components` contracts; payment-body token path; passenger-email field path; Aevatar signal surface; `approval.node_id` semantics; workflow composition details; Duffel idempotency-key absence (nothing depends on it). **Retired 2026-08-14 by live measurement** (`fixtures/duffel-offer-request-sample.json`): holdable-fare share (78%), hold-window distribution (1–3 days — drives the re-entrant wait and the resource-token TTL in `PR_B_PLAN.md`), and price-guarantee-inside-hold (re-price is a live branch). Also confirmed live: the proxy transport works end to end with zero new code, and `Duffel-Version: v2` as a non-overridable default header is necessary (HTTP 400 without it).

**External owners needed:** Duffel sandbox + Cards approval; Aevatar — booking workflow (A.6), `payment.complete` parser acceptance, MCP operation-tool enablement, signal surface, `approval.node_id`, and workflow run state retention/ACLs (Part I §13.1).
