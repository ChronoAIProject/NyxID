# Capability Contracts — Three Capabilities, Configured in the Database

> **Superseded (2026-08-21):** replaced by `ONBOARDING_CAPABILITIES.md` — the converged onboarding-scoped plan after two adversarial reviews. Kept as working history.

**Owner framing:** three capabilities, defined by what a user wants — **social listening and research** (Reddit, X, Firecrawl), **calling and getting things done over the phone** (Twilio + ElevenLabs), **flight search and booking** (Duffel). Two hard constraints govern everything here: **no irrelevant APIs** — every exposed operation must be justified by the user intent it serves, or cut — and **NyxID is open source, so the structure must be DB-configurable**: a downstream operator running their own NyxID must be able to express these policies without forking our code. That second constraint inverts an earlier conclusion in this document's history: purpose-built handlers are code, and code is *our* policy imposed on every operator. The design goal is now **maximise what lives in the database, and be explicit about the irreducible minimum that must be code.**

Standing facts, carried not rediscovered: the shipped matcher checks method + path segments only (`services/proxy_authorization.rs:186`); nothing is enforced in production yet; every leak found in Twilio and ElevenLabs is a GET, so **write-only is the shape, not a mitigation** — deny-by-default means omitting a rule *is* the block, at zero cost; X and Reddit reads are safe **because the data is public, not because they are reads** — and only with app-only credentials, which is an activation invariant because the repo's providers default to user OAuth (X: `services/provider_service.rs:574-592`, user scopes including `tweet.write`; Reddit: `:1267-1271`). Repo and overlay claims cite `file:line`; assumptions marked.

**Verdicts at a glance:**

| Capability | Ships with configuration alone | Waits, and on what |
|---|---|---|
| Social listening & research | X reads + Reddit reads **now** (method+path policies suffice; app-only credentials as invariant) | Firecrawl: the constraint engine (bounded bodies) + direct-only routing; its async pair excluded outright |
| Phone | ElevenLabs TTS **once the engine lands** (write-only + curated account) | Twilio SMS: irreducible code (consent/opt-out/phone identity) + compliance decision; voice calls: out entirely |
| Flights | Duffel search + hold **once the engine lands** (hold-only body constraint) | Payment: the link surface (code, planned) + Duffel Cards approval (external) |

---

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

## 4. The DB structure

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

## 5. The irreducible code list — what must be Rust, and why

1. **The constraint engine itself** (the §4 evaluator: form+JSON parsing, nested paths, ValueRules, capture constraints; plus honoring `direct_only`/`retriable_methods`). Ships once, unlocks every capability; all three sections above depend on it except X/Reddit. A more expressive DSL doesn't remove this — it *is* the DSL's implementation.
2. **Twilio consent/opt-out/suppression + phone-ownership verification.** Stateful, cross-request, identity-bound: "this number belongs to the requesting user, consented, not suppressed, under velocity" is workflow over user identity (which today has no phone at all — `models/user.rs:115`), not request validation. No DSL removes it. Until it exists, Twilio SMS stays dark — for every operator, which is honest: we ship the engine and the constraint template; the operator inherits a *disabled-by-default* service whose enablement requires this code plus their own compliance posture.
3. **The Duffel payment link surface.** Browser flow, card form, 3DS, server-side bill read — a PCI-boundary product feature. No DSL applies.
4. **Send-idempotency for real-world writes** (SMS dedupe, and the general no-blind-retry discipline for non-idempotent POSTs beyond what `retriable_methods` expresses). Mostly engine work with the DB knob; the residual (per-message dedupe keys) belongs to the Twilio handler in item 2.
5. **Not on the list, deliberately:** the ElevenLabs voice question (curated account = operator state, not code; the per-user-voice endpoint is an optional product upgrade) and X/Reddit app-only enforcement (an activation invariant an operator satisfies with credentials — though a cheap row-validation warning for OAuth-shaped credentials on platform rows would be a kindness, not a requirement).

**Assumptions register:** ElevenLabs stock-voice acceptance outside the account library; Twilio non-master account topology and built-in STOP-handling coverage; Reddit commercial API terms and the token-exchange-on-platform-credential shape; Firecrawl response usage-headers presence; per-user fair-share on shared rate pools remains an open platform gap (tracked in the enablement plan).
