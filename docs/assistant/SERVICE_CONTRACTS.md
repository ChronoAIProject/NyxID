# Service Contracts — Five Platform Surfaces, In Depth

**Scope (owner):** read Firecrawl, read Twitter, read Reddit, Twilio, ElevenLabs. Duffel is deliberately absent. Billing is deferred — this document is about **access control and operation safety** on a shared platform credential: what each service gives a user, exactly which operations to expose from the existing overlays in `backend/specs/catalog/`, the field-level constraints that make them safe, **what the shared account leaks**, and an honest verdict per service. Written for a reader deciding, not implementing.

Two standing facts frame everything: the shipped matcher checks method + path segments only (`services/proxy_authorization.rs:186-203`) — parameter constraints do not exist yet and are assumed as the configuration mechanism already designed in `CATALOGUE_ENABLEMENT_PLAN.md`; and every operation below runs as *ChronoAI's own vendor account*, so "what does a caller learn about the shared account or about other users" is asked explicitly for each service. One design decision (owner) reshapes two of the five: **expose no reads at all where the reads are what leaks** — the policy is deny-by-default, so omitting a rule *is* the block, at zero enforcement cost. Repo and overlay claims cite `file:line`; assumptions are marked.

**Verdicts at a glance:** Firecrawl **offer now** (sync ops; async agent with caveats) · Twitter reads **offer now** · Reddit reads **offer with constraints** (terms check) · ElevenLabs **offer now, write-only** (TTS only; every GET and all conversational AI excluded) · Twilio **not yet** (abuse and spend remain after write-only; purpose-built handler + compliance decision — but a materially simpler one than before).

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

**Risk and verdict.** Lowest-risk surface in this document — because these are reads of **public data**: what they return exists on the open internet regardless of whose credential fetched it. **Offer now.** Configuration case.

## 3. Reddit reads — "monitor the conversation"

**What a user gets.** "Watch r/singaporefi for anything about my product", "what does Reddit think of X", thread retrieval for research — community sensing.

**Operations** (overlay: `backend/specs/catalog/reddit.openapi.json` — seven operations):

- **Expose:** `GET /r/{subreddit}/hot`, `GET /r/{subreddit}/new`, `GET /r/{subreddit}/about`, `GET /search`, `GET /comments/{article}`.
- **Exclude:** `POST /api/submit` — posting as ChronoAI's account: reputational and a ban vector. `GET /api/v1/me` — the platform account's own identity, same reasoning as X.

**Parameter constraints.** None needed structurally (all reads of public data). Optional: bound `limit` parameters.

**What the shared account leaks.** With `/api/v1/me` excluded: the shared surface is the account's rate limit and its ban exposure — Reddit bans accounts, and one caller's abusive query pattern (scraping at volume) risks the account every other user depends on. No cross-tenant data endpoint exists in the exposed set.

**Risk and verdict.** Two things stand between this and "offer now", neither technical: **Reddit's commercial API terms** for exactly this use (reselling access through a shared credential) are unverified — **ASSUMPTION, check before activation, this is the kind of term vendors enforce** — and the platform-credential auth shape (app-only OAuth via the declarative `token_exchange` method) has no production precedent (**ASSUMPTION**, carried from the enablement plan). **Offer with constraints:** technically ready as pure configuration; commercially gated on the terms check.

## 4. Twilio — "text someone for me" (write-only)

**What a user gets.** The agent reaches the physical world: confirm a reservation by SMS, notify a phone number. Voice calls are a later chapter (below). Highest-value, highest-risk intent in the set.

**The write-only reshaping (owner decision).** Every disclosure leak in this service was a GET: account-wide message listings, recording listings, cross-user phone numbers — all reads of **shared private state** with no tenant partition. So the reads are not constrained; they are **not exposed at all**. Deny-by-default means their absence from the policy is the entire enforcement, at zero cost.

**Operations** (overlay: `backend/specs/catalog/twilio.openapi.json` — eight operations):

- **Candidate to expose:** `POST /2010-04-01/Accounts/{AccountSid}/Messages.json` — the *only* candidate.
- **Not exposed — all six GETs:** `Messages.json`, `Messages/{Sid}.json`, `Recordings.json`, `Recordings/{Sid}.json`, `Calls.json`, `Calls/{Sid}.json`. With them gone, the disclosure problem is gone — there is nothing shared left to read.
- **Excluded:** `POST .../Calls.json` — call creation takes `Url`/`Twiml`, handing the caller the call's **control flow**; write-only does not touch that problem. Voice needs a NyxID-owned TwiML endpoint before it is thinkable.

**What is actually left, re-derived.** With reads gone, the residual is **abuse and spend, not disclosure** — a narrower problem than before, and worth stating precisely (body fields verified from the overlay: `Body, From, MediaUrl, MessagingServiceSid, StatusCallback, To`, form-encoded):

- **`To` is an unconstrained recipient** — the spam vector *is* the destination, and no value-set enumerates legitimate recipients.
- **`Body` is free text** — content is unboundable by constraints.
- **`MessagingServiceSid` selects a sender outside any approved `From` set** — must be forbidden; `StatusCallback` (attacker webhook) and `MediaUrl` (attacker media under our identity) likewise.
- **`{AccountSid}` is caller-supplied in the path.** Mostly fail-safe: Twilio authenticates the credential against the account, so a mismatched SID dies at the vendor — *unless* the platform credential is a master account with subaccounts, where master auth can reach subaccount paths. Operational rule: the platform credential must be a plain (non-master) account (**ASSUMPTION** on the deployment's Twilio account topology — verify at setup).
- **Per-segment cost** on every accepted message — spend, unbounded until billing returns; velocity caps are the interim control.

**What the shared account leaks (post-reshaping).** The sender numbers (learnable by receiving one SMS) and delivery behavior. Nothing else — the leak surface was the reads.

**Risk and verdict — is the handler still required? Yes, but a much simpler one.** Write-only killed the disclosure problem; it does not touch abuse and spend, because those live in `To` and `Body` — precisely the fields configuration cannot bound. What changed is the handler's size: no read-API wrapping, no TwiML surface, one operation. Its shape: server-owned `AccountSid` and sender, **server-constructed body** (no caller-supplied form fields at all), recipient bound to the requesting user — which surfaces a real prerequisite: **NyxID has no phone-verified identity today** (`models/user.rs:115` — `email_verified` only; no phone field), so "text *me*" requires adding phone verification (field + OTT flow, itself sent via this same Twilio account) — plus the consent/opt-out lifecycle (Twilio's built-in STOP handling covers part — **ASSUMPTION**, verify coverage for the sender type chosen), velocity caps, and send-idempotency (a resend dedupe, since a duplicate SMS is a real-world annoyance and a real cost). **Not yet** — but the distance shrank: the compliance decision plus a small "notify the requesting user" handler, rather than a Twilio-shaped subsystem.

## 5. ElevenLabs — "give the agent a voice" (write-only)

**What a user gets.** Text becomes speech: voice replies, narrated summaries, audio content. The response *is* the artifact — audio returns directly to the caller (`audio/mpeg`), lands in their chat or workflow like any other output, and NyxID stores nothing.

**The write-only reshaping (owner decision).** As with Twilio, every leak was a GET: `/v1/voices` exposes the account's voice inventory (including any private or cloned voices), and the entire convai family exposes shared-account state — `GET /v1/convai/conversations` lists **every conversation on the shared account** (a direct cross-tenant read), agents are persistent shared resources, and the signed-URL handoff opens a realtime WebSocket whose frames are out of band by the overlay's own note (*"Realtime WebSocket frame protocols … pass through NyxID separately"*, `elevenlabs.openapi.json:6`). None of that is constrained; **none of it is exposed**.

**Operations** (overlay: `backend/specs/catalog/elevenlabs.openapi.json` — ten operations):

- **Expose:** `POST /v1/text-to-speech/{voice_id}` and `POST /v1/text-to-speech/{voice_id}/stream`. The `/stream` decision, made explicitly: it is plain HTTP chunked audio out — the same operation with lower latency, **not** the realtime frame protocol — and the response-is-the-artifact property holds identically. Exposed.
- **Not exposed — every GET:** `/v1/voices`, `/v1/models`, and all six convai operations. Voice/model *discovery* moves to documentation and the skill (the offered voice list is a curated, published fact, not an API call).

**The `voice_id` question — three options, assessed.** It sits in the path, and the matcher cannot constrain capture values today.

- **(a) Curate the account** — the shared ElevenLabs account contains only voices we are willing to offer; any other `voice_id` a caller supplies fails at the vendor. Zero code; enforcement becomes operational discipline (a documented runbook rule — *no cloned or private voices on the platform account, ever* — plus a periodic audit of the account's voice list). One vendor nuance to pin down: whether ElevenLabs accepts stock/premade voice ids regardless of account library (**ASSUMPTION** — verify at activation). If it does, curation still bounds what matters: the impersonation risk lives in *cloned* voices, which exist only if we create them; arbitrary stock voices are vendor-sanctioned and harmless. **Recommended — the owner's working favourite holds.**
- **(b) Server-constructed path** — the caller never supplies a voice; NyxID inserts the user's configured voice. No path-rewrite capability exists on catalog rows today (the `path` auth method injects *credentials* into paths, `models/downstream_service.rs:177-178` — not per-user config), so (b) means a small purpose-built handler in the `chrono-llm-public` mould. That is also the natural home for the owner's product framing: **a per-user assistant voice** — a `voice_id` preference on user settings, a `speak` endpoint that reads it, calls TTS on the user's behalf, and returns the audio *with the input text as its transcript* (the transcript is not composed by NyxID and not stored by NyxID — the caller supplied the text; the endpoint returns both together as the user's own artifact). Right shape for v2; not needed to launch.
- **(c) Path-capture constraints** — requires the parameter-constraint mechanism, which is designed but not built. Correct eventually; not the launch path.

**Parameter constraints (launch shape, under (a)).** Body: `text` length-bounded, `model_id` from the published set, `closed` otherwise (fields verified: `text, model_id, voice_settings, language_code, seed`). The voice identifier also appears as a **nested body field** in the (excluded) convai family — recorded so a future convai enablement doesn't silently lose the restriction.

**What the shared account leaks (post-reshaping).** Subscription quota — shared burn — and nothing else. The inventory, conversations, and agents were all behind GETs that no longer exist here.

**Risk and verdict.** **Offer now, write-only:** TTS with a curated account, bounded text, published voice list. The residual is quota burn (velocity caps interim, billing later) and the content of synthesized speech — which, like SMS bodies, no constraint polices; unlike SMS, the artifact goes back to the requester rather than out into the world, which is why this is offerable and Twilio is not. Configuration case at launch; the (b) handler is the v2 product upgrade, not a safety prerequisite.

---

## The framing question: purpose-built handlers vs configuration

`chrono-llm-public` is safe because a hand-written handler validates everything against fixed allowlists for one upstream (`services/assistant_direct.rs:176-217`). The five services sort against the working hypothesis with two refinements:

- **Configuration cases: Firecrawl, Twitter reads, Reddit reads** — and the reason must be stated correctly: they are safe because their reads touch **public data**. Twilio's GETs were dangerous precisely *because* they were reads — of shared private state. **"Reads are safe" is the wrong rule and would burn us on the next service; the axis is whose data an operation touches, not its HTTP verb.** *(Corrects the earlier framing.)*
- **Purpose-built handler: Twilio** — still, but write-only shrank it: the danger that remains lives in `To` and `Body`, exactly the fields a value-set cannot bound, so a handler that *changes the shape* (verified recipient = the requesting user, server-built body) is the path — now a small notify-me handler plus the compliance decision, not a subsystem. *(Agrees, narrowed.)*
- **ElevenLabs: configuration — by exclusion, not constraint.** Write-only plus a curated account needs no handler at all at launch; the per-user-voice handler is a product upgrade. The earlier statement sharpens: the scoping decision *is* the mechanism decision, and here scoping alone crossed the bar. *(Upgraded by the write-only reshaping.)*

The two rules worth carrying forward: **configuration bounds structure; handlers change shape** — and **write-only is not a mitigation but a shape**: where every leak is a read of shared private state, not exposing reads removes the problem rather than defending it. Four of the five surfaces can now switch on (Firecrawl, Twitter reads, Reddit pending its terms check, ElevenLabs TTS); Twilio alone stays dark, for reasons that are now purely about abuse and spend rather than disclosure.

**Assumptions register:** Firecrawl response usage-headers presence; Reddit commercial terms and the token-exchange-on-platform-credential shape; ElevenLabs stock-voice acceptance outside the account library; Twilio account topology (non-master account) and built-in STOP-handling coverage; the exact nested body path of the convai voice id; per-user fair-share on shared rate pools remains an open platform gap (tracked in the enablement plan, not solved here).
