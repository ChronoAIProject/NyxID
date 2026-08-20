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

**Social listening and research.** Search recent posts on X. Read subreddit listings, search Reddit, pull comment threads. This is public conversation, so a shared credential reveals nothing about anyone else. It is a genuinely good demo: "what is being said about this."

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

**Reddit** may not ship at all. The credential type it needs expires hourly and requires a flow we currently exclude, and their commercial terms for this kind of use are unverified. Either that changes or Reddit waits.

**Duffel's Stays and Cars** products are switched off on our account — hotels and cars are a sales conversation, not an engineering one. Flights work today.

**Duffel Cards approval** is needed before any payment work. Worth requesting now, because that clock runs regardless of what we build.

## Corrections to earlier drafts

Two things I previously told you that were wrong, corrected here so this document does not carry them forward.

I said the platform could already fall back to our credential when a user has not connected their own. It cannot — I misread two similar code paths. That capability would need building.

I described flight booking as safe because no money moves. It creates real obligations regardless: passenger data in our account, held inventory, and a deadline. "No payment" is not the same as "no consequence."

---

## Appendix — the specifics

**Operations to expose.** X: recent search only. Reddit: subreddit hot and new, search, comment threads. Firecrawl: scrape and search, with a body-size cap and a credit ceiling — no crawling, no site mapping, no async jobs. Duffel: offer requests and offer retrieval, nothing else. ElevenLabs: text-to-speech only, one voice, capped length.

**Not exposed anywhere:** anything that writes, anything returning our own account identity, anything that lists activity across the shared account, and Firecrawl's asynchronous job polling — which accepts any job identifier and cannot be made safe by configuration, because nothing proves the job belongs to whoever is asking.

**What an operator supplies:** app-only credentials for each vendor, a curated voice library if they enable speech, and their own answer to the vendor terms questions above. Everything else is configuration on the service row.

**Where the code changes live:** assistant tool wiring; the credential validity check; policy-required enforcement on platform rows; the overlay/endpoint sync that currently resurrects removed operations on restart; and the rate limiter.
