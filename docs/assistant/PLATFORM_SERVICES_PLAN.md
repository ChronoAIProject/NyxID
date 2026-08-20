# Platform Services — What Can Be Done, and How

**Decision document.** Owner ask: *"I need plan to tell me what can be done and how it can be achieved."* Revised after adversarial review (11 findings, mostly CONFIRMED with citations); every finding is absorbed as a work item in §8 — none argued away. Repo claims cite `file:line` on `travel-allowlist`, rebased onto `origin/main` today. Sizes: **S** ≤ 2 days, **M** ≈ a week incl. review, **L** = multi-week. Every size is justified where claimed — the previous revision called the substrate "one backend PR" and was wrong; sizes below are itemized to rebuild that credibility.

## The 30-second table

| # | Decision | Enables | Costs | Depends on |
|---|---|---|---|---|
| 0 | **Launch free platform services now** (Firecrawl + X reads) | User value this week; pattern validated on real traffic | **Zero build.** Platform pays vendor bills, uncapped per-user (risk §2) | Nothing. Admin API can do it today |
| 1 | **Ops hardening** (credential rotate/clear + CLI, MCP↔policy parity, WS kill-switch gap) | Safe operations at any scale; agents stop discovering forbidden tools | **S+S+M** (itemized §3) | Nothing; parallel with Tier 0 |
| 2 | **Billing hardening** (6 confirmed defects, §4) | **The ability to charge.** Nothing is chargeable until this lands | **L** — honest total ≈ 3–5 weeks of focused backend work | Tier 1 not required; Lago rate config required |
| 3 | **Metered activation** per service (ElevenLabs, Twilio, Duffel searches) | Revenue per call | **S each** + per-service unit-fidelity decision (§4.6) | Tier 2 complete; Twilio also blocked on compliance decision |
| 4 | **Travel** (hold orders + payment page + skill) | The flagship | **M+M** (row+skill; page) | Tier 2 for the instant-order guard (§6); **Duffel Cards approval (external)** |
| — | **Fee/pricing decisions** (§9) | Unblocks 2→3 pricing and travel fee | Owner decision, not build | — |

**The central distinction this document enforces: what NyxID can *serve* is not what NyxID can *charge for*.** Serving safely is done — shipped and reviewed. Charging correctly is not — six confirmed defects stand between "metered" and "billable." Conflating the two was the prior error.

---

## §1 What is true today (verified)

- The **allowlist mechanism** is shipped, independently reviewed, and enforced before side effects on both executors (REST `handlers/proxy.rs:2021`; MCP `mcp_transport.rs:1357, 1695`). Nothing uses it yet (the only policy in `main` is a test fixture).
- An admin can **create a platform-credentialed row with a policy in one call today**: create accepts `credential` (`handlers/services.rs:49`) and `proxy_operation_policy` (`:91`, applied `:1056-1109`); update toggles `is_active` (`:252`) and the proxy rejects inactive rows — a working kill switch for *new* requests.
- The **billing subsystem exists end to end** (meter/reservation/ledger/reconcile/Lago) but `platform_billable` has never been enabled outside tests, and the review confirmed it is not charge-correct yet (§4).
- The four seeded rows can't carry the platform credential themselves: three are BYO-key connection services and all carry `provider_config_id`, which the credential gate rejects by design — platform variants are **fresh provider-less rows**, which the admin create API can already produce.

## §2 Tier 0 — free platform services, zero build (recommended: do it)

**The assessment asked for: yes, this is true, and free-tier-first is legitimate.** An admin today creates `platform-firecrawl` (internal, public, bearer credential, allowlist policy = scrape/search/map/crawl+status) and `platform-x` (reads/search only); the overlay publishes typed MCP tools; users call them, free. The shared credential is safe because the allowlist confines it and #1436 gates its decryption.

**What "free" costs, stated honestly:** ChronoAI pays the vendor bills with **no per-user ceiling** — the per-key limiter exists (`ApiKey.rate_limit_per_second`) but browser/session users bypass it (limiter keyed by API-key id, `mw/rate_limit.rs:462-496` — review finding 6), and there is no fair-share on the shared vendor quota. Acceptable for Firecrawl (cheap, capped by our vendor plan) and X reads (rate-pooled, read-only). **Not acceptable free for Twilio** (every call spends real money and carries TCPA exposure) **or ElevenLabs** (quota burn + voice abuse) — those wait for Tier 2/3. Known rough edges accepted at this tier: no rotate-in-place (**ASSUMPTION:** the admin update path may accept a credential — verify; fallback is delete/recreate, which changes the UUID), duplicated POSTs on MCP node-retry (finding 10, `mcp_service.rs:3664-3739`) cost a wasted scrape — tolerable free, must be fixed before *billed* writes, and non-allowlisted operations discovered via MCP 404 opaquely (finding 8 — fixed in Tier 1).

## §3 Tier 1 — small, high-leverage ops work (sizes justified)

1. **Admin credential subresources + CLI** — `PUT/DELETE /api/v1/services/{id}/credential`, `nyxid admin service credential set|clear` + `enable|disable`. **S**, justified: two handlers mirroring existing admin patterns in the same file, one CLI command family, no new models or migrations; the PUT validates provider-less + policy-present. Enables rotation without UUID churn and an audited kill sequence.
2. **MCP publication ↔ policy parity** (finding 8) — filter published operations to the policy *after* full catalog assembly (seeded, admin, instance-override, fallback sources — the draft plan's enumerated list). **S–M**, justified: one filtering pass at a known chokepoint plus tests per source; no schema change. Enables: agents stop discovering tools they can't call and burning retries on 404s.
3. **Kill-switch completeness** (finding 9) — credential clear + disable stops *new* requests only; in-flight HTTP (seconds) is fine, but **long-lived WS/streaming sessions hold the decrypted credential until they end**. Fix: terminate active WS sessions for a row on disable. **M**, justified: touches session tracking in the WS manager, not just a flag check. Until it lands, the runbook states the gap.

## §4 Tier 2 — billing hardening: what stands between "callable" and "chargeable"

All six are review-confirmed. Combined honest size: **L (≈3–5 weeks)** — this is the real work, and none of it is optional for charging.

1. **Cost ceiling at reservation** (CRITICAL, finding 2): reservation always reserves quantity `1` (`reservation.rs:1058-1116` — verified: `estimate_fresh_credits(..., 1, ...)`), settlement bills the full measured quantity (`meter.rs:480-485`). One credit of headroom can buy megabytes. Fix: per-operation reservation sizing (configured max-units per rule, or caller-declared cap validated at authorize time), settle ≤ reserve or re-gate. **M** — touches the reserve path's sizing contract, not its concurrency machinery.
2. **Forwarded-row recovery** (finding 3): a crash after `mark_forwarded` strands the wallet hold forever (`reservation.rs:961-983` recovers only unforwarded/finalized shapes). Fix: a recovery rule for forwarded-but-never-finalized rows (finalize from recorded response metadata after a deadline, else release). **M** — new sweep clause + crash tests in the style the suite already has.
3. **Failure refund semantics** (finding 4): failed/partial upstream responses are billed today (`handlers/proxy.rs:3950-3986, 3650-3807`). Decide the policy — recommend: no charge on transport failure/5xx/timeout; charge on 2xx and on 4xx-caused-by-caller; document per-service exceptions — then implement in the settle path. **M**, mostly in defining partial-stream semantics.
4. **Activation wiring** (finding 5): `BILLING_ENABLED` and `BILLING_FAIL_CLOSED` both default false (`config.rs:1119, 1159`) — enabling a row without flipping these yields metered-but-free. Fix: codified activation runbook (policy → credential → Lago rate → `BILLING_ENABLED` → `platform_billable` → enable) plus an `enable`-time check that refuses a billable row when billing is off or the rate is missing (`estimate_fresh_credits` fails closed on missing rates, `reservation.rs:1102-1115` — the check makes the outage impossible instead of documented). **S** once Tier 1.1 exists.
5. **Fair-share on shared credentials** (finding 6): add a per-*user* bucket (not just per-key) for platform rows + a per-service concurrency cap. **M** — limiter extension along existing lines (`mw/rate_limit.rs:462-496`).
6. **Metric fidelity** (finding 7): our meters don't match vendor billing — Firecrawl bills per page (a crawl is one request, many pages), ElevenLabs per character (bytes are codec-dependent), Twilio per message segment. Two honest options for the owner: **(a) approximate-with-margin** — charge per-request/bytes with a safety margin, simple, ships with Tier 2, mispriced at the tails; **(b) exact unit adapters** — per-service extractors reading pages/characters/segments from request/response, correct, **+M per service**. Recommend (a) for launch with (b) scheduled for whichever service shows real volume. Decision §9.

**Also in Tier 2 because it gates billed writes:** MCP node-retry duplicate-POST fix (finding 10) — a duplicated billed write is a double charge; **S–M** (no-retry-on-ambiguous for POSTs through the node path, matching the travel rule).

## §5 Tier 3 — external blockers (no build can move these)

**Duffel Cards approval** (request now — gates the travel payment page only; test mode has 3DS cards). **Stays/Cars entitlement** (both live-403 today; commercial conversation). **Twilio sender/compliance decision** (platform-owned numbers sending user SMS is a legal posture). **Lago rate configuration** (finance/ops, prerequisite to any Tier 3 activation).

## §6 Travel — intact, sequenced late, with the CRITICAL finding absorbed

The agent-driven flow stands unchanged (hold orders → final bill → payment link to the NyxID page embedding Duffel Cards; SAQ-A; no markup on the card leg — verified; chargebacks on ChronoAI; agents cannot pay; skill ships with the row; full contract in git history of this file and unchanged in substance). **Finding 1 (CRITICAL) lands here and is real:** the allowlist matcher checks method+path only (`proxy_authorization.rs:186-203` — confirmed), so a policy allowing `POST /air/orders` also allows `type: "instant"` with a `payments` block — an agent *could* pay after all. Fix options: **(a)** the generic runtime body-validation hook from the parked draft (validate allowed writes against a pinned schema requiring `type: "hold"` and forbidding `payments` — also what makes Twilio bodies safe), **M**, generic; **(b)** a NyxID-owned hold-create endpoint that constructs the body server-side, **S–M**, travel-only. **Recommend (a)** — it is the same primitive Twilio activation needs, so it pays twice. Travel's dependency line: Tier 2 items 1–4 + body validation + Cards approval.

## §7 Sequence

```
now:      Tier 0 launch (Firecrawl free, X-reads free)      [zero build]
parallel: Tier 1 (S+S+M)  |  Cards approval request  |  Lago rates  |  compliance decision
then:     Tier 2 billing hardening (L)  ──►  Tier 3 activations (S each: ElevenLabs, Twilio, X)
then:     Travel row + skill (M) ──► payment page (M, gated on Cards approval)
```

## §8 Review-findings disposition (nothing argued away)

| Finding | Disposition |
|---|---|
| 1 instant-order bypass (CRITICAL) | §6 — body validation hook, recommended option (a) |
| 2 quantity-1 reservation (CRITICAL) | §4.1 |
| 3 forwarded-row stranding | §4.2 |
| 4 failed-response billing | §4.3 |
| 5 billing flags default-off | §4.4 |
| 6 no fair-share / browser bypass | §4.5; accepted-with-eyes-open for Tier 0 free launch (§2) |
| 7 metric/vendor mismatch | §4.6 — owner decision (a) vs (b) |
| 8 MCP publishes excluded ops | §3.2 |
| 9 kill switch vs in-flight | §3.3 — WS session termination |
| 10 MCP node retry duplicates POSTs | §4 tail — before billed writes; tolerated free (§2) |
| 11 PR-1 undersized | Absorbed: the old "PR-1" is now §3 (S+S+M) *plus* §4 (L), sized item by item |

## §9 Decisions needed from the owner

1. **Ship Tier 0 free now?** (Recommended yes — Firecrawl + X reads.)
2. **Metric fidelity:** approximate-with-margin at launch, exact adapters later — or exact-first? (§4.6; recommend approximate.)
3. **Fee/pricing:** per-service credit rates for Tier 3, and the travel fee (unchanged constraint: no markup on the card leg — fee is none / credits-via-metering / a Links funnel).
4. **Twilio compliance posture** (§5) — activation blocker for Twilio only.
5. **Dual-mode vs convert** for the three BYO-key rows (recommend dual-mode: new platform rows alongside existing connections).
6. Confirm the sequence in §7 (Firecrawl proves the pattern; travel is not delayed by going later — the Cards approval wait runs in parallel).

**Assumptions register:** admin-update credential rotation path (verify; fallback delete/recreate); Firecrawl v2 paths re-pinned at activation; ElevenLabs streaming byte-metering behavior; X app-only read scopes; Duffel items unchanged (Cards timeline, idempotency-key absence — design never retries writes).
