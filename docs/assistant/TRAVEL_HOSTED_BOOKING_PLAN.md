# Agent-Driven Travel Booking — Plan

**The owner's model:** *"User tells them about holiday plans. They simply plan and execute. Show user final bill."* — and on payment: *"There is no payment hosted. So payment is shown in a link for the user to click and complete. A skill might need to be written for agents to know how this works."*

So: the **agent** searches, plans, and executes the booking through NyxID's proxy; the user sees a **final bill** and clicks a **payment link** to complete. NyxID monetizes convenience; NyxID does not process cards. This plan defines that flow end to end, resolves what the payment link actually is (the decisive question, answered from Duffel's docs rather than assumed), specifies the agent skill, and assesses what shipped work carries.

**Fact bases:** `fixtures/duffel-offer-request-sample.json` (sanitized live capture, 2026-08-14) and a fresh live search (2026-08-14, 602 offers, **442 holdable — 73%**; sample `payment_requirements`: `requires_instant_payment: false`, `price_guarantee_expires_at` +1 day, `payment_required_by` +3 days). Duffel Links/Cards/hold-order semantics verified against live Duffel docs 2026-08-17 (citations inline; the Links API reference page itself is currently 404 — schema facts come from the live guide plus Duffel's official clients, flagged where that matters). Repo claims cite `file:line` on `travel-allowlist` (rebased on `origin/main`).

---

# Part I — Narrative

## 1. The flow, end to end

1. **Intent → plan.** The user states holiday intent in chat. The agent turns it into `POST /air/offer_requests` through the proxy (`/api/v1/proxy/s/duffel/...`, transport already proven live with zero new proxy code), reads offers, and reasons about price/stops/timing/carrier. This is the product; it happens in conversation.
2. **Selection → hold.** The agent re-fetches the chosen offer for a live price, collects passenger details in conversation, and creates a **hold order** (`type: "hold"`, no `payments` key — *"no payment takes place at the time of booking"*, duffel.com/docs/guides/holding-orders-and-paying-later). 73% of measured offers are holdable; non-holdable fares are declined conversationally with alternatives. Seats reserved, **no money moved**, self-expiring.
3. **The final bill.** The agent presents: itinerary, total amount/currency (from the order response), `price_guarantee_expires_at`, and `payment_required_by` — *"pay by Fri 21:00 or the hold lapses; the price is locked until Thu 21:00."*
4. **The payment link.** The agent posts a link to a **NyxID-hosted payment page** for that order (what this page is and why — §2). The user clicks, sees the live bill re-fetched server-side, enters their card into **Duffel's embedded form**, passes their bank's 3DS, and the payment goes card → airline.
5. **Confirmation.** The page verifies the paid proof (`awaiting_payment == false` **and** ticket documents issued — Duffel's own rule) before showing success; the agent independently reports the booking reference. The airline/Duffel confirmation email is the user's durable record.

If the user never pays: the hold lapses at `payment_required_by`, *"the space will be released by the airline"*, and the order is dead — recovery is a **new** order, there is no revive (verified, holding-orders guide). Nothing to clean up anywhere.

## 2. What the payment link is — resolved, not assumed

The three candidate answers, against verified facts:

- **A Duffel-generated payment surface for the held order: does not exist.** Duffel Links is a one-time, 24-hour, full *search-and-book* funnel; its session schema (`reference`, three redirect URLs, branding, currency, markup fields, product toggles, `open_path`) accepts **no order id, no offer id, and no checkout-only mode** (live guide + both official clients). No pay-by-link product exists anywhere in Duffel's docs. This matches the owner's own statement; it is now also a verified negative.
- **A third-party processor link (Stripe-style), NyxID collects, then pays Duffel from balance: exists but is the rejected model.** It puts ChronoAI back into collecting customer money, pre-funding Duffel Balance, and carrying refund/chargeback reconciliation — the float model the owner already ruled out. Not proposed.
- **A NyxID-hosted page embedding Duffel Cards: the answer.** Duffel Cards *"can also be used to pay for hold orders"* (duffel.com/docs/guides/paying-with-customer-cards): the page mounts `DuffelCardForm` (`@duffel/components`) with a server-minted component client key → `createCardForTemporaryUse` → `createThreeDSecureSession(card, resource_id = the hold order)` — hold orders are an accepted 3DS resource type (duffel.com/docs/api/v2/threedsecuresession) — → `POST /air/payments` `{type: "card", three_d_secure_session_id}`.

**Who touches the card, in exact terms:** card data goes browser → Duffel (PCI DSS Level 1); NyxID hosts the page but never sees a PAN, landing in **SAQ-A, the lightest PCI self-assessment**. If NyxID instead built its own card fields, NyxID would be **in PCI scope — a materially different product and a serious cost**; nothing here proposes that, and the plan's guardrail is that no card field ever renders outside Duffel's iframe. The travel supplier is merchant of record on the card leg; ChronoAI is contractually liable for chargebacks/fraud (duffel.com/docs/guides/collecting-and-making-payments — a standing commercial fact to note, not new). **Gate:** Duffel Cards requires approval from Duffel support before integrating — that request is the single most important external action this plan creates.

## 3. The convenience fee — the honest answer is uncomfortable

**On this rail, the fee cannot ride Duffel markup.** Verified verbatim: *"you cannot markup or discount the transaction as card payments must be for the exact amount quoted by the travel supplier"* (collecting-and-making-payments guide). Duffel-side markup (`markup_amount`/`markup_rate`, remitted through Duffel) exists **only** where Duffel Payments collects — i.e., inside Links' hosted search-and-book funnel, which is not this product's flow. So, options for Calvin, stated plainly:

1. **No fee in v1** (recommended to start): the product's value story first; zero build.
2. **Fee in NyxID credits via existing per-call metering** — the Duffel catalog row's search/booking operations are meterable with the existing `ServiceBilling` per-request machinery (platform metering, reserve-before-forward, settle-after — all shipped and battle-tested). This is *plain existing metering*, not the REWORKed transactional-settlement design, which stays dead. Cost: users need credit balances; the fee decouples from booking value.
3. **A secondary Duffel Links funnel** ("browse and book on Duffel yourself") where markup *does* work and remits through Duffel — a complement, not the agent-driven product. (Markup remittance mechanics: **UNVERIFIED** beyond "retained via Balance"; needs a dashboard/support check before relying on it.)

If NyxID never collects and markup is unavailable on the card leg, options 1/2 are the whole space. Decision needed; nothing below depends on it.

## 4. Expiry, re-quote, and what the agent says

Measured windows are short and asymmetric: price guarantee ~1 day, payment deadline ~3 days. Between them the seat is held but the price can move (*"the price can change"* when the guarantee is null/passed; stale amounts are rejected with `price_changed` — verified). Behavior: the payment page **always re-fetches the order server-side** before rendering the bill (required anyway — *"always re-retrieve the order before paying"*); a price change between bill and payment is surfaced on the page and by the agent as a normal re-confirm branch, never silently absorbed. When `payment_required_by` passes unpaid, the agent — who retains the deadline from the creation response — tells the user the hold lapsed and offers to re-book (new search → new hold; prices may differ). The agent initiates this proactively when the user returns, by comparing now against the retained deadline — no NyxID timer exists or is needed.

## 5. Order read-back — reopened honestly

The `GET /air/orders/{id}` block shipped in `#1448`'s policy design assumed nobody legitimate needed reads. Under this product, two parties do: **the payment page** (it must re-fetch the live bill and verify paid-proof) and **a returning user** asking "what did I book?". Options:

- **(a) The agent retains the creation response** (order id, reference, amounts, deadlines) in conversation/workflow state and answers from it. Zero NyxID change; fails only if the conversation is lost — then the fallback is the airline/Duffel email. 
- **(b) Allowlist `GET /air/orders/{id}` on the generic proxy.** Rejected: it re-opens the cross-user bearer-id exposure on the shared credential that the allowlist exists to prevent.
- **(c) A narrow NyxID payment endpoint that reads the order server-side** — not a caller-addressable proxy operation; used only by the payment page to render the bill and verify payment. Whoever holds the link can view that order's bill and pay it *with their own card* — an accepted bearer-link risk, bounded by unguessable `ord_…` ids, links shared in the user's own chat, and payment requiring the payer's own 3DS.

**Recommendation: (a) + (c).** (c) is required regardless for payment; (a) covers "what did I book" without any new NyxID surface; the generic-proxy block stays exactly as shipped. The skill (§7) teaches (a) explicitly.

## 6. What NyxID stores, and what carries forward

**Stores: nothing new.** No order rows, no order↔user mapping, no passenger data, no payment state. The payment page is stateless (order id in the URL; everything else re-fetched from Duffel per request). Attribution lives in the conversation and the payer's own card/3DS. This is the minimum, and it is sufficient because every question the system must answer at runtime ("what's the bill?", "is it paid?") is answerable from Duffel with the order id.

**Carries forward — the earlier "probably moot" assessment is withdrawn:** `#1436` (credential gate + redaction) protects the shared credential; **`#1448` is load-bearing again** — search *and* order creation flow through the proxy, so the allowlist is what makes the shared credential safe. The Duffel policy becomes: `POST /air/offer_requests`, `GET /air/offers`, `GET /air/offers/{id}`, `POST /air/orders` — **and deliberately not `POST /air/payments`**: payment goes through the NyxID payment endpoint only, so **an agent structurally cannot pay**; the human's card + 3DS is the only payment path. Also revived: the V-era payment-page/card-component design (now as the link target) and the uncommitted `docs/plans/duffel-air-stays-cars.md` draft's substrate items (platform-seed shape with literal-empty-ciphertext sentinel, admin credential set/clear + CLI, catalog header precedence fix, deny-before-decrypt hardening — its booking/payment-through-proxy scope for Stays/Cars stays parked: both returned live **403 "not enabled for your account"**, commercially gated).

## 7. The skill — a first-class deliverable

`skills/duffel-travel-via-nyxid/SKILL.md`, following the in-repo conventions of `skills/github-via-nyxid/SKILL.md` and the authoring guide in `skills/nyxid-service-skill-authoring/` (frontmatter + operational doc; wired to the catalog row via `recommended_skills`, backfill precedent at `provider_service.rs:3310-3329`). Content contract in Appendix A.3 — it teaches the full loop *and, above all, the failure modes*: an agent that double-books on retry or silently loses a hold is worse than one that refuses.

---

# Appendix — Implementation contract

## A.1 The Duffel row and policy (backend)

Row per the parked draft's settled decisions: slug `duffel`, `https://api.duffel.com`, public/internal/no-provider, `Duffel-Version: v2` non-overridable (live-verified: 400 without), seeded **inactive with literal `Vec::new()` ciphertext** (never `encrypt(b"")` — indistinguishable-from-present, `handlers/services.rs:828, 889-917` encrypts empty plaintext). Policy (always explicit; the merged mechanism keeps `None` = passthrough, `handlers/proxy.rs:2013-2029`): the four operations in §6, method+template rules. Enforcement as merged: REST check before approval/billing/forward (`handlers/proxy.rs:2021`), MCP via `prepare_proxy_tool_call` before approval persistence (`mcp_transport.rs:1357, 1695`; `mcp_service.rs:2892-2936`). **Known hardening gap, stated:** the REST check runs after target resolution, which decrypts the credential before a denial (nothing forwards, but decrypt-before-deny survives; the draft's split-resolution fix remains a wanted follow-up, not a blocker). Substrate from the draft's Step 1A that this plan adopts: admin credential subresources (`PUT/DELETE /api/v1/services/{id}/credential`) + `nyxid admin service credential set|clear` + `enable|disable`, and the catalog-header precedence fix (catalog `overridable: false` must beat user-service layers on all transports).

## A.2 The payment endpoint and page

- `GET /api/v1/travel/orders/{order_id}/bill` — NyxID-owned (not the generic proxy): fetches the order server-side via the server-chosen credential gate (`authorize_master_credential_server_chosen`, public rows only, `proxy_service.rs:217`), returns bill projection (itinerary summary, totals, deadlines, `awaiting_payment`, documents-present). Bearer-link access model per §5(c), stated in code comments. Rate-limited; no listing variant exists.
- `POST /api/v1/travel/orders/{order_id}/component-keys` and `POST .../payments` — mint the Duffel component client key server-side; accept `{three_d_secure_session_id}` and submit `POST /air/payments` `{type: "card", amount, currency}` using the **re-fetched current totals** (never client-supplied amounts); map `price_changed` to a re-confirm response, `past_payment_required_by_date` to a lapsed response (enum verified: duffel.com/docs/api/v2/payments/create-payment). Human session **not** required (the payer may not be the NyxID account holder — accepted bearer-link model), but agent/API-key/delegated callers are rejected: this surface is for browsers.
- Page `frontend/src/pages/` `/pay/duffel/:orderId`: bill render from the endpoint, `DuffelCardForm` + 3DS (Duffel-managed frames), paid-proof verification before success (`awaiting_payment == false` **and** documents non-empty), lapsed/price-changed/failed states with "ask your assistant to re-book" copy. `@duffel/components` dependency → **wizard bundle rebuild** (`npm --prefix frontend run build:wizard` + commit `cli/src/wizard/`). No card field outside Duffel's iframe, asserted in review.

## A.3 The skill (content contract)

Frontmatter + sections, in flow order, each with real request/response snippets from committed fixtures:

1. **Intent → offer request:** slice construction (origin/destination/dates/cabin), passenger counts; `POST /air/offer_requests` via `/api/v1/proxy/s/duffel/...`; expect large result sets (measured 602).
2. **Reading offers:** `total_amount`/`total_currency` are decimal strings — never parse to floats for comparison display; filter on `payment_requirements.requires_instant_payment == false` (73% measured); weigh price, stops, duration, carrier; re-fetch the chosen offer (`GET /air/offers/{id}`) for a live price before proposing.
3. **Hold, never pay:** `POST /air/orders` with `type: "hold"`, passengers, **omit `payments`**; the agent must never attempt `POST /air/payments` (it is not callable through the proxy — by design).
4. **The final bill:** present totals + both deadlines verbatim; explain the price guarantee vs seat hold distinction.
5. **The payment link:** `{FRONTEND_URL}/pay/duffel/{order_id}`; say the card is entered on a secure Duffel form and NyxID never sees it; after the user reports paying (or on next turn), confirm with the retained booking reference — do not claim "paid" without the page/user confirming.
6. **Expiry & re-quote:** retain `price_guarantee_expires_at` and `payment_required_by` from the creation response; on any later turn, compare against now; lapsed → say so plainly and offer a fresh search; guarantee-passed-but-unpaid → warn the price may have moved and the page will show the live amount.
7. **What you cannot do:** no order list, no order read (`GET /air/orders*` blocked) — **retain the creation response**; if the conversation lost it, the user's recovery is the airline/Duffel confirmation email, and re-booking is the offer.
8. **Failure modes (the largest section):** order create timeout/ambiguous error → **never blind-retry** (Duffel has no idempotency keys — assumption consistent with docs; a retry can double-book); tell the user the outcome is unknown, wait, and only re-book with explicit user confirmation. `price_changed` at any step → re-fetch, re-present, re-confirm. Passenger-detail validation errors → fix and retry is safe (no side effect occurred). Non-holdable-only results → present instant fares as "book on the airline's own site" or offer alternatives; never attempt instant orders (they require payment at create, which the agent cannot do). When uncertain — wrong-looking name, ambiguous dates, price jumped >X% — **stop and ask the user**.

## A.4 Sequence and gates

- **PR-1:** substrate (A.1 items: seed shape, credential admin + CLI, header precedence) — backend, no Duffel row active.
- **PR-2:** Duffel row (inactive) + overlay (4 search/hold operations) + policy + skill + fixtures; activation is the two-step audited runbook (credential set → enable) from the draft plan.
- **PR-3:** payment endpoint + page (frontend; **gated on Duffel Cards approval** — request it now; sandbox has 3DS test cards so build/test proceeds pre-approval in test mode).
- **External:** Duffel Cards approval request (decisive), Aevatar MCP operation-tool enablement (standing), fee decision (§3, Calvin), Stays/Cars entitlement (commercial, parked — both 403 today).
- **Verification gate:** standard CI set; policy parity + negative tests per the draft's Step 2/3 checklists (denied reads never reach a mock upstream, denial precedes approval/billing on both executors); the paid-proof and price-changed branches fixture-tested; no card field outside Duffel's iframe.

**Assumptions / unverified, carried openly:** Duffel Cards approval timeline and any account-country constraint; markup remittance mechanics (only relevant to fee option 3); Links redirect params on failure/abandonment (only relevant to option 3); idempotency-key absence (design never retries writes); `open_path` semantics (unused here); Stays/Cars via proxy or Links (entitlement-blocked, 403 measured).
