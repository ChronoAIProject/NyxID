# Platform Capability Surface — What We Have, What AsiaOne Is, What to Add

> **Superseded (2026-08-20):** converged into the canonical `PLATFORM_CAPABILITY.md` (single article for the owner, fact-checked against the repo). Kept as working history.

**Owner ask:** what NyxID already provides that can be offered as a **platform capability** (fungible account — the user wants the capability, not the account), what asiaone.com provides, and which additions cover the widest scope, with approach and risk. Identity-bound services (GitHub, Google, Slack, Lark, Microsoft, Discord) are out of scope by decision — those stay BYOK.

**Governing constraints, applied throughout:** agent reads/searches/prepares — the human does anything irreversible via a link; the allowlist is the only boundary on a shared credential (#1436 and #1448 are merged; **nothing is enforced in production yet** — the only policy in `main` is a test fixture); billing is deferred, so access control is the near-term lever, which makes anything with per-call real-world cost or compliance exposure high-risk to offer broadly right now.

Repo claims cite `file:line`/docs on `travel-allowlist` (rebased onto `origin/main` today). Assumptions marked.

## The 30-second table

| Capability | User intent covered | Approach | Risk (money / abuse / compliance / reputation) | Status |
|---|---|---|---|---|
| Web scrape + web search (Firecrawl) | "read this page", "search the web" | existing overlay + platform row + allowlist | low / low / low / low | **ready today** (free-tier plan) |
| LLM inference (OpenAI, Anthropic, Mistral, DeepSeek, Cohere, Google, OpenRouter) | "think", "summarize", "see an image" (multimodal) | **already live** — LLM gateway on platform keys | metered cost / low / low / low | **shipping today** |
| X/Twitter reads + search | "what's being said about…" | existing overlay + platform row, **reads only** | low / rate-pool starvation / low / low | ready today |
| Speech: TTS + transcription (ElevenLabs, OpenAI audio) | "speak", "transcribe this" | existing overlays + platform row; allowlist **excludes voice cloning/dubbing** | quota cost / **impersonation** / med / med | near-ready (needs allowlist + streaming-meter check) |
| Image generation (OpenAI images, Google AI) | "draw/generate an image" | existing overlays + platform row | metered cost / deepfake-abuse / med / med | near-ready |
| Deep research via browser LLM (Oracle relay, platform pool) | "research this thoroughly" (ChatGPT Pro-class) | existing subsystem; pool `visibility: platform` is built in | subscription not per-call / low / **provider-ToS** / med | built; ToS decision needed |
| Reach the user (Telegram/Discord/Lark/WhatsApp bots, notifications) | "message me when…", approvals | existing channel-bot + notification machinery | low / spam-if-misused / low / low | **shipping today** |
| React to events (Triggers) | "when X happens, do Y" | existing triggers subsystem (`nyx_trg_`) | low / low / low / low | **shipping today** |
| Dedicated web-search API (Tavily/Brave-class) | cheaper, better "search the web" | **new** overlay + platform row | low / low / low / low | small addition |
| Document parsing (PDF/OCR) | "read this document" | mostly covered via LLM gateway multimodal; optional parser row | low / low / low / low | mostly covered; small gap |
| Book + pay for things (travel) | "book my holiday" | agent holds via proxy; human pays via link (Duffel Cards) | high / med / **PCI-adjacent, chargebacks** / high | planned, sequenced late |
| Telephony/SMS (Twilio) | "call/text someone" | exists as overlay — **do not offer broadly now** | **per-call money** / high / **TCPA** / high | blocked on compliance + billing |
| Run code | "execute this" | **gap in NyxID** (ecosystem has a sandbox service; could front it as a row) | low / sandbox-escape / low / med | gap — decision |
| Send email | "email someone" | **gap — recommend not offering** as platform | low / **spam** / deliverability / high | deliberately absent |
| Act on the web (browser automation) | "click through this site for me" | gap in NyxID proper (oracle CDP worker is adjacent) | low / **high** / site-ToS / high | not now |

---

## Part 1 — What we already have (the full surface, not just the overlays)

### 1a. The overlay/catalog layer (24 curated OpenAPI overlays, `backend/specs/catalog/`)

Fungibility split of the 24 (`anthropic cohere deepseek discord-bot discord elevenlabs facebook firecrawl github google-ai google lark-bot lark microsoft-graph mistral openai openrouter reddit slack spotify telegram-bot twilio twitch twitter`):

- **Fungible — platform-credential viable:** the seven LLM providers (see 1b — already served on platform keys), **firecrawl** (scrape/search/crawl/map/extract), **elevenlabs** (TTS/STT/voice), **twilio** (SMS/voice — fungible but high-risk), **twitter reads** (app-only bearer). What an agent does with them: read any page, search, synthesize, speak, transcribe — the read-and-prepare half of nearly every task.
- **Identity-bound — out of scope (BYOK stays):** github, google, microsoft-graph, slack, lark(+bot), discord(+bot), facebook, reddit, spotify, twitch, telegram-bot, and twitter *writes* (a platform token posts as ChronoAI — established earlier and unchanged).
- Caveat carried from the platform-services plan: the seeded rows are BYO-connection-shaped and carry `provider_config_id`, which the credential gate rejects by design (`provider_service.rs:2546-2650`) — platform variants are fresh provider-less rows the admin API can create today (`handlers/services.rs:49, 91, 1056-1109`).

### 1b. The LLM gateway — the existing proof of the whole model

`/api/v1/llm` (mounted in the delegated router, `routes.rs:1252`) is an OpenAI-compatible surface over multiple providers on **platform credentials, in production** — it *is* a platform capability service, live, metered by the billing subsystem's platform layer. Every argument for "can this model work" has an existence proof here. It also covers multimodal intents (vision via multimodal chat, image generation via the provider APIs) without new machinery.

### 1c. Oracle relay — a differentiated capability no aggregator has

A logged-in browser LLM tab (ChatGPT Pro etc.) as a callable resource: submit/poll task queue, multi-turn sessions, PDF attach, transcript import (`services/oracle_{pool,task,session}_service.rs`, `docs/ORACLE_RELAY.md`). Pool `visibility` already includes **`platform`** (CLAUDE.md Rule 11) — a platform-operated pool is designed-in, not a hack. User terms: "do deep research with a frontier consumer model," at subscription cost rather than per-token. **The risk is not technical but ToS/compliance:** operating consumer ChatGPT accounts as shared platform capacity is an OpenAI ToS question, and account bans are the failure mode. Decision needed before offering platform pools; org/private pools (users' own accounts) carry that risk themselves.

### 1d. Node proxy and SSH — user-bound by nature

On-prem credential nodes (`docs/NODE_PROXY_ARCHITECTURE.md`) and SSH exec/terminal (`docs/SSH_REMOTE_EXEC.md`, cert + node-key modes) reach *the user's* machines and secrets — inherently identity/premises-bound, so **not** platform capabilities. Their platform relevance: platform-operated nodes could host future capabilities (e.g. a managed browser or sandbox fleet) — infrastructure, not an offering today.

### 1e. Channel bots, triggers, connect links, approvals, MCP — the connective tissue that already ships

- **Reach the user:** channel bots + relay (Telegram/Discord/Lark/WhatsApp-via-OpenClaw, `docs/CHANNEL_BOT_RELAY.md`) and the notification/approval system. Platform-owned bot identity is *correct* here (nobody wants their own approvals bot). Covers "message me / ask me / confirm with me" — the human-in-the-loop half of the governing principle.
- **React to events:** triggers (`nyx_trg_` ingress → agent/notification/webhook delivery) — "when X happens, tell my agent" is a platform capability already in production shape.
- **MCP server + connect links + approvals:** the delivery, onboarding, and consent surfaces every capability above is published through — not capabilities themselves, but they're why adding a capability is configuration rather than a project.

## Part 2 — What asiaone.com is (the caution was warranted)

Fetched live today: **AsiaOne is a Singapore news portal** — title *"AsiaOne, Asia's Leading News Portal"*, meta description *"free access news portal delivers latest breaking news and top stories updates in Singapore, Asia Pacific and across the World"* (SPH Media). It has **no API, developer, or marketplace surface** (page scan: zero marketplace/RapidAPI references, one incidental "developer" string). **It is not a comparator for aggregated API platforms, and the URL was probably not what was meant.** What is genuinely extractable: AsiaOne is a *content source* — the kind of site a user's agent scrapes for Singapore market/news research — i.e. **demand-side evidence for the Firecrawl/search capability**, not a competitor.

The comparison the owner likely wanted — the API-aggregator class:

- **RapidAPI**: a marketplace of thousands of third-party APIs behind one key/billing relationship — breadth via listings, no agent layer, no human-in-the-loop machinery.
- **Composio / Pipedream / Zapier**: managed-auth tool platforms for agents/workflows — strong on identity-bound app connections (the BYOK class we're keeping) and event triggers; thin on platform-credential fungible capabilities and on approval/consent surfaces.
- **NyxID's differentiation against that class** (honest, not marketing): the shared-credential capability rail with a data-plane allowlist, the human-approval/channel machinery, the oracle relay (nobody else has "a ChatGPT Pro tab as an API"), and node reach into user premises. **Their advantage over us:** raw catalog breadth. The widest-scope strategy below closes intent coverage, not vendor-count.

## Part 3 — Widest scope: the intent map, the gaps, and what not to do

**Covered today or near-today** (approach = platform row + allowlist on existing overlays unless noted): read a page / search the web (Firecrawl), think/summarize/see (LLM gateway — live), speak/transcribe (ElevenLabs + OpenAI audio; allowlist must exclude voice cloning/dubbing — impersonation abuse on a shared account), generate images (OpenAI/Google AI; moderate deepfake-abuse risk, model/endpoint allowlist bounds it), social listening (X reads), deep research (oracle platform pool — pending the ToS decision), reach/confirm with the user (channels + approvals — shipping), react to events (triggers — shipping), read documents (LLM multimodal covers most; a dedicated parser row is a small optional add).

**Gaps worth adding, with approach and risk:**

1. **Dedicated web-search API (Tavily/Brave-class).** New overlay + platform row. Cheapest, safest, widest single addition — search is the most common agent intent and a purpose-built search API beats scrape-search on cost/quality. Risk: negligible; allowlist trivially bounds it.
2. **Code execution.** NyxID has nothing; the ChronoAI ecosystem runs a sandbox service (**ASSUMPTION:** its productization status — verify before committing). Approach if wanted: front it as a platform row like any capability. Risk: sandbox escape (technical, bounded by the sandbox not by NyxID), moderate reputation. Decision, not default.
3. **Booking + paying** — the travel plan (hold via proxy, pay via link) is the template for the whole "do something irreversible" class, and it is already specced and sequenced (see `PLATFORM_SERVICES_PLAN.md`). Risk profile unchanged: money, PCI-adjacency, chargebacks, Cards approval gate.

**What we should *not* offer broadly now — blunt list:**

- **Twilio/telephony/SMS**: fungible, yes — but every call spends real money on a shared account with no billing ceiling (billing deferred), from platform-owned sender numbers carrying TCPA/spam exposure. The allowlist bounds *which endpoints*, not *how much money or to whom*. Blocked on the compliance decision + billing hardening, both already tracked.
- **ElevenLabs voice cloning/dubbing**: impersonation abuse attributed to ChronoAI's account; excluded from any allowlist, permanently until a consent story exists.
- **Email sending as a platform**: spam and deliverability ruin are account-wide and slow to repair; the "reach the user" intent is already covered by channels. Deliberately absent, not forgotten.
- **Identity-network writes on platform credentials** (posting as ChronoAI): established, unchanged.
- **Browser automation at platform scale**: high site-ToS/abuse surface, no allowlist analogue (arbitrary web); the oracle CDP worker stays user-scoped.
- **Anything crypto/fiat on-ramp**: out, as decided long ago.

**The widest-coverage move, stated as a recommendation:** ship the free tier already planned (Firecrawl + X reads), add a dedicated search row (small), turn on the audio/image allowlisted rows next (near-ready), and put the **oracle platform pool ToS question** to the owner — it is the single most differentiated capability in the inventory and the only one whose blocker is a decision rather than work. Everything with per-call money or compliance exposure waits for the billing hardening and compliance decisions already on the board (`PLATFORM_SERVICES_PLAN.md` §4–§5, §9).

**Assumptions register:** sandbox-service productization status; ElevenLabs streaming byte-metering (carried); X app-only read scopes (carried); oracle platform-pool operation at scale (worker capacity, account rotation) — unmeasured; aggregator-class characterizations are from general knowledge, not fresh research (**ASSUMPTION** — commission a proper competitive scan if this comparison drives strategy).
