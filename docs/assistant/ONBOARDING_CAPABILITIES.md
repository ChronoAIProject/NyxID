# What a New User Should Be Able to Do on Day One

**NyxID platform capabilities — the plan, in plain terms**
*2026-08-21. Written after two independent adversarial reviews, both of which concluded the previous plan was solving a bigger problem than we have. This replaces it.*

---

## What we're trying to do

Someone signs up for NyxID. They have connected nothing — no API keys, no accounts, no setup. Within a minute, they should be able to ask for something useful and get it.

"What are people saying about our competitor this week." "Read this page and summarise it." "Find me flights to Tokyo in October."

They can do that because **we** hold the credentials for a handful of services and lend the capability out, safely bounded. That is the whole idea. It is also, separately, how we eventually make money — but that part is deferred and this plan does not depend on it.

## The problem nobody noticed until now

**The assistant a new user actually talks to cannot use any of these services.**

Our production chat is text-only. It has no ability to call tools at all. So we could wire up every service described below, configure them perfectly, and a new user's experience would be exactly what it is today: a model that talks back.

This is the single most important thing in this document. Everything else is worthless without it. It is also not hard — it is wiring that already exists elsewhere in the codebase — but it has to come first, and it has to be understood as the actual product work rather than a supporting task.

## What we're building

Five things. All of them are small, and all of them fit into code that already exists. Together this is a few hundred lines, not a subsystem.

**1. Let the assistant use tools.** Without this, nothing else is visible to a new user.

**2. Make platform services refuse to run without a policy.** Today, a service with no operation rules configured allows *everything*. That is backwards. A service holding our shared credential should refuse to do anything until someone has explicitly said what it may do.

**3. Check the credential is the right kind.** We want these services running on app-only credentials — the sort that read public data and cannot act as any particular person. Right now nothing stops an operator pasting in a personal account token instead, which would quietly turn "search public posts" into "act as whoever owns that account." This needs to be a check that fails, not a warning that scrolls past.

**4. Make the published tool list match what's actually allowed.** We publish a menu of operations to agents, and separately enforce a shorter list at execution. They disagree — the menu still advertises posting to X, deleting posts, and submitting to Reddit. Worse, the menu rebuilds itself from our shipped files every time the server restarts, so an operator cannot remove those entries from their own database. An agent discovers a tool, tries it, gets refused, tries again. Both lists need to be the same list.

**5. Limit how much any one person can use.** Everyone shares one key with each vendor. Without a per-user cap, one enthusiastic session exhausts the quota and the demo stops working for everybody.

## What a user gets

**Social listening and research.** Search recent posts on X. This is public conversation, so a shared credential reveals nothing about anyone else, and it is a genuinely good demo: "what is being said about this." **Reddit is excluded** — see below.

**Read the web.** Firecrawl scrape and search, with a size cap and a spending ceiling per request. "Read me this page" is the strongest single first-run demo we have. It does cost us a little per page, and one reviewer would cut it for that reason — I think it earns its keep, but the disagreement is real and worth knowing about.

**Flight search.** Ask for flights and get real options back. We tested this: a live search returned 602 offers. Searching commits nothing and costs nothing. *Booking* flights is a different matter and is not part of this.

**A voice, optionally.** One stock voice, capped length. Pleasant, costs money per character. Worth noting the browser can already speak text for free, so this is only worth doing if we specifically want to show off the vendor.

## Phone calls — the piece you've asked for

You want Twilio for **calls**, not texts. That changes the picture, and it is worth being precise about why.

Twilio's call API takes a destination, a caller ID, and then either a URL or a block of markup that tells Twilio what to say and do once the call connects. The danger has never really been the phone number — it is that whoever supplies that script controls the call. Hand that to an agent and you have handed it an arbitrary automated calling system operating under our phone number.

**The version that works: we own the script.** The agent says "call this restaurant and ask whether they have a table for four at eight." NyxID builds the call flow itself, on our own endpoint, and hands Twilio only a reference to it. The agent never supplies markup. This pairs naturally with the voice work — the call can be spoken in the voice the user picked.

That is a genuinely differentiated capability. It is also **the largest single piece of work in this document**, and it is honest to say it is a product rather than a configuration change. It needs our own call-script endpoint, rules about who may be called, a per-user spend cap, and a decision about recording and consent — which varies by jurisdiction and is a real legal question, not a checkbox.

It is not an onboarding capability. Nobody's first minute on the platform should involve us phoning a stranger on their behalf. Build it deliberately, after the five items above, and let it be its own thing.

## What we cut, and why

**Sending SMS.** The dangerous parts are the destination and the message body, and no amount of configuration constrains either meaningfully. It also carries real regulatory exposure. Calls, with a script we own, are the better shape.

**Flight booking and payment.** Creating a booking writes passenger details into our airline account, reserves real inventory, and starts a payment clock. We would also be liable for fraud on money we never receive. That is a travel product, not a first-run demo.

**A general-purpose rule language.** We had designed a way to express detailed constraints on request contents, stored in the database. Both reviewers found it existed almost entirely to make SMS and flight booking safe — and that its flagship example did not actually work. Remove those two capabilities and there is nothing left for it to do. If we need it later, we will know precisely why.

## What we need from outside

**Reddit is excluded from this scope, by decision.** The credential it needs expires hourly and requires an auth flow we currently exclude from automatic setup, and their commercial terms for reselling access through a shared credential are unverified. Both are solvable; neither is worth solving now.

**Duffel's Stays and Cars** products are switched off on our account — hotels and cars are a sales conversation, not an engineering one. Flights work today.

**Duffel Cards approval** is needed before any payment work. Worth requesting now, because that clock runs regardless of what we build.

## Corrections to earlier drafts

Two things I previously told you that were wrong, corrected here so this document does not carry them forward.

I said the platform could already fall back to our credential when a user has not connected their own. It cannot — I misread two similar code paths. That capability would need building.

I described flight booking as safe because no money moves. It creates real obligations regardless: passenger data in our account, held inventory, and a deadline. "No payment" is not the same as "no consequence."

---

## How this gets built

We have done this once already, and it worked. `chrono-llm-public` is a platform service running in production today: an admin-managed row holding our credential, resolved identically for every caller, serving a capability nobody has to sign up for. It is the model. But the interesting lesson is **how little of this plan should copy it.**

### What the chrono-llm pattern actually cost

Four parts:

1. **A catalog row**, created through the admin interface rather than seeded in code — a slug, our credential, and the target URL. No deployment needed.
2. **A validation and request-building module** — roughly 490 lines. It checks what the caller asked for against fixed lists, and constructs the upstream request itself rather than passing the caller's through.
3. **A handler** — roughly 470 lines. Finds the row by slug, forwards, streams the answer back.
4. **Routes**, mounted with the usual wrappers.

About 950 lines for one service. That is the price of a purpose-built capability, and it bought real safety: the caller cannot ask for a model we did not approve, cannot inject a system prompt, and cannot reach a tool because the route has none.

### Why most of this plan will not cost that

**Four of the five capabilities need no code at all.** X search, Firecrawl, flight search and speech go through the proxy we already have, with a list of permitted operations attached to the row. That list is data. It lives in the database, an operator can change it without our involvement, and adding a sixth service later is the same act again.

Concretely, enabling one looks like a single administrative call that creates the row, attaches the credential, and attaches the permitted operations in the same request — so the service is never live-but-unrestricted for even a moment. Then a flag turns it on.

**One capability does need the chrono-llm treatment: phone calls.** For exactly the reason chrono-llm needed it — the safety required is about *what the request contains*, not which endpoint it reaches. NyxID has to build the call script itself, the way the assistant builds its own prompt. Same four parts, same rough size, and the same payoff: the caller supplies a destination and an intent, never the script.

### The order, and what each step touches

**First — let the assistant use tools.** This is the one that makes everything else visible, and it is the only item on the list that is not optional. It touches the assistant path rather than the catalog, and it is the piece where I would expect the estimate to be least reliable, because it depends on how the existing tool machinery fits the direct chat route.

**Second — the three safety fixes**, which are small and independent of each other. Make a platform row refuse to run without a policy. Make the credential check reject the wrong kind of credential outright. Make the published tool list stop rebuilding itself from our shipped files on every restart. Each is a contained change in code that already exists; none requires a migration.

**Third — turn on the capabilities**, one at a time, in the order they are least likely to embarrass us: X search, then web reading, then flight search, then speech. Each is configuration. Each can be reverted by disabling the row.

**Fourth — the rate limit**, before anyone outside the building sees it.

**Then, separately and deliberately — phone calls.** Its own design, its own review, and the compliance question answered before a line is written rather than after.

### How we will know it worked

A new account, nothing connected, asks the assistant what people are saying about a topic and gets real posts back. Then asks it to read a page. Then to find a flight. If those three work from a cold signup, the plan succeeded. If they do not, no amount of correctly-configured catalog rows will have mattered.

---

# Implementation specification

Concrete enough to hand to an engineer. Every file path and identifier below was verified against the branch.

## Item 1 — Let the assistant use tools

**This is not a build. The machinery exists and is already routed.**

`services/assistant_direct_agent_poc.rs` (1,579 lines) plus `services/assistant_direct_agent_poc/{tools,prompt,sse_decode}.rs` (1,845 lines in `tools.rs` alone) implement a read-only tool registry exposing two tools to the model — `nyx_search_tools` and `nyx_call_tool` — with operation resolution, eligibility filtering, and a result boundary. It is mounted at `POST /api/v1/assistant/direct/agent` (`routes.rs:136-138`).

**And it already understands platform services.** `tools.rs:198` maps `McpToolSource::Platform` to `"platform"` in its service listing — unlike `handlers/mcp_transport.rs:659`, which filters platform rows out for scoped keys. The POC does not inherit that restriction.

**What gates it:** `handlers/assistant_direct_agent_poc.rs:35` calls `require_direct_chat_enabled`, which checks the feature flag `experimental:direct-chat-engine` (`services/feature_flag_service.rs:111`), settable per-platform via `set_platform_override`.

**The work is therefore:**
1. Enable the flag for a test account and drive the POC against a configured platform row end to end.
2. Confirm `is_poc_operation_eligible` (`tools.rs:130`) admits the operations we intend to expose — it filters to typed read operations, so `POST /v2/scrape` and `POST /air/offer_requests` need checking specifically, since they are POSTs that are semantically reads.
3. Decide whether the POC becomes the default assistant path or stays flagged.

**Acceptance:** a flagged account asks for recent posts on a topic and receives real results, with the tool call visible in the response trace.

**This is the item whose size I cannot yet state honestly** — it depends entirely on what step 2 finds. Everything else below is bounded.

## Item 2 — Platform rows must refuse to run without a policy

**Current behaviour:** an absent `proxy_operation_policy` means passthrough — every operation allowed (`models/downstream_service.rs:351-353`). Only an empty-but-present policy denies.

**Change:** in `services/proxy_authorization.rs`, when the resolved service holds a master credential, treat absent policy as deny rather than allow. Reuse the existing predicate `is_valid_master_credential_service` (`services/proxy_service.rs:246`) to identify those rows.

**Acceptance:** a platform-credentialed row with no policy returns not-found for every operation; a non-platform row is unaffected.

## Item 3 — Reject the wrong kind of credential

**Current behaviour:** nothing inspects credential type. Seeded `api-twitter` carries `provider_config_id` and `auth_method: "none"` (`services/provider_service.rs:3765`, `:3825`), and its provider defaults include `tweet.write` (`:590-595`).

**Change:** at admin write (`handlers/services.rs`, the create and update paths that already call `normalize_policy`), reject a row that combines a master credential with either a `provider_config_id` or a user-mode credential shape. This is a hard error, not a warning.

**Acceptance:** creating a platform row with a provider config returns 400; the existing BYOK rows are untouched.

## Item 4 — Stop the overlay resurrecting removed operations

**Current behaviour:** `services/service_endpoint_service.rs:336` force-sets overlay-named endpoints `is_active: true` on every startup sync, so an operator cannot remove `POST /tweets` from their own database — it returns on the next boot.

**Change:** two parts. Trim the shipped overlays so they stop advertising operations we exclude — X `POST /tweets`, `GET /users/me`; Firecrawl `POST /v2/agent`, `GET /v2/agent/{id}`, `POST /v2/map`; ElevenLabs `GET /v1/voices` and the six `convai` operations. And make the sync stop reactivating an endpoint an operator has explicitly deactivated.

**Acceptance:** an operator deactivates an endpoint, restarts, and it stays deactivated. `tools/list` matches the policy.

## Item 5 — Per-user rate limit on the shared credential

**Current behaviour:** the limiter keys on API-key id and only runs when that key carries an explicit limit (`mw/rate_limit.rs:462-496`); browser sessions bypass it entirely.

**Change:** add a per-user bucket for platform-credentialed rows, keyed on the effective user rather than the key.

**Acceptance:** one session exceeding the cap is throttled without affecting another user.

## The service configurations

Each is one admin call — `POST /api/v1/services` — carrying `credential` and `proxy_operation_policy` together, so the row is never live-but-unrestricted. Then `is_active: true`.

**X** — one operation:
`GET /2/tweets/search/recent`
Excluded and to be removed from the overlay: `POST /tweets`, `GET /users/me`. Retained but unused: `GET /users/by/username/{username}`, `GET /users/{id}/tweets`.
Credential must be app-only.

**Firecrawl** — two operations:
`POST /v2/scrape`, `POST /v2/search`
Excluded: `/v2/agent` both verbs — job ids are not caller-scoped and no configuration makes them so — and `/v2/map`.

**Duffel** — two operations:
`POST /air/offer_requests`, `GET /air/offers/{id}`
Nothing under `/air/orders`, `/air/payments`, or `/air/order_cancellations`.

**ElevenLabs** — one operation, optional:
`POST /v1/text-to-speech/{voice_id}`
Excluded: `/stream`, `/v1/voices`, `/v1/models`, all six `convai` operations. The shared account should contain only offerable voices, since the matcher cannot constrain the path parameter.

## Sequence

Item 1 first — without it nothing is visible. Items 2 and 3 before any row is enabled, because they are what make an enabled row safe. Item 4 before agents see the tool list. X first among the services, then Firecrawl, then Duffel search, then speech. Item 5 before anyone outside the building.

Phone calls are separate work with their own design and review, after all of the above.

---

## Appendix — the specifics

**Operations to expose.** X: recent search only. Firecrawl: scrape and search, with a body-size cap and a credit ceiling — no crawling, no site mapping, no async jobs. Duffel: offer requests and offer retrieval, nothing else. ElevenLabs: text-to-speech only, one voice, capped length.

**Not exposed anywhere:** anything that writes, anything returning our own account identity, anything that lists activity across the shared account, and Firecrawl's asynchronous job polling — which accepts any job identifier and cannot be made safe by configuration, because nothing proves the job belongs to whoever is asking.

**What an operator supplies:** app-only credentials for each vendor, a curated voice library if they enable speech, and their own answer to the vendor terms questions above. Everything else is configuration on the service row.

**Where the code changes live:** assistant tool wiring; the credential validity check; policy-required enforcement on platform rows; the overlay/endpoint sync that currently resurrects removed operations on restart; and the rate limiter.
