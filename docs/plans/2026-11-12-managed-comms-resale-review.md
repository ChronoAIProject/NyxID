# Managed phone and voice resale: review ledger

Plan: [2026-11-12-managed-comms-resale.md](2026-11-12-managed-comms-resale.md).
Reviewers: GPT-6-Astra (adversarial plan review), Fable 5.1 (arbitration of
Opus/Astra disagreements), Opus 5.5 (author and implementation reviewer).

## Round 1: GPT-6-Astra on draft v1, verdict REWORK

Reviewed at `6d3cc67cee71b368be6ed4b4532ba226bc5615b3`. Findings in Astra's own
severity, with Opus dispositions.

| # | Sev | Finding (condensed; evidence in Astra's report) | Opus disposition |
| --- | --- | --- | --- |
| A1 | P0 | Platform bindings resolve the vendor key through `resolve_catalog_platform_target` (`binding.rs:43`), the same catalog platform-key grant that allows generic proxy/MCP access to the raw vendor account. A line claimant would also get raw Telnyx/Twilio access. | **Accept.** Verified. v2 adds a managed-communications execution authority: allocation/claim-bound credential resolution that is not a platform-key grant, enforced so generic ingress (HTTP proxy, MCP, WS, provisioning, discovery) never gains vendor access from a claim. |
| A2 | P0 | NyxID watchdog is not a provider-enforced bound; Telnyx Answer/Relay expose no duration cap; cleanup needs resolvable credentials; `hangup_dispatched` is recorded before the network call and uncertain commands are not replayed; revocation, provider failure, failed proof or a missed answer callback defeat it. Keep platform Telnyx voice refused, or treat it as an underwritten exception with a platform loss budget and commercial acceptance, never as reserved liability. | **Disputed in part → Fable.** Opus agrees it is not reserved liability and must not be described as such. Opus proposes implementing the underwritten exception now (owner explicitly asked to resell Telnyx): default off, per-trusted-account platform loss budget, 30 s windowed funding, watchdog + recovery-sweep hangup with replay of uncertain hangups, separate billable/provider/absorbed seconds. Astra recommends deferring it. |
| A3 | P1 | Telnyx `call.hangup` parses `seconds: None` (`protocol.rs:336`); provider duration lookup is BYOK-only (`coordinator.rs:311`); `session.rs:516` falls back to elapsed time and then zero and freezes it. | **Accept** (applies to any platform Telnyx voice path): termination ≠ billable quantity; missing duration stays pending; three quantities recorded separately. |
| A4 | P1 | Allocation unique indexes (provider/account/resource, /phone, /application; `session.rs:28`) and single-service allocation cannot express one number with voice + SMS. | **Accept.** Managed numbers get their own ownership record with capability-specific associations; they claim the existing global `ResourceClaim` identities so manual allocation cannot capture pool resources. |
| A5 | P1 | Stable pool URL cannot simply delegate: Twilio signatures bind the binding URL (`coordinator.rs:104`); routing late terminal events to the current claim leaks evidence/messages across tenants. | **Accept.** New authenticated dispatcher; new calls/messages route to the current eligible claim; terminal callbacks and receipts route via immutable effect → original claim/binding mappings. |
| A6 | P1 | "Charge + bind + enable in one transaction" needs a caller-owned funding transaction and recovery contract (precedent `sandbox_runtime/windows.rs:151`, `CMA_BILLING.md:134`). | **Accept.** Request UUID + payload digest, claim generations, unique period receipts, caller-owned transaction with existing funding primitives. |
| A7 | P1 | Rental as a voice-lane component conflicts with `call_channels/billing.rs:50` (rejects components), `route_context.rs:209` (unsynced components free), no rental quantity source (`meter.rs:1223`). | **Accept.** Dedicated lifecycle admission for rental; required managed prices fail closed. |
| A8 | P1 | Price consent, paid coverage, midnight/price-change behaviour underspecified. | **Accept.** Immutable quotes, `paid_through` checked at execution admission, no back-charging suspended days, admin suspension not cleared by top-up. |
| A9 | P1 | Per-character "exact" billing over a WebSocket is not durable evidence of provider synthesis; recommends a verified bundled voice-second surcharge and deferring character billing. | **Accept the billing unit** (per-second surcharge, no new TTS metric). **Disputed scope → Fable:** Opus keeps premium ElevenLabs voice in this phase (per-second surcharge + NyxID-enforced per-call synthesis character cap as a supplier-cost ceiling); Astra recommends deferring premium provider switching. |
| A10 | P1 | Single `vendor_cost` would mislead (Twilio call `price` excludes relay/TTS; Telnyx multiple record types; ElevenLabs billed separately). | **Accept.** Component-level cost evidence with completeness; no margin shown as established while components are missing. Minimum reconciliation is launch-blocking. |
| A11 | P1 | Inbound SMS liability is incurred before funding (`coordinator.rs:334`); partial per-part accounting is not resumed. | **Accept.** Persist inbound accounting progress and recover; unfunded inbound cost recorded as absorbed liability. |
| A12 | P1 | Existing create/reverify/remove/revoke routes can bypass pool state (`management.rs:358`). | **Accept.** Claim generation bound to allocations/bindings/sessions; shared gates enforce pool state, paid coverage, owner admission, suspension. |
| A13 | P1 | "Concurrency 2" is not a spend policy; no number-level slot; Twilio `<Reject>` must be first verb. | **Accept.** Atomic call slots, per-owner number quota, claim/release throttles, call/SMS rate limits, merchant spend limit, fail-closed before bootstrap. |
| A14 | P1 | Pilot's call → SMS confirmation not implemented (`profiles.rs:51` `initiated_send: false`). | **Accept: include** a narrowly scoped "text the current caller" confirmation bound to the call, line and verified caller number, bounded, reserved, never retried when uncertain. |
| A15 | P1 | Pooling does not remove regulatory/vendor-contract requirements (Twilio SG reseller/SBO, SG SMS sender registration, ElevenLabs OEM terms, Twilio ToS §2.2, Telnyx messaging terms). | **Accept.** Operator-approved merchant eligibility record and per-number regulatory/eligibility gates before claim; end-user terms acceptance. |
| A16 | P1 | Erasure (`personal_erasure_service.rs:196`, `local.rs:143`) unaware of pool/rental records; local retire ≠ supplier release. | **Accept.** |
| A17 | P1 | Rollback unsafe after the first new-metric charge (closed `BillingMetric` enum persisted in usage). | **Accept.** Readiness-gated activation; flag-off stops admissions but not settlement/cleanup; no destructive rollback after financial data exists. |
| A18 | P1 | Missing executable validation/integration contract. | **Accept.** Test matrix, fixtures, CLI/frontend parity, docs/CLAUDE.md. |
| A19 | P2 | Baseline wording: deployment facts vs code facts; tariff lacks monetary amount/currency/increments. | **Accept.** |

Astra's answers: Q1 resell capabilities (yes) plus provider-side tenant isolation
where practical (Twilio subaccounts early; Telnyx Managed Accounts need approval);
Q2 stable pool URLs; Q3 per-second surcharge; Q4 minimum reconciliation required
now; Q5 ≥30-day cooldown, 7-day unpaid retention, operator approval before
cross-merchant reassignment in the pilot. Scope: operator-approved cohort,
pre-provisioned compliant numbers, prepaid rental, durable claim/release, web/CLI
parity, minimum reconciliation.

## Fable 5.1 arbitration (binding)

- **A — Telnyx platform voice:** ship the underwritten exception, but only with
  evidenced provider ceilings (non-null inbound channel limit on every pool
  application; maximum inbound leg duration from Telnyx written confirmation or an
  observed `time_limit` hangup on a paid-tier inbound test call; full per-minute
  tariff) and absorbed exposure **reserved per call at admission** against a
  rolling 30-day per-account budget, released on settled provider evidence.
  Hangup deadline ≤ funded horizon − 60 s and ≤ 1,740 s, anchored at answer
  dispatch. Replace the pre-written dispatched flag with attempts + confirmed and
  replay the derived `command_id`. Cleanup resolves the trusted account
  credential bound to the admitted session. Billable quantity is provider
  duration only (hangup payload → call read → detail record), else pending.
  Record billable/provider/absorbed seconds separately. Docs/UI say
  "underwritten exception, not reserved liability".
- **B — Premium ElevenLabs voice:** in this phase, Telnyx relay only, per-second
  surcharge as a second `VoiceSeconds` meter on a dedicated ElevenLabs lane; per-
  call Unicode-scalar character cap (greeting + frames) persisted on the session;
  per-account monthly character budget and global kill switch; plan tier, OEM
  terms version and admin acknowledgment on the account; merchant End User
  Agreement clickwrap; integration-secret reference usable only from NyxID-
  authored relay start; no silent fallback while charging; Twilio pinned to
  Google Standard.
- **C — Cohort:** mandatory admin-approved eligibility record per owner (business
  identity, jurisdiction, terms versions, SMS sender path/registration,
  approver, revision), checked at claim, renewal and every admission via a
  revision stamped on the claim; pool numbers carry regulatory evidence and true
  capabilities; additive to both flags.
- Additional: post-hoc absorbed counters cannot bound loss (reserve at
  admission); connected timestamp is webhook-only (anchor at dispatch); Telnyx
  hangup payloads lack duration and call reads have limited retention (detail
  record fallback); per-component cost capture; trial-tier ceilings invalid.

All rulings and Astra's accepted findings are folded into plan v2.

## Round 2: GPT-6-Astra on v2, verdict REWORK (13 × P1)

Astra accepted Fable's rulings and found precision gaps. All accepted; v3
resolves each:

| # | Finding | v3 resolution |
| --- | --- | --- |
| R2-1 | One funding horizon; define billable vs absorbed seconds | D6: 1,800 s only; `billable=min(provider,1800)`, `absorbed=provider−billable`, pending if missing; anchor/deadline persisted before answer leaves |
| R2-2 | Underwriting needs a consumed-loss balance and tariff-unit exposure | D5 budget primitive invariant; D6 hold→recognized replacement, loss above hold, frozen tariff/FX, increments/minimums, capacity revalidation, character cap for standard synthesis |
| R2-3 | Two meters need distinct identities and one admission/finalization contract | D6 admission receipt with `managed-call:{session}:base|premium`, one transaction, verified finalization, missing rows = recovery error |
| R2-4 | Durable character ceiling | D7 atomic block pre-debit before greeting/text, uncertain writes consumed, wrap-up reserve, ElevenLabs-account-keyed monthly budget, kill switch per block, fallback only pre-dispatch |
| R2-5 | Account/credential/price relationships; cleanup authority | D0 one canonical credential service per account, billing-only services, dependency fences, admitted-reference cleanup allowlist |
| R2-6 | Custody and legacy-route isolation | D2 platform custody for life, exclusive resources, server-derived `ManagedLine`, dedicated managed claim fields, exception only for managed path |
| R2-7 | Inbound SMS routing vs liability | D3/D9 SMS always records liability first; immutable per-part outcome |
| R2-8 | Late events, erasure | D3 effect-route tombstones and quarantine of unseen older events; D11 corrected acceptance guard and retention |
| R2-9 | Confirmation identities | D8 durable operation keyed by session/tool invocation/ordinal, per-send IDs, dual identity revalidation, BYOK association, model-asserted consent |
| R2-10 | Renewal boundaries and price acceptance | D4 immutable intervals, commit at cutoff, ≤1 h pending grace, renewal quote acceptance (web/CLI), auto-renew consent |
| R2-11 | Write-fenced eligibility and spend limit semantics | D1 explicit revision handoff; D5 conditional fences and budget primitive for monthly managed spend |
| R2-12 | Detail-record identity/duration contract | D6 persisted control/leg/session IDs; sources 1–3; call-control record fields verified via `/v2/detail_records/options` before enablement; terminated vs finalized states; admin evidence resolution |
| R2-13 | Cost completeness vs inventory periods | D10 inventory cost periods per supplier cycle; coverage vs evidence quality; estimated vs actual margin |

## Round 3: GPT-6-Astra on v3, verdict REWORK (2 × P1, 4 × P2)

11/13 round-2 findings closed. Remaining, all applied verbatim or as directed in v3.1:

- P1 — BYOK owner-signed timestamps must never establish quantities (D6 source 2 is `ManagedLine` only; BYOK pending without API/CDR evidence; regression test).
- P1 — No unfunded renewal grace; renewal commits funded coverage before the cutoff; release cancels only uncommitted renewals (D4, D5).
- P2 — Premium settles on `billable_seconds` (D7); credential rotation vs rebinding (D0); managed gate only for `ManagedLine`, BYOK confirmations keep existing fences (D8); provider-specific hangup retry (D6).

## Round 4: GPT-6-Astra on v3.1 — **APPROVE**

R2-10, R2-12 and all four round-3 P2 items confirmed closed against v3.1; no new
P0/P1 issues. Fable's binding conditions intact. Plan converged; implementation
proceeds under D14's validation contract.

## Implementation review log (Opus)

| Round | Scope | Findings | Outcome |
| --- | --- | --- | --- |
| M1 r1 | Foundations | F1–F3 P1 (in-use row designation; readiness on managed writes; existing-effect authority), F4–F5 P2, F6–F7 P3 | All fixed with tests |
| M1 r2 | F-fixes | R1–R3 P2 (creator-role dependency; BYOK ingress order; socket call validation), R4–R5 P3 | All fixed with tests |
| M2 r1 | Lifecycle billing | M2-1 P1 (readiness inside shared billing batches; corrected readiness scope), M2-2 P2 (voice/language allowlist), M2-3/M2-4 P3 (quote growth/TTL; erasure marker) | All fixed with tests; 868 tests pass |
| M3+M4 r1 | Ingress/SMS, calls/underwriting | M34-1 P1 (BYOK pending quantity could never settle → bounded 72 h fee waiver + admin resolution), M34-2 P2 (slot release on confirmed termination), M34-3 P3 (typed collections, retention) | All fixed with tests; 660 unique tests pass (Sol 2, fresh agent) |
| M5+M6 r1 | Premium voice, caller confirmations, cost evidence | M56-1 P2 (block pre-debit instead of a transaction per delta), M56-2 P2 (no default NyxID greeting; localized, reserve-bounded wrap-up), M56-3 P2 (caller-confirmation tool outside the `nyxid__` namespace, classified `change`, documented guest exception), M56-4 P3 (prove the integration-secret reference at configuration/rotation), M56-5 P3 (MongoDB-aggregated report totals with paged detail) | All fixed with tests; fmt, whitespace and strict Clippy (production and tests) pass on the post-fix source; only later change is test fixtures. Runtime suites run in CI with the final validation (local host could not finish the test build) |
| M7 r1 | Surfaces, CLI, docs, admin line operations | M7-1 P2 (retired/supplier-released lines still count toward Telnyx capacity and block credential rotation), M7-2 P2 (web claim pre-checked auto-renew), M7-3 P2 (admin inputs required BSON extended JSON), M7-4 P3 (CLI file credential not fully zeroized), M7-5 P3 (per-row reads in inventory listings) | All fixed with tests (shared capacity/custody-proof selectors with an account-identity rotation proof, opt-in auto-renew, decimal-string and RFC 3339 API inputs, zeroized credential file reads, batched listing projections) |
| M7 r2 | M7 r1 fixes | None | Accepted. Local: backend `cargo check`, fmt, CLI tests (2,001) and clippy, frontend lint/test (8,179)/build pass; the backend suite and strict Clippy run in CI |
| Rebase | Onto main `0aa248d7c` (shutdown framework, billing recovery batches, stuck-request recovery) | RB-1 P2 (caller-confirmation and managed SMS reply meters could be waived by stuck-request recovery) | Fixed: every managed effect identity (`managed-call:`, `managed-sms-out:`, `managed-inbound:`, `managed-line-rental:`, `call-confirmation:`) is excluded in both lanes; `push_unacked` guard re-pinned for `phone_number_days` only (accepted) |
| Rebase r2 | RB-1 fix | None | Accepted; backend suite and strict Clippy run in CI |
| CI round 1 | First CI compile/run of the backend tests (PR #2564) | CI1-1 P1 (admission fences compared a counter the admission itself advanced: concurrent admissions on one provider account refused each other), CI1-2 P3 (test expectation vs the exact exponent Credits parser), CI1-3 P2 (base fixture lacked the operator system role) | Fixed: authority `fence` separated from the write-only `admission_touch` conflict counter at every read-fence (all fence sites audited); concurrency, authority-change and slot-capacity tests |
| CI round 1 r2 | Review of the fixes | CI1-4 P3 (renewal refused a racing call admission), CI1-5 P2 (one unreadable row failed a whole listing) | Fixed: renewal uses `admission_touch`; per-row tolerant listings with raw-cursor pagination and additive `unreadable_ids`; rebased onto main `2e08abb85` |
| CI round 1 r3 | Review | CI1-6 P3 (`unreadable_ids` invisible to operators) | Fixed: web warning banners and CLI JSON/stderr reporting across pages; accepted with no further findings |
| CI round 2 | Second CI run | CI2-1 P1 (a refused confirmation kept its reserved credit: zero settlement only finalizes forwarded rows, and RB-1 removed the stuck-request backstop) | Fixed: parts are forwarded inside the admission transaction (`forward_in_session`), `cancel` releases or zero-settles, and a managed reservation sweep releases unadmitted parts and finishes undispatched sends behind a `prepared` → `dispatching` barrier |
| CI round 2 r2 | Review | CI2-2 P2 (an uncertain forwarded SMS part could stay held forever) | Fixed: provider evidence first, else a 72 h `provider_evidence_unavailable` zero settlement for every RB-1-excluded part (audited, idempotent, late callbacks never charge); stale managed Twilio read reference fixed |
| CI round 2 r3 | Review | CI2-3 P3 (no managed Telnyx exact-message read) | Fixed: policy-gated fixed-origin Telnyx `GET /v2/messages/{id}` with identity checks; accepted with no further findings |
