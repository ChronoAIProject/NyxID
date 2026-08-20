# Service Contracts — Five Platform Surfaces, In Depth

**Scope (owner):** read Firecrawl, read Twitter, read Reddit, Twilio, ElevenLabs. Duffel is deliberately absent. Billing is deferred — this document is about **access control and operation safety** on a shared platform credential: what each service gives a user, exactly which operations to expose from the existing overlays in `backend/specs/catalog/`, the field-level constraints that make them safe, **what the shared account leaks**, and an honest verdict per service. Written for a reader deciding, not implementing.

Two standing facts frame everything: the shipped matcher checks method + path segments only (`services/proxy_authorization.rs:186-203`) — parameter constraints do not exist yet and are assumed as the configuration mechanism already designed in `CATALOGUE_ENABLEMENT_PLAN.md`; and every operation below runs as *ChronoAI's own vendor account*, so "what does a caller learn about the shared account or about other users" is asked explicitly for each service. Repo and overlay claims cite `file:line`; assumptions are marked.

**Verdicts at a glance:** Firecrawl **offer now** (sync ops; async agent with caveats) · Twitter reads **offer now** · Reddit reads **offer with constraints** (terms check) · ElevenLabs **offer with constraints** (TTS family only; conversational AI excluded) · Twilio **not yet** (purpose-built handler + compliance decision).

---

## 1. Firecrawl — "read the web for me"

**What a user gets.** Their agent can read any public page as clean markdown, search the web, and map a site's structure — the ingredient in "research this market", "summarize what this company does", "find who's writing about X". This is the broadest single enabler of agent usefulness in the catalog.

**Operations** (overlay: `backend/specs/catalog/firecrawl.openapi.json` — five operations):

- **Expose:** `POST /v2/scrape`, `POST /v2/search`, `POST /v2/map` — synchronous, one request one result, safe to retry.
- **Expose with caveats:** `POST /v2/agent` + `GET /v2/agent/{id}` (async submit + poll). Two caveats, both verified: **(a)** submission is not idempotent under the MCP node path — node failover re-sends the same request with a *fresh* request id per node (`services/mcp_service.rs:3664-3680`), so a submit that reached node A but failed to report can run twice. Mitigation: route the Firecrawl row direct-only (no node binding), which also matches its nature — there is nothing on-prem about it. **(b)** the poll is bearer-by-job-id (below).

**Parameter constraints.** `url` fields: require `^https?://` (nothing else — Firecrawl does the fetching, but the pattern keeps obviously malformed schemes out); numeric bounds on result limits (`limit`, crawl depth-class fields) so one call cannot request unbounded work; `closed` bodies. Modest, because the operations are inherently read-shaped.

**What the shared account leaks.** Async job ids are account-scoped at Firecrawl: `GET /v2/agent/{id}` returns *any* job on the platform account, so caller A holding caller B's job id reads B's results — which are the pages B was researching, i.e. B's interests. There is no tenant field to constrain; the ids are vendor-generated and unguessable. This is bearer-id access, the same accepted-risk class as elsewhere in the platform design — but here it is **stated**: research topics are sensitive, and if that is unacceptable, the async pair comes out and only the synchronous three ship. No identity endpoint exists in the overlay; account-level usage/quota headers in responses are an **ASSUMPTION** to check and strip if present.

**Risk and verdict.** Worst realistic outcome: quota burn on the platform key and scraping-of-objectionable-content attributed to our account — both bounded by the vendor plan and the allowlist. **Offer now** (`scrape`/`search`/`map` immediately; `agent` direct-routed, with the bearer-poll risk either accepted in writing or the pair deferred). Configuration case — no handler needed.

## 2. Twitter/X reads — "what is being said"

**What a user gets.** Social listening: "what's being said about my product", "pull this person's recent posts", brand and topic monitoring — the sensing half of marketing and research agents.

**Operations** (overlay: `backend/specs/catalog/twitter.openapi.json` — six operations):

- **Expose:** `GET /tweets/search/recent`, `GET /users/by/username/{username}`, `GET /users/{id}/tweets`.
- **Exclude:** `POST /tweets` and `DELETE /tweets/{id}` — writes publish and destroy *as ChronoAI's account*; permanently out on a shared credential. `GET /users/me` — returns the **platform's own identity** (account id, handle); a caller learns which X account ChronoAI operates, inviting targeted reports/bans. Nothing a legitimate user needs.

**Parameter constraints.** None required beyond the method+path layer — the three reads take queries and ids whose worst case is a noisy search. Optional hygiene: bound `max_results`.

**What the shared account leaks.** With `/users/me` excluded: essentially nothing identity-shaped. The real shared-account effect is the **rate pool** — X app-level limits are one bucket for all users, so one heavy caller starves everyone (fair-share is a known platform gap, tracked, not solved here). Search queries are request-scoped; no cross-tenant data exists to read.

**Risk and verdict.** Lowest-risk surface in this document. **Offer now.** Configuration case.

## 3. Reddit reads — "monitor the conversation"

**What a user gets.** "Watch r/singaporefi for anything about my product", "what does Reddit think of X", thread retrieval for research — community sensing.

**Operations** (overlay: `backend/specs/catalog/reddit.openapi.json` — seven operations):

- **Expose:** `GET /r/{subreddit}/hot`, `GET /r/{subreddit}/new`, `GET /r/{subreddit}/about`, `GET /search`, `GET /comments/{article}`.
- **Exclude:** `POST /api/submit` — posting as ChronoAI's account: reputational and a ban vector. `GET /api/v1/me` — the platform account's own identity, same reasoning as X.

**Parameter constraints.** None needed structurally (all reads). Optional: bound `limit` parameters.

**What the shared account leaks.** With `/api/v1/me` excluded: the shared surface is the account's rate limit and its ban exposure — Reddit bans accounts, and one caller's abusive query pattern (scraping at volume) risks the account every other user depends on. No cross-tenant data endpoint exists in the exposed set.

**Risk and verdict.** Two things stand between this and "offer now", neither technical: **Reddit's commercial API terms** for exactly this use (reselling access through a shared credential) are unverified — **ASSUMPTION, check before activation, this is the kind of term vendors enforce** — and the platform-credential auth shape (app-only OAuth via the declarative `token_exchange` method) has no production precedent (**ASSUMPTION**, carried from the enablement plan). **Offer with constraints:** technically ready as pure configuration; commercially gated on the terms check.

## 4. Twilio — "call or text someone for me"

**What a user gets.** The agent reaches the physical world: confirm a reservation by SMS, send a notification to a phone number, eventually place a call. This is the highest-value and highest-risk intent in the set.

**Operations** (overlay: `backend/specs/catalog/twilio.openapi.json` — eight operations):

- **Candidate to expose:** `POST /2010-04-01/Accounts/{AccountSid}/Messages.json` — and, per the analysis below, *not yet even this*.
- **Exclude, permanently on a shared account:** every GET — `Messages.json`, `Messages/{Sid}.json`, `Recordings.json`, `Recordings/{Sid}.json`, `Calls.json`, `Calls/{Sid}.json`. **Twilio resources are account-scoped with no tenant partition:** the list endpoints return *every user's* phone numbers, message metadata, and call recordings; even single-resource GETs are account-wide addressable by Sid. One caller reading another's recordings is not a risk, it is the documented behavior of the API on a shared account.
- **Exclude for now:** `POST .../Calls.json` — call creation takes `Url`/`Twiml` (caller-controlled webhook or arbitrary call script): the call's *content and control flow* come from the caller, which no field constraint meaningfully bounds. Voice needs a NyxID-owned TwiML endpoint (purpose-built) before it is thinkable.

**Why constraints are insufficient for Messages — the concrete schema.** The overlay's message-create body permits exactly `Body, From, MediaUrl, MessagingServiceSid, StatusCallback, To` (form-encoded; verified from the overlay). Pinning `From ∈` approved senders is necessary but nowhere near sufficient: **`MessagingServiceSid` selects a sender pool outside any `From` constraint** (must be forbidden), `StatusCallback` points Twilio's delivery webhooks at an attacker URL (forbidden), `MediaUrl` sends attacker-hosted media under our identity (forbidden), **`To` is an unconstrained recipient** — the spam vector *is* the destination list, and no value-set can enumerate legitimate recipients — and `Body` is free text that constraints cannot police. What remains after maximal configuration is "send arbitrary text to arbitrary numbers from ChronoAI's identity, rate-limited" — which is a TCPA/spam liability decision, not a bounded capability.

**What the shared account leaks.** Everything, absent the GET exclusions (above). With them: the sender numbers themselves (learnable by receiving one SMS) and delivery behavior.

**Risk and verdict.** **Not yet — and not as configuration.** This is the clearest purpose-built-handler case in the set: a handler can do what config cannot — recipient verification (e.g. only numbers the *receiving user* has verified through NyxID, turning "text anyone" into "text me"), server-constructed request bodies (no caller-supplied fields at all), content policy hooks, per-user velocity. Offering Twilio = building that handler *plus* the standing compliance decision on sender identity. Until both exist, Twilio stays dark.

## 5. ElevenLabs — "give the agent a voice"

**What a user gets.** Text becomes speech: voice replies, narrated summaries, audio content — plus voice/model discovery to pick a voice.

**Operations** (overlay: `backend/specs/catalog/elevenlabs.openapi.json` — ten operations):

- **Expose:** `POST /v1/text-to-speech/{voice_id}`, `POST /v1/text-to-speech/{voice_id}/stream` (plain HTTP chunked audio out — not the realtime protocol), `GET /v1/voices`, `GET /v1/models`.
- **Exclude, the whole conversational-AI family:** `POST /v1/convai/agents/create`, `GET /v1/convai/agents`, `GET /v1/convai/agents/{agent_id}`, `GET /v1/convai/conversation/get-signed-url`, `GET /v1/convai/conversations`, `GET /v1/convai/conversations/{conversation_id}`. Three independent reasons: **(a)** `GET /v1/convai/conversations` lists **every conversation on the shared account** — a direct cross-tenant read of other users' voice conversations (the Twilio-GET problem again, found here by reading the overlay rather than the brief); **(b)** agents are *persistent shared-account resources* — one user's created agent is visible to and addressable by all; **(c)** the signed-URL handoff opens the realtime WebSocket, whose frame protocol is out of band by the overlay's own statement — *"Realtime WebSocket frame protocols are documented by ElevenLabs and pass through NyxID separately"* (`elevenlabs.openapi.json:6`) — so frame content cannot currently be policed. Realtime stays excluded until a frame policy exists.

**Parameter constraints — and the two-forms requirement.** TTS: `{voice_id}` **path capture** ∈ a curated platform voice set; body `model_id ∈` allowed models, `text` length-bounded, `closed` otherwise (body fields verified: `text, model_id, voice_settings, language_code, seed`). The voice identifier also appears as a **nested body field** in the convai family (inside `conversation_config` on agent create) — excluded today, but the constraint mechanism must support both the path-capture form and the body-path form, or a future convai enablement silently loses the voice restriction. Voice cloning/dubbing endpoints are absent from the overlay entirely — keep them absent; the curated-voice-set constraint is the impersonation control.

**What the shared account leaks.** With convai excluded: subscription quota (shared burn) and the voice catalog. With convai *included*, other users' conversations and agents — which is exactly why it is excluded.

**Risk and verdict.** **Offer with constraints:** the TTS family under voice/model/length constraints is a well-bounded capability; quota burn is the residual. The hybrid question's honest answer for this service: **configuration suffices for what is being offered**, precisely *because* the risky half (realtime, convai) is excluded rather than constrained. If realtime conversational AI ever comes in scope, that is a purpose-built handler (frame policy, session brokering) — the service genuinely sits between, and the line runs through streaming.

---

## The framing question: purpose-built handlers vs configuration

`chrono-llm-public` is safe because a hand-written handler validates everything against fixed allowlists for one upstream (`services/assistant_direct.rs:176-217`). The five services sort cleanly against the working hypothesis, which survives with one refinement:

- **Configuration cases: Firecrawl, Twitter reads, Reddit reads** — read-shaped, no money movement, constraints modest to none. Building handlers for these would be ceremony. *(Agrees with the hypothesis.)*
- **Purpose-built handler: Twilio** — the schema analysis shows why config maxes out below the safety bar: the danger lives in `To` and `Body`, which are exactly the fields a value-set cannot bound. A handler changes the *shape* of the capability (verified recipients, server-built bodies) rather than filtering its inputs. *(Agrees.)*
- **ElevenLabs: configuration — because of scoping, not despite it.** The hypothesis said "between, depending on streaming"; the sharper statement is that the service is a configuration case **if and only if** the convai/realtime family stays excluded. Include streaming conversations and it becomes a handler case (frame policy, per-session brokering). The scoping decision *is* the mechanism decision. *(Refines the hypothesis.)*

The general rule this yields, worth carrying forward: **configuration bounds structure; handlers change shape.** When safety requires changing what the operation *is* (verified recipients, server-owned bodies, brokered sessions), configuration is the wrong tool no matter how expressive it gets.

**Assumptions register:** Firecrawl response usage-headers presence; Reddit commercial terms and the token-exchange-on-platform-credential shape; the exact nested body path of the convai voice id; per-user fair-share on shared rate pools remains an open platform gap (tracked in the enablement plan, not solved here).
