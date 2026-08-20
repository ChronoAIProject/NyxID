# Catalogue Enablement — Making Chrono LLM the Pattern, Not the Exception

> **Superseded (2026-08-21):** replaced by `ONBOARDING_CAPABILITIES.md` — the converged onboarding-scoped plan after two adversarial reviews. Kept as working history.

**Owner ask:** *"How can we model Chrono LLM using catalogue services. This needs to be platform level and billing metrics needs to be adjusted from here in the future. We need this enabled for Duffel flight, Twilio, ElevenLabs, Twitter, Reddit. What can we do to enable this flow?"*

Repo claims cite `file:line` on `travel-allowlist`, rebased onto `origin/main` today. Assumptions are marked. Written for both a non-implementer and a builder.

## The precedent, and why it is the easy case

`chrono-llm-public` is the working proof that a platform-credentialed, platform-billed catalogue service functions in production: an admin-managed catalog row (`services/assistant_direct.rs:6`, resolved by slug via `resolve_admin_service_by_slug`, `handlers/assistant_direct.rs:158`), `service_category: "internal"`, `platform_billable: true`, `platform_metric: Tokens`, streaming, behind a feature flag. Users chat; the platform's LLM credential serves them; token metering bills them. The model the owner is asking for exists and runs.

**But it must be said precisely why it was easy.** Its safety comes from *hand-written Rust*: the endpoint is text-only with no tool execution, and every input — models, skills, effort, message sizes — is validated against fixed, compiled-in allowlists (`validate_direct_request`, `services/assistant_direct.rs:176-217`). And it happens to bill in the one unit (`Tokens`) the billing enum already speaks. Neither convenience generalises: the five requested services need validation that is **configuration, not code**, and three of them bill in units the system cannot currently represent. Those two gaps *are* the design problem, and the rest of this document is their solution plus the per-service contracts.

---

## The enablement flow — in order, each step unlocking the next

1. **Let scoped agent keys reach platform services (small, and nothing works for agents without it).** Today a scoped key is structurally excluded: MCP filters platform tools unconditionally (`handlers/mcp_transport.rs:642-660` — `Platform => false`, its own comment says scoped keys are "UserService-only"), REST rejects scoped keys on the catalog path (`handlers/proxy.rs:1958-1978`), and the scoping field can't even express an opt-in because `allowed_service_ids` holds *UserService* ids (`models/api_key.rs:32-35`) and a platform row has none. Fix: a new `ApiKey.allowed_platform_services: Vec<String>` (catalog slugs) consulted by both the MCP filter and the REST checks; empty by default, so existing scoped keys change behavior not at all. The intended caller of this whole programme is an agent on a scoped key — this ships first.
2. **Parameter-level authorization as configuration** (design below). This is what replaces `chrono-llm-public`'s hand-written validation for arbitrary services. Without it, "hold orders only" and "these voice ids only" are documentation, not enforcement — the current allowlist matches method + path segments and nothing else (`services/proxy_authorization.rs:186-203`).
3. **Billing metrics that can be adjusted and extended from configuration** (design below) — the owner's explicit requirement.
4. **Activate the five services in risk order**, each through the established runbook (policy + constraints → credential → Lago rate for the metric code → `platform_billable` → enable): **Reddit reads → Twitter reads → ElevenLabs → Duffel flights → Twilio**. The first activation is also the first time any operation policy is enforced in production (the only policy in `main` today is a test fixture) — so the first wave doubles as the production shakedown and should be treated as such, with the read-only services absorbing that risk rather than the money-moving ones.

**A question, not silently resolved:** the owner's list adds **Reddit** (new — no prior analysis) and omits **Firecrawl**, which was previously the recommended first proof precisely because a mis-scoped rule there costs a failed scrape. Is Firecrawl dropped, or assumed? If it is still in scope, it slots into wave one beside Reddit as the lowest-risk validator; this plan does not re-add it uninvited.

---

## Design 1 — parameter constraints as configuration

Extend the operation rule (the `#1448` shape) with a declarative constraint block, evaluated in the same pre-side-effect check on both executors:

```
ProxyOperationRule {
  method, path_template,                     // as shipped
  capture_constraints:  { "<name>": InSet([...]) | Pattern("...") },      // path {captures}
  body_constraints: {
    content_type: "json" | "form",           // Twilio is form-encoded; both must be first-class
    require:  { "<field path>": Equals(v) | InSet([...]) | Pattern | NumberRange(min,max) },
    forbid:   [ "<field path>", ... ],
    closed:   bool,                          // reject unknown top-level fields
  }
}
```

Failing any constraint denies before approval, billing, node transport, or forwarding — the same placement discipline the allowlist already has. This single mechanism expresses four of the five:

- **Duffel:** on `POST /air/orders` — require `data.type == "hold"`, forbid `data.payments`, `closed` at the top level. "An agent cannot pay" becomes enforcement instead of documentation.
- **ElevenLabs:** on `POST /v1/text-to-speech/{voice_id}` — `voice_id ∈` a curated platform voice set (a *capture* constraint); voice creation/cloning/dubbing paths simply absent from the policy.
- **Twilio:** on `POST .../Messages.json` (form) — require `From ∈` the approved sender set, forbid `StatusCallback`/`Url`/media fields, `closed`. No other Twilio path is allowlisted at all.
- **Twitter / Reddit:** need **no constraint block** — GET-only method+path rules suffice; the mechanism's absence of writes is the control.

**What one mechanism cannot cover, said plainly:** field constraints bound *structure*, never *content*. Nothing in this design inspects whether an SMS body is spam, a search query is abusive, or synthesized speech is harmful. For read-only services that residual is negligible; for Twilio it is the reason the compliance gate exists and why Twilio activates last — a sender-set constraint pins *who we send as*, not *what we say*. No split mechanism is needed; the split is between what configuration can bound (structure) and what only policy/moderation can (content).

## Design 2 — billing metrics adjustable and extensible from configuration

The constraint today: `BillingMetric` has exactly three variants — `Tokens`, `Requests`, `Bytes` (`models/service_billing.rs:6-11`) — while Twilio bills per segment, ElevenLabs per character, Duffel per booking. Adding an enum variant per unit means a code change and touching every exhaustive match, each time, forever. The fix exploits a fact already in the codebase: **the billing pipeline below the enum is string-shaped.** Usage rows carry `lago_metric_code: String` (`models/usage_meter.rs:66`), and the rate cache is keyed by string code (`BillingRateCache::cache_id`); the enum's only real jobs are choosing a *code* and choosing a *counting method*. So separate those two concerns explicitly:

- **Metric code: a string, configured per service — and per operation.** `ServiceBilling` gains `platform_metric_code: Option<String>`; each operation rule may override it (`billing: { metric_code, unit_source }`), because one service legitimately bills two ways — Duffel searches are requests, a Duffel booking is a booking. New metric = new string + a Lago price. **No schema migration, ever again.**
- **Unit source: a small closed set of counting methods** (this part stays code, deliberately — counting is behavior): `request_count`, `request_bytes`, `response_bytes`, `token_usage`, `char_count(body field)`, `response_field(path)`. These six cover all five services: ElevenLabs per-character = `char_count(text)`; Twilio per-segment = `response_field(num_segments)` (**ASSUMPTION:** Twilio's create-message response carries `num_segments` — fixture-verify at activation, fall back to `char_count`-derived segments); Duffel per-booking = `request_count` on the order-create rule alone.
- **What changing a metric means for existing data:** nothing retroactive, by construction. Usage rows are immutable snapshots carrying the code they were metered under; historical rows settle and report under the old code; the switch affects only rows created after it. The one ordering rule stands: **price the new code in Lago before flipping the config** — reservation fails closed on an unpriced code (`estimate_fresh_credits`, `services/billing/reservation.rs:1102-1115`), which is the safe failure but still an outage if sequenced wrong.
- The `BillingMetric` enum remains for legacy rows (serde-stable), with new resolution preferring the string code — an additive change, not a migration.

## Design 3 — platform level throughout

Every row here is the `chrono-llm-public` shape: admin-managed, `internal`, platform credential, no per-user rows, no `ProviderConfig`. Resolution is identical for every caller — the ordinary slug proxy (`/api/v1/proxy/s/{slug}/...`) for agents and the `resolve_admin_service_by_slug` shape for server-chosen surfaces — with the #1436 credential gate in front of every decrypt and the #1448 policy in front of every forward. Billing metrics are then a *configuration surface on the row* — which is exactly what "adjusted from here in the future" requires.

---

## The five services

| Service | Operations exposed | Constraints (Design 1) | Metric now → eventually | The specific risk |
|---|---|---|---|---|
| **Duffel flights** | `POST /air/offer_requests`, `GET /air/offers`, `GET /air/offers/{id}`, `POST /air/orders` | orders: `data.type == "hold"`, forbid `data.payments`, closed | `requests` → search `requests` + order-create `booking` (per-op override) | Money at the provider; passenger PII transits; payment stays a human link (Duffel Cards, approval pending). Order list/read stay denied — on a shared credential they are every user's bookings |
| **Twilio** | `POST /2010-04-01/Accounts/{sid}/Messages.json` — **only** | form body: `From ∈` approved senders, forbid callbacks/media, closed | `requests` → `sms_segment` via `response_field(num_segments)` (ASSUMPTION above) | **Operates as our identity**: every SMS sends as ChronoAI's numbers — TCPA/spam liability is ours; content is unboundable by constraints (the compliance gate, activation last). **No reads at all**: message/recording endpoints are account-wide with no tenant partition — a read exposes every user's traffic |
| **ElevenLabs** | TTS + streaming TTS with curated `{voice_id}` set; STT; voices/models reads | capture constraint on `voice_id`; cloning/dubbing paths absent | `bytes` → `character` via `char_count(text)` | Impersonation is excluded structurally (curated voices only); residual is quota burn and content of synthesized speech |
| **Twitter/X** | GET search/lookup only | none needed (no writes allowlisted) | `requests` (stays) | Reads run as our app: one shared rate pool (a heavy user starves everyone), and writes would post as ChronoAI — permanently out on this rail |
| **Reddit** | GET listings/search/comments only | none needed | `requests` (stays) | Same shared-identity shape as Twitter, plus two to verify: Reddit's commercial API terms for this use (**ASSUMPTION — check before activation**), and the auth shape (app-only OAuth via the existing declarative `token_exchange` method, `models/downstream_service.rs:43-95` — **ASSUMPTION** that the platform-credential + token-exchange combination is exercised; it has no production precedent) |

**The shared-identity hazard, addressed directly:** Twilio, Twitter, and Reddit all operate as *the platform's own account*. For the read-only pair, "read-only" genuinely holds — reads neither impersonate nor publish, and the exposure is rate-pool fairness plus vendor ToS. For Twilio there is no read-only refuge (account-wide reads leak cross-tenant data) and writes *are* the product — which is why it carries a compliance decision no configuration can substitute for, and activates only after everything else has proven the machinery.

## The two blockers, in the plan rather than assumed away

1. **Scoped keys** — step 1 of the flow; without it the intended caller cannot see these services at all.
2. **Nothing is enforced in production yet** — the first activation is the first live enforcement of any operation policy. The wave order puts read-only services first so the shakedown happens where a defect costs a failed request, not money or reputation.

**Assumptions register:** Twilio `num_segments` in the create response; Reddit commercial terms and token-exchange-on-platform-credential; ElevenLabs streaming byte/char accounting under the relay path; Lago pricing turnaround per new metric code; Firecrawl's status in the owner's list (question above).
