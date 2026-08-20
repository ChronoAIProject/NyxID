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

## The precedent we already run: `chrono-llm-public`

Before treating any of this as new, it is worth noting that **NyxID already operates a platform-credentialed catalog service, and it works.**

The direct assistant route resolves the slug `chrono-llm-public` (`services/assistant_direct.rs:6`) through the admin catalog — `resolve_admin_service_by_slug` at `handlers/assistant_direct.rs:158` — so it is an admin-managed row resolved identically for every caller, not a per-user row. Its billing shape is exactly the one proposed here: `service_category: "internal"`, `platform_billable: true`, `platform_metric: Tokens`, streaming enabled, behind a platform feature flag.

That is the same three ingredients: **an internal catalog row carrying a platform credential, metered on a platform metric.** The catalogue strategy is therefore not a new architecture. It is the generalisation of something already in production.

**But it is the easy case, and the two ways it is easy are precisely the two open problems.**

*It picked the one metric that fits.* LLM usage bills in tokens; `BillingMetric::Tokens` exists; the proxy can count them. Firecrawl bills per page, ElevenLabs per character, Twilio per segment — none of which the three-variant metric enum can represent. `chrono-llm-public` sidesteps metric fidelity rather than solving it.

*It got its safety from hand-written code, not configuration.* The route is text-only with no tool execution, and models, skills and effort levels are validated against fixed allowlists in Rust (`services/assistant_direct.rs:176-217`). That is **parameter-level authorization** — exactly what the generic allowlist cannot do. It is safe because someone wrote a purpose-built handler for one upstream.

The catalogue system proposes to reach the same safety by *configuration* across arbitrary services. That is the right goal, and `chrono-llm-public` is both the proof the shape works and the clearest illustration of what the generic path still lacks: a configurable equivalent of the parameter validation it hard-codes.

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

---

# Enabling the flow: making `chrono-llm-public` the pattern rather than the exception

The question this section answers: **how do we model Chrono LLM as a catalogue service, at platform level, with billing metrics we can adjust later — and turn it on for Duffel flights, Twilio, ElevenLabs, Twitter and Reddit?**

A plan for this exists (`docs/assistant/CATALOGUE_ENABLEMENT_PLAN.md`) and has been through adversarial review. The review returned **REWORK**, and the findings are integrated below rather than appended, because several of them change the design rather than decorate it.

## The shape being generalised

`chrono-llm-public` works because someone wrote a purpose-built handler: one upstream, text-only, no tool execution, with models and skills validated against fixed allowlists in Rust (`services/assistant_direct.rs:176-217`), metered on tokens — the one unit that already matches its vendor. Generalising it means expressing that same safety **as configuration**, across services whose vendors bill in units we cannot currently represent.

Three designs carry the weight: parameter constraints as configuration, extensible billing metrics, and platform-level resolution throughout. The third is uncontroversial and already proven. The first two are where the difficulty lives.

## What survived review

The constraints we most need **are expressible in principle**: Duffel hold-only with `data.payments` rejected; Twilio restricted to Messages with an approved sender; ElevenLabs voice-id membership — though that needs separate path and body rules, since the id appears in both depending on operation. Platform-level resolution through the shared proxy data plane is sound architecture. Snapshotting the Lago metric code per usage row is a valid foundation for changing prices without rewriting history. And sequencing Reddit and Twitter ahead of Duffel and Twilio is the right risk order.

One expressiveness limit worth recording: **equality between two fields is not expressible** in the proposed language. No target operation needs it today.

## Two findings that will break in production

**Anonymous proxying bypasses the policy entirely.** `handlers/public_proxy.rs:65-74` authorizes against `anonymous_endpoints` alone and never consults `proxy_operation_policy`, while forwarding the body at `:90-113`. `validate_anonymous_service_runtime_safety` checks identity propagation and resale billing, but does not refuse an anonymous endpoint on a platform-billable row. A broad anonymous `POST` rule would forward a Duffel order carrying `data.type: "instant"` without the constraint ever running. Public *MCP* execution is blocked (`public_mcp.rs:96-103`), so that surface is not the hole — this one is.

**Per-character and per-segment billing would authorize work while reserving a single unit.** Admission always estimates one unit at the platform layer (`services/billing/reservation.rs:1065-1071`); settlement then bills the measured quantity (`services/billing/meter.rs:480-485`). A prepaid user holding one credit can send a 20,000-character ElevenLabs request or a multi-segment Twilio message, clear admission, and incur the real charge only *after* the provider side effect. Finer-grained metrics make this worse, not better: they widen the gap between what admission checks and what the call costs. Any metric work must land with request-derived reservation and a maximum-unit admission rule, or it converts a billing defect into a spend hole.

## Findings that change the design

**"Metric code as a string" does not escape the enum.** `BillingMetric` is an exhaustive three-variant enum threaded through `ServiceBilling`, `BillingRouteContext`, quantity selection in the meter, and `UsageMeterRow` — and it is **hashed into the tamper-evident billing ledger** (`services/billing/ledger.rs:306-313`). Billing reports parse only `tokens`, `requests` and `bytes`, silently defaulting anything else to `Tokens` (`handlers/billing.rs:264-299`). A Lago code alone cannot describe the unit. Adjustable metrics are achievable, but the honest cost is a coordinated code, API, ledger and report migration — not a config field.

**Per-operation metrics have nowhere to attach.** The plan allows a different metric for searches and bookings, but REST derives one target-level metric before forwarding (`handlers/proxy.rs:4108-4130`) with no selected-operation context, and **MCP hard-codes `BillingMetric::Requests`** (`services/mcp_service.rs:119-139`) regardless of the row. So a Duffel order cannot become `booking` by configuration, and a platform ElevenLabs call through MCP stays request-metered whatever the REST row says.

**REST denies after decrypting the platform credential.** Target resolution runs first (`handlers/proxy.rs:1937-1951`), decrypting the master credential (`services/proxy_service.rs:890-894`), and only then does policy run (`handlers/proxy.rs:2014-2026`). A refused Duffel payment still loads key material. MCP's placement is correct; REST, node and admin surfaces need one pre-resolution evaluator.

**Streaming and WebSocket cannot enforce any of this.** Upgrades deliberately skip body buffering (`handlers/proxy.rs:2138-2163`) and then forward arbitrary client frames (`:4919-4984`). The only real-time usage collector is gated to `llm-openai`. A permitted ElevenLabs streaming handshake can therefore carry arbitrary later text and options, with no frame-level policy and no character meter. **Either a frame protocol policy is designed, or streaming and WebSocket are explicitly excluded from activation.** Excluding them is the honest short-term answer.

**The scoped-key fix must key on IDs, not slugs.** The proposal grants access by catalog slug; existing isolation uses stable UserService ids (`models/api_key.rs:55-73`). Catalog deletion is a soft deactivate and slug uniqueness is checked only among active rows, so a key granted `twilio-platform` would silently inherit access to a *replacement* row reusing that slug. The fix must also reach `AuthUser`, MCP auth context, and OAuth/delegated/relay claims, which today carry service ids only — omitting them yields inconsistent denial, and defaulting missing scope to allow would widen access rather than fix it.

**"Read-only" is not a privacy boundary on a shared account.** The current overlays already expose the authenticated shared identity: Reddit `/api/v1/me` and X `/users/me`. The matcher reasons about method and path only, with no caller or resource-owner predicate, so it cannot express "public resources only" or "never the platform account's own data." Excluding writes is right; calling reads inherently safe is not.

**Twilio needs enforceable controls, not acknowledged risk.** Restricting `From` is necessary and insufficient: the message schema also permits `MessagingServiceSid` — which can select a sender outside the approved set — plus unconstrained `To` and arbitrary `Body`. Destination limits, consent and opt-out, country and cost caps, and per-sender rate limits all need specifying. Excluding the Calls endpoint is sound, and avoids both `Url` and inline TwiML.

**Constraining a body means parsing it, which is new attack surface.** Bodies are buffered and forwarded byte-for-byte under a 100 MB default cap. Duplicate JSON keys, duplicate form fields, non-canonical Unicode, or a deep parse can make NyxID and the vendor disagree about what was sent, or multiply memory and CPU. Duplicate rejection, canonical parsing, depth and field limits, and per-rule body caps are required — not optional hardening.

**Retire Lago metrics on a delay.** Snapshotting the code per row is right, but the old metric must stay live until every reserved, forwarded and unsettled row clears; Lago dead-letters usage whose billable metric is missing.

## What this means for the sequence

The plan's own step one is described as small and is not: it crosses the API-key model, service APIs, auth middleware, MCP transport, JWT and relay claims, CLI and frontend schemas. The activation runbook also omits global billing enablement — `BILLING_ENABLED` and `BILLING_FAIL_CLOSED` both default false — so an operator can follow every row-level step and either serve the five services free or move money with billing fail-open.

The workable order, given all of the above:

1. **Scoped-key access keyed on stable ids**, propagated through every claim carrier — otherwise the intended caller stays locked out.
2. **One pre-resolution policy evaluator** shared by REST, node and admin paths, ahead of credential decryption, with anonymous proxying brought under it or platform-billable rows refused an anonymous endpoint outright.
3. **Parameter constraints**, with canonical parsing and body limits, and streaming/WebSocket explicitly out of scope until a frame policy exists.
4. **Reddit and Twitter reads** as the first activation — lowest consequence, and it exercises the whole chain.
5. **Metric extensibility** as its own migration, landing together with request-derived reservation and maximum-unit admission.
6. **Duffel, then Twilio and ElevenLabs** — Twilio last, because its controls are the most demanding and its failures are the most public.

---

# The five surfaces, in depth

Billing is set aside here by decision. What follows is about **what each service safely offers on a shared credential**, and what it leaks. *Verdicts below reflect adversarial review; two were downgraded from the first draft.*

**At a glance, after adversarial review:** Twitter reads **offer now** (with an app-only credential) · Reddit reads **gated** on commercial terms *and* app-only auth · Firecrawl **not yet** · ElevenLabs **not yet** · Twilio **not yet**.

The first draft of this section said three services could switch on with configuration. Review downgraded two of them, for the same underlying reason in both cases: **the contracts assumed parameter constraints that do not exist in the merged code.** Only method and path are matched today (`services/proxy_authorization.rs:186`). A contract that reads "expose scrape with bounded limits" is not a contract until bounds are enforceable.

## Firecrawl — "read the web for me"

Market research, competitor monitoring, reading a page an agent was pointed at. The synchronous operations are pure configuration: the parameter space carries no money and no impersonation, and a mis-scoped rule costs a failed scrape.

Two findings moved this from "offer now" to "not yet".

**The bodies are open and the bounds are imaginary.** The overlay leaves `scrape`, `search` and `map` bodies unconstrained; search accepts an unbounded `limit` plus arbitrary `scrapeOptions`, and agent submission makes `maxCredits` optional with no ceiling. The proposed contract depends on bounds the matcher cannot express.

**Retry duplication is broader than I said.** The MCP node executor retries *every* method across fallback nodes with a fresh request id (`services/mcp_service.rs:3664`, `:3736`) — so `scrape` and `search` are affected, not only async submission. Platform tools resolve node bindings automatically, and there is no catalog-row `direct_only` control to prevent it. If a node completes a large search and its response is lost, the same body goes to the next node and can fall through to direct execution: the caller sees one result, the vendor bills two or more.

**And async polling cannot be made multi-tenant by configuration.** `GET /v2/agent/{id}` accepts any task id, and no method-and-path rule can prove the id was issued to the calling principal. If a job id reaches a support ticket, a log, or another agent's output, its research topic and results are readable by anyone. That needs either exclusion or a handler storing `(principal, job id)` and checking ownership.

## Twitter/X reads — "what is being said"

Search, username lookup, and a selected user's posts. Writes are excluded outright — a platform credential posting means posting *as ChronoAI*. `/users/me` is excluded too, since it returns the platform's own identity.

Review confirmed the three-route surface is clean: no DM, inbox, saved-item, blocked-list, following-list or quota operation exists in the overlay. **This is the one service that can switch on with merged code.**

One condition, and it is not optional. The privacy result depends entirely on credential class, and the repo's existing X provider defaults to *user OAuth* (`services/provider_service.rs:590`, `:609`). With a shared **user-context** token, a caller could name a protected account that the shared account follows and read content visible only through that relationship. **App-only credentials must be a tested activation invariant, not an assumption.**

## Reddit reads — "monitor the conversation"

Technically the same shape as Twitter: configuration is sufficient, no handler needed. **What gates it is commercial, not technical.** Reddit's API terms for precisely this use — reselling access through a shared credential — are unverified, and this is the kind of term vendors actively enforce. The auth shape for a platform credential here also has no production precedent with us.

Both are marked as assumptions in the underlying document rather than waved through, and the terms check should happen before activation rather than after a suspension.

## ElevenLabs — "give the agent a voice"

The TTS family under voice, model and length constraints is a well-bounded capability *once those constraints exist* — which is why this is "not yet" rather than "offer now". Quota burn is the residual risk beyond that.

Review also found a leak in the proposed operation set: **`/v1/voices` lists voices available to the shared account**, which includes private or cloned voices deliberately kept out of any curated subset. A caller can enumerate their names, ids and metadata; the TTS constraint might stop them being *used*, but it cannot retract the disclosure, and a method-and-path rule cannot filter a response body. Exclude it, publish the curated set as NyxID-owned metadata, or filter it in a handler. `/v1/models` does not have this problem.

The exclusion of realtime and conversational AI *is* structurally sound — exact segment matching means the allowed POST rules cannot reach `/stream-input` or `/convai/...`, and WebSocket handshakes are GET upgrades that match nothing. One defence-in-depth gap worth noting: the shared HTTP client follows redirects by default while authorization checks only the initial path, so a same-origin 307/308 from an allowed endpoint would carry method, credential and body to a disallowed one. No caller-inducible redirect was found.

The sharp finding here refined my own hypothesis. I had guessed ElevenLabs sat "between configuration and a handler, depending on streaming." The more accurate statement: **it is a configuration case if and only if the realtime and conversational-AI family stays excluded.** Include streaming conversations and it becomes a handler case — frame policy, per-session brokering — because realtime frame protocols are out of band and cannot be policed by method and path. **The scoping decision is the mechanism decision**, not a factor in it.

## Twilio — "call or text someone for me"

The one that has to wait, and the reason is instructive rather than merely cautious.

Messages are the only candidate; **call creation is excluded** because `Url` and `Twiml` hand the caller the call's content and control flow, which no field constraint meaningfully bounds. The account-wide message and recording endpoints are excluded too — they have no tenant partition, so one user could read another's phone numbers and recordings.

Even restricted to sending, configuration is not enough. `MessagingServiceSid` can select a sender outside any approved set, `To` is unconstrained, and `Body` is arbitrary. Constraining `From` alone does nothing.

What a purpose-built handler can do that configuration cannot is the interesting part: **recipient verification** — permitting only numbers the *receiving* user has verified through NyxID, which turns "text anyone" into "text me" — plus server-constructed request bodies with no caller-supplied fields at all, content policy hooks, and per-user velocity limits.

So Twilio is the clearest handler case in the set, and it needs a standing compliance decision on sender identity besides. Until both exist it stays dark.

## What this settles about the hybrid

The pattern holds, with one correction to my hypothesis. **Read surfaces are configuration cases** — the dangerous parameter space is thin, and a method-and-path rule genuinely bounds them. **Twilio is a handler case**, for the same reason `chrono-llm-public` is a handler: the safety needed is about *what the call does*, not which endpoint it reaches. **ElevenLabs is a configuration case because of scoping** — not despite it.

That is still a much smaller first step than a universal constraint language — but the honest count is **one service this week, not three.** Twitter reads switch on with merged code, contingent on an app-only credential. Reddit joins it once the terms question and the same credential gate are cleared. Firecrawl and ElevenLabs are configuration cases *after* parameter constraints exist, not before. Twilio needs the handler.

The correction is worth stating plainly because it generalises: **a service contract written against constraints we have not built is a wish, not a contract.** The value of the review was catching two of those before activation rather than after.

