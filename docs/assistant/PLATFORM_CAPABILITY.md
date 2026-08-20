# Selling Capability to Someone Else's AI

> **Superseded (2026-08-21):** replaced by `ONBOARDING_CAPABILITIES.md` — the converged onboarding-scoped plan after two adversarial reviews. Kept as working history.

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

# The three capabilities

Defined by what a user wants done, not by vendor. Every operation below is justified by that intent; anything that could not be is cut, because an unneeded operation is security surface for no product gain.

## 1. Social listening and research — Reddit, X, Firecrawl

**What a user gets.** "What is being said about my product, on Reddit, on X, and on the open web — and read me the sources." The sensing half of research and marketing agents.

**Minimal operation set, each justified by intent — and what was cut:**

| Op | Intent served |
|---|---|
| X `GET /tweets/search/recent` | topic/brand listening |
| X `GET /users/by/username/{username}`, `GET /users/{id}/tweets` | monitor a named account |
| Reddit `GET /r/{subreddit}/hot`, `/new` | monitor a subreddit |
| Reddit `GET /search`, `GET /comments/{article}` | topic search; read a thread |
| Firecrawl `POST /v2/scrape`, `POST /v2/search` | read a page; search the web |
| Firecrawl `POST /v2/map` | site-structure research ("what does this company publish") |

Cut, with reasons: X `POST /tweets`, `DELETE /tweets/{id}` (write/destroy as ChronoAI), `GET /users/me` and Reddit `GET /api/v1/me` (the platform's own identity — invites targeted reports; serves no user intent), Reddit `POST /api/submit` (posts as ChronoAI), Reddit `GET /r/{subreddit}/about` (marginal — monitoring does not need it; it can return with a one-line justification if an agent proves the need), and **Firecrawl's async pair** `POST /v2/agent` + `GET /v2/agent/{id}`: the poll is bearer-by-job-id over account-scoped jobs — one caller reading another's research topics — and **cannot be made multi-tenant by configuration at all**; excluded rather than constrained.

**DB configuration that makes it safe.** X and Reddit: method+path rules only — the shipped matcher already expresses them, which is why they ship first. Firecrawl needs the constraint engine (§4): `url` fields `Pattern("^https?://")`, `limit`/`maxCredits`-class fields `Range`-bounded, bodies `closed` — today those fields are open and unbounded, which is exactly why Firecrawl is **not** "offer now" despite being read-shaped. Firecrawl's row also sets `direct_only: true`: the MCP node executor retries *every* method across fallback nodes with fresh request ids (`services/mcp_service.rs:3664`, `:3736`), duplicating vendor jobs on failover — a per-row routing flag in the DB, honored by engine code that ships once (§5).

**What the operator supplies.** Firecrawl API key; X app-only bearer; Reddit app credentials via the declarative `token_exchange` method (**ASSUMPTION:** that method on a platform credential has no production precedent). **Invariant:** the credentials must be app-only — the provider defaults are user-OAuth and would make "reads" run as a person.

**Cannot be expressed in configuration:** nothing, for the operations that ship. The things that couldn't be — multi-tenant async polling — were cut instead.

## 2. Calling and getting things done over the phone — Twilio + ElevenLabs

**What a user gets.** Today's slice: the agent can *speak* (text → audio, returned to the user as their own artifact) and — later — *text a phone*. The full capability the pairing implies (the agent talks on a live call: ElevenLabs voice over a Twilio call) is explicitly future: it requires a NyxID-owned TwiML surface, which is code, and is out of scope until the pieces below exist.

**Minimal operation set — write-only, both services:**

| Op | Intent served |
|---|---|
| ElevenLabs `POST /v1/text-to-speech/{voice_id}` (+ `/stream` — plain HTTP chunked audio, not the realtime protocol) | "say this"; the response *is* the artifact, `audio/mpeg` straight back to the caller |
| Twilio `POST /2010-04-01/Accounts/{AccountSid}/Messages.json` | "text this number" — *when it ships* |

Everything else is absent, and absence is the enforcement: **all six Twilio GETs** (account-wide message/recording/call listings — reads of shared private state with no tenant partition), `POST .../Calls.json` (caller-supplied `Url`/`Twiml` hands over the call's control flow — write-only does not fix control flow), **all eight ElevenLabs GETs** (`/v1/voices` exposes the account's voice inventory; the convai family exposes every user's conversations, shared persistent agents, and an out-of-band realtime protocol — `elevenlabs.openapi.json:6`). Voice and model *discovery* becomes documentation and the skill: the offered voice list is a published fact, not an API call.

**DB configuration that makes it safe.** ElevenLabs: body `closed`, `text` `MaxLen`, `model_id` `InSet`; the path `{voice_id}` is bounded either by a `capture_constraints` `InSet` (engine) or — the recommended launch posture — by **curating the account**: the operator's shared ElevenLabs account contains only offerable voices, so any other id fails at the vendor. That is operator-supplied account state, not code (**ASSUMPTION:** whether ElevenLabs accepts stock voice ids outside the account library — verify; the risk that matters, cloned-voice impersonation, is bounded either way since cloned voices exist only if the operator creates them). Twilio (when it ships): body `closed`, `From` `InSet` (operator's approved senders), `To` `Pattern` (E.164), `Body` `MaxLen`, **forbid** `MessagingServiceSid` (sender-set bypass), `StatusCallback` (attacker webhook), `MediaUrl`; `{AccountSid}` fails safe at the vendor for a plain account (**ASSUMPTION:** the credential is a non-master account — operator invariant, since master credentials reach subaccount paths).

**What the operator supplies.** ElevenLabs: API key + the curated voice account + the published voice list. Twilio: credentials (non-master), approved sender numbers, and their own compliance posture — which is precisely why this must not be baked into our code.

**Cannot be expressed in configuration — the honest line.** ElevenLabs: nothing, at launch scope (the per-user default-voice product — server-inserted voice, transcript+audio returned as the user's artifact — is a v2 *product* handler, not a safety prerequisite). Twilio: **the danger lives in `To` and `Body`, the two fields no value-set can bound.** Consent, opt-out/suppression, per-recipient velocity, and *phone ownership* ("text me" requires knowing a number is the requester's — the user model has `email_verified` only, `models/user.rs:115`) are stateful, identity-bound workflow. No DSL expressiveness removes them; they are the irreducible code (§5), and Twilio SMS waits on them.

## 3. Flight search and booking — Duffel

**What a user gets.** "Find me flights, hold the one I pick, show me the bill; I'll pay by link." The agent reads, searches, and prepares; the human does the irreversible step. Hold orders are confirmed available on live offers (`requires_instant_payment: false` measured on real search results).

**Minimal operation set, each justified:**

| Op | Intent served |
|---|---|
| `POST /air/offer_requests` | search (offers return inline in the response — measured, 602 offers) |
| `GET /air/offers/{id}` | re-price the *chosen* offer immediately before holding (vendor-documented requirement; stale prices are rejected) |
| `POST /air/orders` | create the hold — the booking itself, no money moved |

Cut: `GET /air/offers` (list — the create response already embeds the offers; a second enumeration surface serves no intent), `POST /air/payments` and cancellations (human-link territory, not agent operations), `GET /air/orders` and `GET /air/orders/{id}` (order list/read — on a shared credential these are every user's bookings with passenger names; the agent retains the creation response instead, and the payment link surface does its own server-side read).

**DB configuration that makes it safe.** The hold-only guarantee becomes a body constraint instead of documentation: on `POST /air/orders` — `data.type` `Equals("hold")`, **forbid** `data.payments`, body `closed`. Plus the non-overridable `Duffel-Version: v2` header (existing `default_request_headers` machinery) and `direct_only: true`.

**What the operator supplies.** Duffel token (test until go-live), and — for payment — their own Duffel Cards approval.

**Cannot be expressed in configuration:** the payment link surface (the page that renders the bill, embeds Duffel's card form and 3DS, and submits the payment server-side) — a browser flow with a PCI boundary, not request validation. Already designed; code by nature (§5).

---

---

# 4. The DB structure

**The rule shape.** `proxy_operation_policy` on `DownstreamService` grows from method + path template to:

```
ProxyOperationRule {
  method: String,
  path_template: String,                          // shipped today
  capture_constraints: { "<name>": ValueRule },   // path {captures}, e.g. voice_id ∈ set
  body: Option<{
    content_type: "json" | "form",                // Twilio is form-encoded; both first-class
    closed: bool,                                 // reject unknown fields
    require: { "<field.path>": ValueRule },       // nested paths, e.g. data.type
    forbid:  [ "<field.path>" ],
    max_bytes: u64,
  }>,
}
ValueRule = Equals(v) | InSet([v]) | Pattern(regex) | Range{min,max} | MaxLen(n)
```

Row-level flags alongside the policy: `direct_only` (no node routing — Firecrawl, Duffel) and `retriable_methods` (which methods the engine may retry across nodes/fallbacks; default none for POST — the DB knob for the duplication problem in `mcp_service.rs:3664-3736`). Everything above is serde on the existing row, validated at admin write, evaluated in the shared pre-side-effect check on both executors. **The engine ships once; the policy ships per deployment** — that is the open-source contract.

**Is "capability" a first-class DB object? No — argued and decided.** A capability is a *presentation and discovery* grouping: a `capability: Option<String>` tag on the service row (plus the skill that narrates the bundle), used by catalog listing, MCP tool grouping, and docs. Making it a first-class enablement object would create a second source of enforcement truth that can disagree with the row-level one — a row enabled but its capability disabled, or vice versa — and every security property in this document lives on rows (credential, policy, flags). One enforcement surface; capability is how humans and agents *find* it, not how it is *gated*. If per-capability gating is ever wanted, it composes as "set the tag, toggle the rows sharing it" in admin tooling — still one truth.

---

# 5. The irreducible code list — what must be Rust, and why

1. **The constraint engine itself** (the §4 evaluator: form+JSON parsing, nested paths, ValueRules, capture constraints; plus honoring `direct_only`/`retriable_methods`). Ships once, unlocks every capability; all three sections above depend on it except X/Reddit. A more expressive DSL doesn't remove this — it *is* the DSL's implementation.
2. **Twilio consent/opt-out/suppression + phone-ownership verification.** Stateful, cross-request, identity-bound: "this number belongs to the requesting user, consented, not suppressed, under velocity" is workflow over user identity (which today has no phone at all — `models/user.rs:115`), not request validation. No DSL removes it. Until it exists, Twilio SMS stays dark — for every operator, which is honest: we ship the engine and the constraint template; the operator inherits a *disabled-by-default* service whose enablement requires this code plus their own compliance posture.
3. **The Duffel payment link surface.** Browser flow, card form, 3DS, server-side bill read — a PCI-boundary product feature. No DSL applies.
4. **Send-idempotency for real-world writes** (SMS dedupe, and the general no-blind-retry discipline for non-idempotent POSTs beyond what `retriable_methods` expresses). Mostly engine work with the DB knob; the residual (per-message dedupe keys) belongs to the Twilio handler in item 2.
5. **Not on the list, deliberately:** the ElevenLabs voice question (curated account = operator state, not code; the per-user-voice endpoint is an optional product upgrade) and X/Reddit app-only enforcement (an activation invariant an operator satisfies with credentials — though a cheap row-validation warning for OAuth-shaped credentials on platform rows would be a kindness, not a requirement).

**Assumptions register:** ElevenLabs stock-voice acceptance outside the account library; Twilio non-master account topology and built-in STOP-handling coverage; Reddit commercial API terms and the token-exchange-on-platform-credential shape; Firecrawl response usage-headers presence; per-user fair-share on shared rate pools remains an open platform gap (tracked in the enablement plan).

---

# The recommendation

**Build one thing: the constraint engine.** Everything else is configuration, operator state, or a separate product surface.

That is the whole finding. Of the three capabilities, exactly one piece of Rust stands between us and all of them — the evaluator that reads a declarative rule from the database and checks a request against it before any side effect. It ships once. After it lands, Firecrawl, ElevenLabs and Duffel search-and-hold are configuration changes, and so is every service an operator adds later without our involvement.

**The order:**

1. **Ship X and Reddit reads now.** They need no engine, no constraints, no code — the existing method-and-path matcher is sufficient because the data is public. The single condition is an **app-only credential**, verified rather than assumed: our providers default to user OAuth, and a shared user-context token can see protected accounts and private subreddits. Reddit additionally needs its commercial terms checked, because reselling access through a shared credential is the kind of term vendors enforce.

2. **Build the constraint engine**, with `direct_only` and `retriable_methods` as row flags. The second matters more than it looks: it is the database knob that stops the MCP node executor retrying a POST across fallback nodes and billing the vendor twice for one caller result.

3. **Then Firecrawl, ElevenLabs and Duffel become configuration.** Firecrawl gets bounded `limit` and `maxCredits` and `direct_only`; its async polling stays out until something records job ownership. ElevenLabs is write-only TTS against a curated account. Duffel is search and hold, with payment and cancellation as links.

4. **Twilio last.** Not because SMS is hard, but because consent, opt-out, suppression and phone-ownership are stateful workflow over an identity that today has no phone at all. Ship it disabled-by-default with the constraint template ready, and let enablement wait on that code plus each operator's own compliance posture.

5. **The Duffel payment link is its own product surface** — browser flow, card form, 3DS. It is gated on Duffel Cards approval, which is external and worth requesting now, since that clock runs regardless of what we build.

**Why this order is defensible rather than merely cautious:** it front-loads the only capability that needs nothing, then builds the only thing that unlocks everything, then converts the rest to configuration. No step depends on a later one. And the one genuinely irreducible piece of code is the piece that makes the project configurable by people who are not us — which, for an open-source platform, is the point.
