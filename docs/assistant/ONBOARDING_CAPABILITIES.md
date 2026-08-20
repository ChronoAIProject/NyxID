# Onboarding Capabilities — The Smaller, Correct Plan

**Purpose, which decides everything below:** showcase capabilities at onboarding. A brand-new user with no accounts connected gets a genuine aha-moment on a ChronoAI credential — safe operations only, and DB-configurable because NyxID is open source and operators must be able to set this up without forking. Two adversarial reviews (code and product) converged on the same verdict about the previous plan: it solved a bigger problem than the one we have. This is the smaller one. Repo claims cite `file:line` on `travel-allowlist` (rebased onto `origin/main` today); assumptions are marked.

## The finding that gates everything else

**The assistant a new user actually meets cannot call any of this.** Production direct chat is text-only with no tool execution — its own system prompt says so (`services/assistant_direct.rs:14-24`), and its validation allows nothing else (`:176-217`). The one relevant skill it ships teaches the *BYOK* Firecrawl flow and the async agent this plan excludes (`prompts/direct/firecrawl-via-nyxid.md:14-37`). Every catalog row configured below is invisible to a first-run user until the assistant is wired to tools. **Without build item 1, the rest of this document delivers nothing.** It is the first work item, not a footnote.

## Cut, one line each

- **Twilio SMS** — irreducibly needs consent/opt-out/phone-identity code plus a compliance posture; not an onboarding capability.
- **Duffel hold/book/pay** — booking is a different product with its own gates (Cards approval, payment surface); onboarding needs search, at most.
- **Firecrawl `agent` + `map`** — the async pair's polling cannot be made multi-tenant by configuration; `map` serves no first-run intent.
- **The constraint DSL** — both reviewers found its flagship example does not work: `closed` + `require {data.type}` either rejects `passengers`/`selected_offers` (no hold possible) or checks top-level only so `data.payment` (a typo) passes. Twilio and Duffel booking were what the DSL existed for; with both cut, the language has nothing left to justify.
- **`direct_only` / `retriable_methods` flags** — solve problems (node-retry duplication of vendor jobs) that the cut capabilities carried in.
- **Dual-mode rows** — the plumbing I previously claimed exists does not (correction below); not needed to showcase.
- **Per-user access lists** — onboarding rows are public by intent.
- **Billing-metric redesign** — billing is deferred; nothing here charges.

## Build — five items, a few hundred lines total, all in existing seams

1. **Wire the assistant to the tools. (M — the largest and least optional.)** Give production first-run chat a tool-execution path to platform catalog rows — the repo already carries a direct-agent proof-of-concept seam (`services/assistant_direct_agent_poc/`) and the MCP operation machinery; the work is productionizing one path from first-run chat to allowlisted platform operations, plus replacing the BYOK-teaching skill with one that narrates the platform capabilities. Without this there is no showcase; with only this and item 2, X and Reddit already demo.
2. **Fail-closed platform rows. (S.)** Today an absent `proxy_operation_policy` means passthrough — documented in the model itself (`models/downstream_service.rs:351-353`). A row holding a master credential must refuse to execute without a present, non-empty policy. One condition in the authorization path plus tests; grandfathering concerns don't apply because no production row carries both a credential and no policy yet.
3. **Reject user-mode credentials on platform rows — a check, not a warning. (S.)** The seeded `api-twitter`/`api-reddit` rows cannot serve this purpose: they carry `provider_config_id` and `auth_method: "none"` (`services/provider_service.rs:3763-3766, :3823-3826`), failing the master-credential predicate on three counts (`services/proxy_service.rs:244-251`); the X provider's default scopes include `tweet.write` (`:590-595`) and Reddit is user-mode (`:1269-1282`). Platform variants are **new provider-less rows**, and admin validation must reject OAuth-shaped/user-mode credentials on them outright — an operator wiring a personal OAuth token into a shared row is the predictable mistake.
4. **Trim the overlays so discovery matches execution. (S–M.)** The overlays still publish X `POST /tweets` and `DELETE /tweets/{id}`, Reddit `POST /api/submit`, `/users/me`, `/api/v1/me`, and Firecrawl `/v2/agent` — operations the policy will deny — and **startup sync force-reactivates overlay endpoints on every boot** (`services/service_endpoint_service.rs:336`), so an operator cannot even remove them from their own DB. Fix both: cut the excluded operations from the overlays, and make sync respect an operator's deactivation. The overlay is enforcement for *discovery*; the matcher is enforcement for *execution*; an agent that discovers tools it cannot call burns its turns on 404s.
5. **Per-user rate limit on the shared key. (S–M.)** The existing limiter is API-key-scoped and browser sessions bypass it (`mw/rate_limit.rs:462-496`); onboarding callers *are* sessions. A per-user bucket on platform rows, else one enthusiastic session starves the demo for everyone.

## The capability set

| Service | Operations | Notes |
|---|---|---|
| **X** | `GET /tweets/search/recent` — only | "What's being said about…" — the single-op demo. App-only bearer, reads of public data. |
| **Reddit** | `GET /r/{subreddit}/hot`, `/new`, `GET /search`, `GET /comments/{article}` | **Conditional:** app-only tokens expire (~1 h) and need the `token_exchange` auth method, which auto-provision currently excludes (`services/unified_key_service.rs:193`) — lift that exclusion for platform rows or do not ship Reddit. Commercial terms remain unverified (**ASSUMPTION** — check before activation). |
| **Firecrawl** | `POST /v2/scrape`, `POST /v2/search` — only, with a request body-size cap and a `maxCredits` ceiling | Kept: "read me this page" is the strongest first-run demo we have. **Dissent, represented honestly:** one reviewer would cut it — per-page cost on the platform key with no billing means every demo scrape is unrecovered spend. The keep is a deliberate marketing cost, bounded by the vendor plan, the ceilings, and item 5 — not a free lunch. Note: `Pattern("^https?://")` is **not** an SSRF control — it happily permits `http://169.254.169.254/`; the fetch happens at Firecrawl's infrastructure, not ours, which is the actual mitigation, and internal-address filtering is Firecrawl's responsibility (**ASSUMPTION** that their fetcher does so — do not represent the URL pattern as a security boundary). |
| **Duffel** | `POST /air/offer_requests`, `GET /air/offers/{id}` — search only, **no orders** | Optional. "Find me flights" with real prices is a strong demo; everything irreversible is out. |
| **ElevenLabs** | `POST /v1/text-to-speech/{voice_id}` — one stock voice, hard character cap | Optional and tiny. Honest alternative: browser `speechSynthesis` is a cheaper aha if the point is delight — a product call, not an engineering one. |

All rows: new, provider-less, `internal`, public, explicit policy (deny-by-default; the GET exclusions that made X/Reddit/ElevenLabs safe are simply absent rules), non-overridable headers where the vendor requires them.

## What an operator must supply

Per service they choose to enable: the credential (X app-only bearer; Reddit app client via `token_exchange`; Firecrawl key; Duffel test token; ElevenLabs key on an account containing **only** the voices they are willing to offer — curation is the voice control), the policy rows above (shipped as seed defaults they can edit), and the rate-limit setting. Nothing in this plan bakes our policy choices into code they would have to fork: the capability set is rows, policies, and skill text — all DB or assets.

## Corrections carried so this document does not contradict its lineage

- **Dual-mode plumbing does not exist.** I previously wrote that resolution prefers a user's row and falls back to the platform's. Misread: those paths are UserService versus *legacy BYOK* (`handlers/llm_gateway.rs:274-280`, `services/proxy_service.rs:1277`), not user-versus-platform. Platform resolution is a separate admin path.
- **Scoped agent API keys still cannot call platform rows** (`handlers/mcp_transport.rs:642-660`; `handlers/proxy.rs:1958-1978`). Under the onboarding frame the first-run caller is a session user, so this is an **availability gap, not a blocker** — but it stays true, and the moment the showcase extends to user-configured agents it becomes work item 6.
- The constraint-DSL cut is not a verdict that parameter validation is worthless — it is that the two capabilities requiring it are out of scope, and a validation language should be built when its first real user exists, against that user's actual semantics (`closed` interacting with required fields was the failure).

## If travel comes back later

Do not generalise from it. Build a NyxID-owned `POST /travel/holds` endpoint that constructs the Duffel order body server-side — hold-only by construction, no caller-supplied fields. That is less code than correct `closed`-body semantics in a generic DSL, and it does not become every operator's configuration problem.

**Assumptions register:** Reddit commercial API terms; Firecrawl-side internal-address filtering; ElevenLabs stock-voice acceptance outside the account library (moot under account curation, noted for completeness); the direct-agent PoC seam's fitness as the production wiring path (item 1's size rides on it — re-estimate if it proves demo-grade only).
