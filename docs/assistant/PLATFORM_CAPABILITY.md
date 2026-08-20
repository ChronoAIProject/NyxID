# Selling Capability to Someone Else's AI

**NyxID Platform Services — an assessment**
*2026-08-20. Sources: the NyxID repo at `travel-allowlist` (rebased on `main`), live vendor probes, and an adversarial review at maximum scrutiny; every repo claim in this document was re-verified against the code before publication. Repo claims cite `file:line`. Assumptions are marked as such. Canonical: supersedes `PLATFORM_SERVICES_PLAN.md`, `PLATFORM_CAPABILITY_SURFACE.md`, and the travel plan lineage.*

---

## The proposition

A user's AI agent is only as useful as the things it can reach. NyxID's bet is that we can be the place it reaches them: one credential boundary, one operation boundary, one audit trail, in front of many services. The user asks their agent to research a market, call a restaurant, or plan a trip; the agent does it through us; nobody handles an API key.

The value scales with the number of services an agent can reach through one integration. That is the whole thesis, and it is a good one — but it makes the *marginal cost of adding a service* the number that matters most, and that cost is not what it first appears.

## Two models, and the fact that separates them

There are two ways a service reaches a user, and conflating them has caused most of the confusion in this workstream.

**BYOK** — the user connects their own account. NyxID stores the credential encrypted and scoped to them and injects it at proxy time. They pay the vendor directly; it costs us nothing.

**Platform catalogue** — ChronoAI holds one credential for everyone. Any permitted user calls it without an account anywhere. We pay the vendor; the user eventually pays us.

The distinction is not cosmetic, and one fact explains everything that follows: **the operation allowlist exists because the credential is shared.** With BYOK there is nothing to protect the user from — it is their key, their quota, their blast radius, and restricting their operations would be paternalistic. With a platform credential, every caller shares one key, so the operation boundary is the only thing standing between one user and everyone else's money, quota and reputation.

That is why platform rows need an allowlist, an access list, and eventually metering, while BYOK rows need none of it.

### Not every service can be both

Identity-bound services — GitHub, Google, Slack, Lark, Microsoft, Discord — cannot sensibly run on a platform credential. A user wants *their* GitHub, not ChronoAI's. These stay BYOK, and the product value is that connecting is easy.

Capability services — Firecrawl, ElevenLabs, Twilio, Duffel, X reads, LLM providers — have fungible accounts. The user wants the capability, not the account. **This is where "platform services" means anything**, and roughly half the existing catalog falls here.

### How they reconcile

The plumbing already exists. Proxy resolution prefers the user's own row and falls back to the catalog (two-tier resolution documented at `handlers/llm_gateway.rs:274-280`, `:641-645`; UserService-first entry at `services/proxy_service.rs:1277`). "Use mine if I have one, otherwise the platform's" is *already built*.

What forbids it is the credential gate: `is_valid_master_credential_service` (`:246`) requires `!requires_user_credential` **and** `provider_config_id.is_none()`. A row is therefore either BYOK or platform-credentialed, by construction. That exclusion is deliberate — it stops a BYOK row silently handing out the platform key — but it is also precisely what blocks dual mode.

Two ways forward. **Separate rows** (`openai` and `platform-openai`) cost nothing and ship immediately, at the price of a catalog that roughly doubles and a choice that leaks into agent prompts. **Dual mode** — one row, platform credential *and* user credentials, user-first — is the experience worth having: *works immediately on our key; connect your own for higher limits*. It requires relaxing a gate hardened days ago, so it deserves its own change and its own review, with eligibility as an **explicit opt-in flag** rather than the current four-condition shape test. Explicit beats inferred for a security gate.

The product story that falls out is clean: **the platform credential is the on-ramp; BYOK is the graduation.**

---

## What we actually have

Here the assessment gets less comfortable, because the honest answer differs from the intuitive one. The repo contains a great deal of machinery. Machinery that exists is not the same as capacity we operate, and the gap between those two things is where every overstatement in this workstream has lived.

**24 curated OpenAPI overlays** exist in `backend/specs/catalog/` — OpenAI, Anthropic, Cohere, DeepSeek, Mistral, OpenRouter, Google AI, GitHub, Google, Microsoft Graph, Slack, Discord, Lark, Reddit, Spotify, Facebook, Firecrawl, ElevenLabs, Twilio, Twitter and more. Each turns a service into typed operations an agent can call with no per-service code. This is a real asset and the curation is largely done.

**But the four capability rows we most want cannot carry a platform credential as they stand.** `api-firecrawl`, `api-twitter`, `api-twilio` and `api-elevenlabs` are BYOK rows carrying `provider_config_id`, which the credential gate rejects by design. Platform variants must be fresh provider-less rows.

**The LLM gateway is not a platform-funded service.** It resolves `UserService`/`UserApiKey` credentials and falls back to personal provider tokens (`handlers/llm_gateway.rs:274`, `:641`); seeded rows carry an empty credential and a provider config (`services/provider_service.rs:3796`, `:3806`). Platform charging is separately opt-in behind a production-default-off flag. It is a strong multi-provider surface, and it is *not* evidence that platform-funded LLM access already works. OpenRouter specifically has no generic-gateway model branch (`services/llm_gateway_service.rs:346-379`), so a seed and an overlay do not add up to gateway support.

**The oracle relay is genuinely differentiated and genuinely problematic.** A logged-in ChatGPT Pro tab as a callable resource is something no aggregator offers. But a pool is *one owner's browser account* (`models/oracle_pool.rs:28-31` — "one ChatGPT (or similar) account whose logged-in browser tabs serve tasks"), and `visibility: platform` makes it submittable by any authenticated user (`services/oracle_pool_service.rs:392-414`). Unrelated users' prompts would execute inside one person's consumer session, with that account's cookies, conversation context, rate limits and retention; bodies persist up to 30 days; follow-ups pin to the account that first answered, which makes rotation and fair pooling hard. There is also a URL-extraction mode that makes the operator's real browser fetch pages using its network position and cookies. As a *personal* capability this is excellent. As *platform* capacity it is an account-sharing, privacy and residency decision that no allowlist resolves. To keep the balance honest: some of that is operable — the account can be a dedicated ChronoAI-ops account rather than anyone's personal session, the URL-extraction mode is disablable per pool (the `OracleExtractDisabled` error class exists, code 11010), and the same small per-user access list proposed below would gate a consented beta cohort who are told, in words, that their prompts run inside a shared consumer account. What no configuration fixes: the provider-ToS exposure of selling consumer-account output as shared capacity, and cross-user prompt commingling in one account's history and retention. A cohort-gated, consent-first beta is viable; a general tier is a decision the owner must make with those two facts in view.

**Channel bots do not ship as platform capability.** Registration accepts Telegram, Discord, Lark, Feishu and Slack only, and requires a caller-supplied bot token stored on a user-owned row (`handlers/channel_bots.rs:403-434`). WhatsApp exists only through per-user OpenClaw mappings, with the unified path still a TODO. A NyxID-owned bot usable by every account is a coherent future product; it is not the present one.

**Node proxy and SSH are user-bound by nature** — they reach a specific user's machines. Real capability, wrong category for a shared credential.

**Some seeded rows would be dangerous as platform rows and were omitted from the inventory entirely:** AWS Cost Explorer (management/payer credentials, per-request charges, consolidated organisation billing) and generic Google Cloud access (caller-selected host, extensible to write scopes). These should be explicitly classified unsafe rather than left unlisted.

---

## Two problems that are structural, not editorial

Most gaps in this space are schedule problems. Two are not, and they bear directly on whether the proposition works at all.

### The allowlist cannot bound parameters

`services/proxy_authorization.rs:186-203` matches HTTP method and path segments. Nothing else. A rule for `/v1/text-to-speech/{voice_id}` matches *every* voice id, including a cloned one. Twilio's allowlisted paths accept arbitrary `To`, `From`, inline TwiML and callback URLs. OpenAI image generation accepts unrestricted `n` and `size`.

The same shape appears in travel: `POST /air/orders` is allowlisted, and the matcher never inspects the body, so an instant-order or inline-payment request passes. The guarantee that "an agent structurally cannot pay" is documentation, not enforcement.

**Endpoint allowlisting is not authorization.** It bounds *which* operations, never *what* they do. Every safety claim of the form "we only allow safe operations" is weaker than it sounds until parameter-level validation exists. This is the single most important correction in this document.

### Scoped agent keys cannot reach platform services

This one is sharper still. `handlers/mcp_transport.rs:642-660` filters tool listings for scoped keys with `Platform => false`, unconditionally — its own comment: *"reject all platform services. Scoped API keys and relay tokens are UserService-only."* Generic REST enforces `allowed_service_ids` against a resolved UserService (`handlers/proxy.rs:1841-1867`) and rejects scoped keys outright on the catalog path (`:1958-1978`). The block is structural, not configurational: `allowed_service_ids` holds *UserService* ids (`models/api_key.rs:32-35`), and a platform row has no UserService — so a scoped key cannot even opt in. Only the human-only assistant path bypasses it.

So an agent issued a correctly-restricted key — the agent-isolation model NyxID recommends — **cannot discover or invoke a platform service.** Removing the restriction to make it work defeats the isolation it exists to provide.

One balance note so this lands accurately: keys default to `allow_all_services: true` (`models/api_key.rs:44-45`), and sessions and the assistant path work — so platform services are not callerless today. What is blocked is precisely the *recommended posture*: the moment a user scopes an agent's key, as the isolation docs tell them to, platform services vanish for that agent. The product is "capability for someone else's AI," and its own best-practice caller is the one excluded by design. Nothing else in this document matters as much.

---

## Access control: the near-term lever

With metering deferred, the allowlist and the access list are the *only* limits on consumption. Today the access options are two: `visibility: "public"`, meaning any authenticated user with no check at all, and `visibility: "private"`, requiring consent to one of the row's developer apps (`services/proxy_service.rs:151`).

**There is no per-user allowlist on a service row.** Control is "everyone" or "whoever consented to app X" — app-shaped, not user-shaped.

The fix is small: a third mode on the same gate, an admin-set list of user or org IDs. One match arm and a field. It is the smallest item in this assessment and the one that most directly answers "decide who can connect."

## Billing, for when it returns

Deferred by decision, but three facts should inform the deferral rather than be rediscovered later.

`platform_billable: true` appears only in tests — **credit billing has never run in production.** A first serious look found reservations that always reserve one unit regardless of metric while settlement bills the full amount, `forwarded` rows that strand a user's credits permanently after a crash, failed upstream responses that still charge, and `BILLING_ENABLED` defaulting false so an apparently-configured paid row serves free.

And `BillingMetric` has exactly three variants — `Tokens`, `Requests`, `Bytes`. Firecrawl bills per page, ElevenLabs per character, Twilio per segment. **Four of five target services bill in units we cannot currently represent.** "Exact unit adapters" therefore means new metric variants on the model, not per-service extractors.

---

## Travel, as the hardest instance

Duffel is worth keeping in view because it stress-tests everything above.

Measured, not assumed: flight search works (a live query returned 602 offers). **Hold orders are available** — real offers carry `requires_instant_payment: false`, so an agent can book without paying and the user settles later. **Stays and Cars both return 403** — *"This feature is not enabled for your account"* — so hotels and cars are commercially gated, not technically missing. There is no Duffel pay-by-link product; Links is a hosted *search-and-book* funnel that accepts no order id.

The design that follows is a good one, and it generalises: **the agent reads, searches and prepares; the human performs anything irreversible, via a link.** Search and hold are allowed. Payment and cancellation are links. Order list and order read are denied, because on a shared credential the list endpoint returns every user's bookings with passenger names.

Two commercial facts belong with it. The customer's card is charged **by the airline** — Duffel *"does not take part in the movement of funds"* — yet ChronoAI is *"contractually liable for any chargebacks or fraud."* We carry the liability on money we never receive. And card payments *"cannot markup or discount the transaction,"* so the convenience fee cannot ride the fare; it is credits or nothing.

---

## What this adds up to

The strategy is sound. The asset is real: 24 curated integrations, a working proxy with credential brokering, an allowlist mechanism merged and independently verified sound at what it does, and a resolution layer that already prefers a user's own credential over the platform's.

The honest correction is that **"ready today" was true for considerably less than claimed.** Nothing is enforced in production — the only operation policy in `main` is a test fixture. The LLM gateway is user-funded. Channel bots are BYO-token. Oracle-as-platform is an account-sharing decision, not a feature flag. Twilio is not fungible even for reads, because its account-wide message and recording endpoints have no tenant partition, so one user can read another's phone numbers and recordings.

**Sequencing that follows from the evidence rather than from enthusiasm:**

Unblock the scoped-key path first — without it the recommended caller model has no path to these services. Add parameter-level validation next, because without it every "safe operations only" claim is aspirational. Add the per-user access list — it is genuinely small and it is what "decide who can connect" means. **Firecrawl need not wait for any of that**: its parameters carry no money or impersonation exposure, so the free-tier proof (sessions and default allow-all keys as callers) can run in parallel while the structural fixes land — a mis-scoped rule costs a failed scrape. Parameter validation is what gates Twilio, ElevenLabs and travel, not Firecrawl.

Twilio and ElevenLabs wait — not for billing, but for parameter validation and tenant isolation. Travel follows, with the Duffel Cards approval request sent now, since that clock runs regardless of what we build.

**Open decisions:** the fee model (none, credits, or a Links funnel); dual-mode versus separate rows; whether oracle-as-platform is acceptable given account sharing; provider contracts, which have not been checked and which matter a great deal — many vendors forbid reselling API access on a shared key, and a suspension would take every tenant down at once.

The uncomfortable summary is that the two hardest problems are not in the roadmap: an agent with a properly scoped key cannot call these services, and the allowlist cannot constrain what a permitted call actually does. Both are fixable. Neither is a schedule item.
