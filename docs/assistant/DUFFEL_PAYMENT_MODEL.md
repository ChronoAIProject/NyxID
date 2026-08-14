# Duffel Payment Model — Who Pays, and How

**Verified against duffel.com/docs and duffel.com/payments on 2026-08-13**, public documentation only.

**Owner decision (2026-08-13):** *"All transactions are to be borne by the customer themselves."* ChronoAI does not front the fare. This document records which Duffel payment rails can deliver that, which are closed to us, and what each implies for the architecture.

---

## 1. The answer: Duffel Cards

**The customer's card pays the airline directly. ChronoAI never holds the money.**

> *"Duffel Cards provides a PCI-compliant way for your customers to pay airlines and accommodation providers directly for their bookings."*

And on custody, from the payment-method comparison: **the travel supplier is merchant of record; funds flow directly from the traveller's card to the supplier.**

### Flow

1. Assistant searches; user selects an offer.
2. **`DuffelCardForm`** (web component, `@duffel/components`) collects the card in the browser → creates a `Card` resource.
3. **`createThreeDSecureSession()`** with the card id → `ThreeDSecureSession`.
4. Create the order with `three_d_secure_session_id` in the `payments` array (`Payment` type `"card"`).

### Properties

| Property | Value |
|---|---|
| Merchant of record | Travel supplier (the airline) |
| Who holds funds | Nobody in our path — card → supplier |
| Card data on our servers | **Never.** Duffel is PCI DSS Level 1 |
| Our PCI scope | **SAQ-A** — the lightest self-assessment |
| Works with hold orders | Yes — also order changes and Stays |
| Test mode | Yes, with 3DS test card numbers |
| Eligibility | **Approval required** — contact Duffel support before integrating |
| Country restriction | None stated |
| Closed to new customers? | **Not stated** (unlike Payment Intents — see §2) |

### Constraints

- **Cardholder name must match the traveller name.** This is genuinely "the customer pays for their own ticket."
- **Agency cards are not widely accepted for flights** and need written supplier permission.
- Some suppliers charge a processing fee for card versus cash purchases.

---

## 2. What is ruled out, and why

| Rail | Status | Why |
|---|---|---|
| **Payment Intents** (collect card → top up your Duffel balance → pay) | ❌ **Closed** | *"We are currently not accepting new customers on to this product."* |
| **Duffel Links** (fully hosted search-and-book page) | ❌ Likely blocked | Requires the Duffel org to be in a **Duffel-Payments-supported country**. Also removes the assistant from the flow, which defeats the product |
| **Balance / Cash** (pre-funded wallet) | ⚠️ Open, but rejected | This is ChronoAI fronting the fare — carries float, FX risk, and refund liability. Contradicts the owner decision |
| **Duffel Cards** | ✅ **The path** | §1 |

---

## 3. What this eliminates from the architecture

The customer-pays model removes most of what the previous designs struggled with:

- **Float / treasury** — gone. No pre-funded balance to maintain or monitor.
- **FX and price drift on the principal** — gone. Not our money at any point.
- **Refund liability for the fare** — gone. The airline refunds the cardholder directly.
- **Charging credits for the fare** — gone. The fare never enters NyxID's credit system.
- **The markup-as-risk-budget question** — dissolved. Markup was sized to absorb Duffel fees, FX movement and refund-window risk on a fare we were fronting. With no fronting, any ChronoAI charge is a **service fee**: small, fixed, per booking, and trivially chargeable through the existing per-call metering.

The whole class of problems that consumed three adversarial review rounds — reserve/settle across a human approval, hold/settlement atomicity, dead-letter recovery for a variable principal — does not arise, because **NyxID never moves the fare.**

---

## 4. The one real constraint: card collection needs a UI

`DuffelCardForm` is a **browser component**. The user must enter card details into a rendered form, so the payment step cannot happen inside a chat message.

This is less disruptive than it appears, because it fuses with the approval gate the design already has. Instead of *"approve this spend from your credits"* on your phone, the interruption becomes *"enter your card to complete this booking"* on a NyxID-hosted page carrying the Duffel component. Same single interruption, and NyxID still never sees the card.

Design implications to settle in the plan:

1. **Where the card step is hosted** — a NyxID page, and how the assistant hands off to it (a one-time link delivered through the existing approval/notification channel is the obvious candidate).
2. **What the approval record means now.** It is no longer authorising a credit spend; it is authorising *a booking* and carrying the price the human agreed to. The `spend_*` fields still apply, but as disclosure rather than a debit ceiling.
3. **Passenger and cardholder identity.** Cardholder name must match the traveller, which constrains who can book for whom — a product rule worth stating.
4. **Whether ChronoAI charges a service fee at all**, and if so whether it is taken in NyxID credits at booking time.

---

## 5. Actions

1. **Request Duffel Cards approval** — this is now the single most valuable commercial ask, alongside a sandbox account. It is a support request, not a KYC/onboarding blocker.
2. **The earlier commercial questions shrink.** Balance funding, Duffel Payments' 22-country list, and seller-of-travel float obligations all become irrelevant if we never front money. What remains is Duffel Cards approval and standard account activation.
3. **Rework the plan** on a customer-pays basis — `TRANSACTIONAL_SERVICES_PLAN_V2.md`'s money sections (credit reserve/settle for the fare) are superseded by this document.
