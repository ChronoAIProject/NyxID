# Duffel API — Fact Verification

**Verified against duffel.com/docs and help.duffel.com on 2026-08-13**, against public (unauthenticated) documentation only.

**Why this exists.** `TRANSACTIONAL_SERVICES_SPEC.md` §13 labelled a set of Duffel facts "verified", and the round-1 review (finding N5) correctly objected that several could not be confirmed from public sources. Those facts are load-bearing: they shape the adapter trait that PR-2 implements. This document re-checks them and records what actually holds, what was wrong, and what remains unknown.

**Scope note.** PR-1 contains no Duffel code. What is checkable today is whether the *contracts* encoded in `TransactionProviderAdapter` match Duffel's real semantics — so that PR-2 implements against a correct seam rather than reshaping it.

---

## 1. Confirmed — the design's load-bearing claims hold

| Claim | Status | Source |
|---|---|---|
| `Duffel-Version: v2` required on every request | ✅ | *"You'll need to send a `Duffel-Version` header with each request"* |
| `x-client-correlation-id` exists and is caller-settable | ✅ | *"allows you to set your own client identifier per request/response"* |
| Hold orders: `type: "hold"`, omit `payments` | ✅ | Holding-orders guide |
| `payment_required_by` / `price_guarantee_expires_at` semantics | ✅ | Holding-orders guide; also on the offer's `payment_requirements` |
| `payment_requirements.requires_instant_payment` gates hold eligibility | ✅ | Offers schema |
| Offers expire, **typically within 30 minutes** | ✅ | *"only available… for a limited time… typically within 30 minutes"* |
| Search prices are not guaranteed; re-fetch the single offer before booking | ✅ | *"search prices returned by airlines are not guaranteed to be available at the time of booking"* |
| `passenger_identity_documents_required` → passport required per passenger | ✅ | Offers schema |
| `price_changed`, `schedule_changed` error codes | ✅ | Holding-orders guide |
| `X-Duffel-Signature: t=<ts>,v1=<hex>`, HMAC over `"{t}.{raw_body}"` | ✅ | Webhooks guide — **exact grammar confirmed; previously unverifiable** |
| Webhook signing secret returned once, never retrievable | ✅ | *"it's only available at the time when you create a webhook"* |
| Webhook payload carries `idempotency_key` for dedupe | ✅ | Webhooks guide |
| Balance is a pre-funded wallet, funded by bank transfer | ✅ | Payments guide (the "Cash" method) |
| Pre-funding ties up capital — the platform carries float | ✅ | Listed by Duffel as a disadvantage of Balance |

### The B1 finding is confirmed verbatim

The round-1 review's most consequential catch — that `awaiting_payment == false` does **not** prove payment — is stated outright by Duffel:

> *"If you don't pay for a flight before the time indicated in `payment_required_by`, the space will be released by the airline and the `awaiting_payment` status of the order will be set to `false`."*

And the correct check is explicitly documented:

> *"retrieve the order again and check that `awaiting_payment` is now set to `false` **and that `documents` have been issued for the order**."*

**Our adapter contract is right.** The three-valued `PaymentClassification` (`Paid` / `ExpiredUnpaid` / `Indeterminate`), with `Paid` requiring positive evidence rather than a single boolean, matches Duffel's own instruction. Had we shipped the original single-boolean check, we would have settled expired unpaid holds as successful bookings.

---

## 2. Corrections — spec §13 claims that were wrong or overstated

| Spec claim | Reality | Consequence |
|---|---|---|
| "Duffel Payments (card intents) is **closed to new customers**" | *"This payment method requires approval to access"* — restricted, not closed | Overstated. **Moot either way:** we pay airlines from Balance and collect from users via NyxID credits, so Duffel Payments is not in our path at all |
| Webhooks retry "for **72 hours**" | Docs say only *"our system will retry failed events"* — no duration published | Downgrade to unverified. Do not size the reconcile window against 72 h |
| Order amounts are returned "in the org billing currency" | Not stated in public docs | Downgrade to unverified. Currently moot (FX descoped with money), but must be settled before pricing ships |
| Webhook events: `order.created`, `order.airline_initiated_change_detected`, `order_cancellation.created` | Public list also includes **`order.updated`** and `ping.triggered`, and Duffel states the list is not exhaustive | Add `order.updated` to PR-2's handler. Confirms the spec's own conclusion that the **poll sweep is mandatory, not optional** |

---

## 3. Still unverified

Public docs are insufficient; these need authenticated docs, an account, or Duffel support.

- **Blanket absence of request idempotency keys.** No mention anywhere in the request documentation, which is consistent with our assumption — but absence of evidence is not proof. This drives the entire one-attempt-then-reconcile design, so confirm it with Duffel directly before PR-2 hardens around it.
- **Whether list-orders can filter on `metadata`** for reconciliation matching. Affects reconcile-sweep cost, not correctness.
- **One webhook endpoint per environment/organisation.** Not addressed publicly. Affects how staging and prod share a Duffel org.
- **Client timeout guidance** (spec claimed ≥130 s) and **live rate limits** — neither is published.

---

## 4. Commercial: the picture is better than recorded

The open blocker was logged as *"can a Singapore-incorporated seller onboard?"* The public docs narrow it considerably.

- **The 22-country restriction applies to Duffel *Payments*, not to accounts.** That list governs card acceptance from end customers — a capability **our architecture never uses**. We pay airlines from Balance and collect from users through NyxID credits and Lago. The most-cited blocker is very likely irrelevant to us.
- **Managed Content is auto-enabled on activation** — *"Once you complete your account activation, this service is automatically enabled."* No ARC/IATA accreditation and no ticketing plate required.
- **The 5 points of sale (AU, FR, IE, UK, US) are sales markets, not seller-domicile restrictions** — a distinct question from where ChronoAI is incorporated.
- **Account activation is KYC-gated per country**, and that remains the genuine unknown for a Singapore entity.

### The finding that de-risks sequencing

> *"Until information has been reviewed and approved through Verification, your Account is available on a preliminary basis only, allowing you to access a **test environment**… and build an integration between your platform and the Duffel Platform."*

**PR-2 can be built and tested against the sandbox before the commercial question is resolved.** Combined with Duffel Airways (ZZ) sandbox support for create/cancel/change, the adapter can be implemented and contract-tested end to end with no commercial commitment and no money at risk.

The residual commercial questions are narrower than logged, and are for Duffel support rather than for us:

1. Can a Singapore-incorporated entity complete KYC/activation for a Balance account?
2. Can a seller domiciled outside the 5 POS markets sell into them?
3. What are live rate limits and the actual webhook retry window?

---

## 5. Actions

1. **Spec §13:** apply the §2 corrections; relabel §3 items as assumptions rather than verified facts.
2. **PR-2:** handle `order.updated`; keep the poll sweep mandatory; do not size reconciliation against an unverified 72 h retry window; confirm idempotency-key absence with Duffel before relying on it.
3. **Overview §7 decision 2:** restate the commercial blocker in its narrower form, and record that sandbox development is unblocked today.
4. **No adapter-trait change required.** The seam — `quote` / `repricing_check` / `execute` / `reconcile` plus three-valued payment classification — matches Duffel's documented semantics, including the re-fetch-before-book recommendation and the documents-issued payment proof.
