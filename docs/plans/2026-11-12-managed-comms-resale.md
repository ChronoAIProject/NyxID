# NyxID-managed phone lines (Telnyx, Twilio, ElevenLabs): resale plan

Status: **v3.1 — implemented (M1–M7)** on branch
`managed-comms-services-reselling`, pending final review and CI validation. v2
folded in GPT-6-Astra round 1 and Fable 5.1's binding arbitration; v3 resolves
Astra round 2; v3.1 applies Astra round 3 text. Review ledger:
[2026-11-12-managed-comms-resale-review.md](2026-11-12-managed-comms-resale-review.md).
Implementation: GPT-6.1-Sol (M1–M6) and Opus (M7); Opus reviews every round
until no findings remain. Operator guide: [MANAGED_PHONE_LINES.md](../MANAGED_PHONE_LINES.md).

## 1. Why

The Jolly Chili pilot (NyxID#2129, Marketing#426) answers a restaurant line with
a NyxBot phone specialist: Singapore Telnyx number → Telnyx Conversation Relay →
specialist (FAQ + booking sheet), with booking confirmations by SMS. It works
only because Calvin's own Telnyx account, key, number, application, webhooks and
verification key were wired by hand (deployment fact, not verified from code). A
merchant today would need vendor accounts, a purchased and registered number,
pasted keys and copied webhook URLs. Goal: an approved merchant picks a NyxID
number, picks a specialist and pays in NyxID credits.

## 2. Baseline (code facts at `6d3cc67`)

| Capability | Profile | User-owned key | NyxID platform account |
| --- | --- | --- | --- |
| Telnyx inbound voice | `telnyx.conversation_relay` | yes | refused (`call_channels/billing.rs` `validate`) |
| Telnyx SMS | `telnyx.messaging` | yes | admin allocation |
| Twilio inbound voice | `twilio.conversation_relay` | yes | admin allocation; `TimeLimit=1800` confirmed before relay |
| Twilio SMS | `twilio.messaging` | yes | admin allocation |
| ElevenLabs Custom LLM | `elevenlabs.custom_llm` | yes | refused |

Platform allocations (`channel_adapter_allocations`) are admin-assigned to one
person, unique per provider/account × resource, phone and application, each with
one catalog service. Platform credentials resolve through
`proxy_service::resolve_catalog_platform_target` (`binding.rs:43`), the same
catalog `platform_key` grant that authorizes generic proxy/MCP use of the vendor
account. Calls reserve 1,800 voice seconds before answering; SMS reserve
conservative parts. Lanes are credential-class service prices; funding is the
exact allowance → grant → wallet path with Lago and the ledger. Primary usage
rows are keyed `{billing_request_id}:platform` (`billing/meter.rs:1171`, unique
index `db.rs:3395`).

## 3. Strategy: resell an outcome, not vendor API access

NyxID sells a **managed phone line attached to a NyxBot specialist**: a
NyxID-owned number, inbound calls answered by the specialist, inbound SMS
replies, booking confirmations texted to the caller, and an optional premium
ElevenLabs voice. Merchants never see vendor accounts, keys or webhooks.

Raw vendor API resale (generic `platform_key` on `api-telnyx`/`api-twilio`/
`api-elevenlabs`) stays out of scope: a shared account behind a generic proxy has
no tenant isolation, unbounded asynchronous liability (dial/transfer/SMS
pumping), no provider-unit evidence for those operations, and makes NyxID the
sender of record for arbitrary traffic. Provider-side tenant isolation (Twilio
subaccounts, Telnyx Managed Accounts, which need Telnyx approval) is a later
phase.

### Charges (NyxID credits, admin-authored with supplier tariff evidence)

| Charge | Retail unit | Admission | Quantity evidence |
| --- | --- | --- | --- |
| Line rental | number-day, prepaid 30-day periods | lifecycle admission (D4) | period receipt |
| Calls | voice second | managed call admission before answer (D6) | provider call duration |
| Premium voice | voice-second surcharge | same admission, second meter (D7) | same provider duration |
| SMS (inbound, replies, caller confirmations) | provider part | before send; inbound recorded as incurred (D9) | provider part evidence |

Retail price is policy (cost plus margin). Supplier tariffs (amount, currency,
unit, increment, minimum, effective date, HTTPS source) are evidence.

## 4. Design

### D0. Managed-communications execution authority

- `CredentialBinding::ManagedLine { line_id, managed_claim_id,
  managed_claim_generation }` (serde-tagged like the existing variants). Only the
  server constructs it (claim lifecycle); public binding create/update routes
  reject it, and every managed binding's account/number/application/profile
  fields are derived and matched server-side from the line.
- **Credential source.** Each trusted provider account keeps exactly one
  canonical credential catalog service (the existing `ProviderAccount.
  catalog_service_id`; existing single-service accounts are compatible). The
  managed resolver loads line → trusted account → credential service and
  decrypts its master credential **without** consulting the merchant's catalog
  `platform_key` grants, after checking the shared admission gate (D5). Billing
  services for capability prices, rental and premium (D2) hold prices only, never
  credentials.
- **Generic ingress refusal.** Credential services used by managed accounts and
  all managed billing services are marked server-side (`managed_comms_role`,
  admin-only field). Saving refuses an enabled `platform_key` audience on them;
  every generic ingress (HTTP/WS proxy, MCP discovery/execution, `/keys`
  provisioning and auto-provisioning, LLM gateway, connect links, catalog
  connect) refuses them. Only `call_channels` effects may use them, under the
  catalog operation policy and fixed provider hosts for new effects.
- **Cleanup authority.** Every admitted session/receipt stores an immutable
  admitted reference `{trusted_account_id, provider, account_id, resource ids,
  provider call/message ids}`. Cleanup and evidence operations (hangup of that
  exact call, call-status read, detail-record read, message read) use a narrow
  allowlist bound to that reference, resolve the trusted account credential
  directly, and do not depend on the claimant's later claim, eligibility, grant,
  catalog policy, or a successful active-call-list lookup. New effects keep all
  ownership proof and live policy checks. Credential-resolution failures in
  cleanup are surfaced, audited and retried, never swallowed.
- **Dependency fences.** While managed lines or unfinished managed effects
  depend on them, NyxID refuses account-identity changes, credential-service
  rebinding, deactivation, unsafe destination/policy changes, enabling
  `platform_key` on a managed service, and deletion of the account, its
  credential service or a managed billing service. Audited credential rotation
  is permitted when proven (provider read) to retain the same provider account
  and the required cleanup capabilities; it uses the existing secret-storage
  machinery and preserves admitted effect references.
- CLAUDE.md documents this explicit server-controlled authority as the only
  exception to "platform-bound resolvers call the shared ACL".

### D1. Merchant eligibility (cohort)

`managed_comms_eligibility` (one per person owner, UUID `_id`): business legal
name, registration ID, jurisdiction (ISO country), allowed use case, SMS sender
path and registration status, accepted terms versions (managed-line terms; End
User Agreement with ElevenLabs OEM minimum terms when premium is allowed),
`premium_allowed`, `max_lines` (default 1), optional `monthly_spend_limit`
(credits, D5), approver, approved_at, `revision`, `fence`, `revoked_at`.

- Platform-admin human-only create/update/revoke (audited, metadata-only). The
  merchant reads their record and accepts terms versions (clickwrap); they cannot
  self-approve.
- Claims stamp `eligibility_revision`. An admin update that needs no merchant
  consent advances claims only through an explicit admin "apply revision to
  claims" action (audited). A change of terms requires merchant acceptance
  before the claim advances; until then new admissions are refused with a clear
  reason. Revocation suspends claims (`eligibility_revoked`) and starts release.
- Org ownership remains out of scope (person-owned; usage billed to the person).

### D2. Managed lines, custody and the claim lifecycle

`managed_phone_lines` (UUID `_id`), one row per NyxID-owned number:

- provider, trusted account ID, E.164 number, ISO country, number resource ID;
  capability associations `voice {application_id, billing_service_id}` and
  `sms {messaging_profile_or_application_id, billing_service_id}` (each optional;
  only true capabilities; a voice-only Singapore number must not advertise SMS);
  `rental_billing_service_id`; regulatory evidence (requirement/bundle reference,
  registration status, SMS sender registration where offered, verified_at/by);
  supplier tariff components; provider channel limit (voice); admin call-slot
  limit; per-line call/SMS rate limits; state; current claim; `revision`; `fence`.
- **Exclusive resources.** Each line's applications/profiles are exclusive to that
  line (reuse only between the line's own capabilities). Registration refuses a
  resource already used by another line, allocation or binding.
- **Platform custody.** Registration records the number and every
  application/profile as global `ResourceClaim`s owned by a platform custody
  sentinel for the line's whole life (available, claimed, suspended, releasing,
  cooldown, retired). Merchant execution rights are the claim, never a
  `ResourceClaim` transfer. Every legacy create/allocate/reverify/remove path
  refuses custody resources; legacy release never deletes custody claims.
- **States**: `available` → `claimed` → `releasing` → `cooldown` (≥ 30 days,
  `available_at`) → `pending_reassignment` (operator approval required in the
  pilot) → `available`; plus `suspended {admin | unpaid | eligibility_revoked}`
  and `retired` (local only; supplier release is a separate operator record).
  Releasing → cooldown requires all of the claim's effects to be accounted (D8).
- **Registration** (platform-admin, human-only) proves with provider reads that
  the number and each associated application/profile belong to the trusted
  account; that configured inbound URLs equal the line's stable URLs (D3); that
  Twilio voice fallback URLs are empty; and (Telnyx voice) that the
  application's inbound channel limit is non-null. It revalidates underwriting
  capacity (D6).
- **Claim records** (`managed_line_claims`, UUID `_id` = `managed_claim_id`):
  line, owner, `managed_claim_generation` (line-scoped, monotonically
  increasing), eligibility revision, accepted terms, voice tier, SMS enabled,
  caller confirmations, auto-renew consent (at the accepted price), coverage
  receipts, state, `fence`. Bindings created for a claim carry the `ManagedLine`
  credential; sessions/receipts store the claim ID/generation in dedicated
  fields (`managed_claim_id`, `managed_claim_generation`), never the socket
  `claim_id`/`generation`.
- **Quotes** (`managed_line_quotes`): UUID, owner, line, kind (`claim` |
  `renewal {claim_id, generation, period_index}`), rental rate (pico credits per
  number-day), Lago code, rate revision, terms versions, current usage rates for
  display, `accept_before` (15 min). Usage rates follow the normal platform rule
  (current lane at admission, retained through settlement); rental is pinned per
  period.
- **Claim** (first-party human; flags D12; eligibility required):
  `{request_id (UUID v4), quote_id, line_id, agent_id, voice_tier, enable_sms,
  caller_confirmations, accepted_terms_versions}`. Actor-scoped request ID +
  payload digest: retries return the original outcome; a different payload with
  the same ID is 409. Provider proof and Lago/tax freshness run outside the
  transaction; then one MongoDB transaction (majority write concern) performs:
  owner admission fence, eligibility fence (`$inc fence` conditional on revision
  and not revoked), line fence (`available` + revision), `max_lines` check,
  first-period funding reservation and meter rows (D4), coverage receipt, claim
  insert, binding inserts (enabled), monthly-spend hold (D5), audit outbox.
  Settlement/journaling then proceeds through the normal recoverable path; an
  unknown commit is resolved by replaying the same request ID.
- **Release** (owner or admin; request ID + digest): CAS `claimed|suspended` →
  `releasing`; disable bindings; cancel any pending renewal; when every effect is
  accounted (D8), remove bindings (tombstones remain) and enter `cooldown`. The
  current paid period is not refunded (stated at claim).
- **Resume** from `suspended{unpaid}` (owner; request ID + digest; fresh quote):
  commits a new period `[now, now+30d)` (never back-charges suspended time).
  Top-up never clears `admin` or `eligibility_revoked`.

### D3. Line ingress and immutable effect routing

- Stable vendor-configured URLs: `POST /api/v1/webhooks/managed-lines/{line_id}/voice`
  and `.../sms` (public, outside JWT, bounded bodies, fixed provider origins,
  query-secret redaction).
- A dispatcher authenticates against the **exact configured line URL** (Twilio
  signature with the account credential; Telnyx Ed25519 with the trusted account
  key over timestamp + raw body within a replay window), parses a typed verified
  event, records the trusted provider occurrence time, then routes:
  - **Voice, new call** → the line's current eligible `claimed` voice binding,
    through managed call admission (D6). No eligible claim, failed admission or
    no slot → Twilio `<Reject/>` as the first verb; Telnyx `reject` — no answer,
    no retail charge.
  - **SMS** → always D9 ingestion first (liability record), then optional
    delivery to the current claimant.
  - **Existing effects** (terminal/status/receipt callbacks, duplicates) →
    resolved by provider identity through `managed_effect_routes` tombstones
    (metadata only: provider ids, line, claim id/generation, binding id, effect
    kind, financial state, coverage interval). They update the original effect's
    financial evidence or are acknowledged as already finalized; they never
    recreate erased tenant rows or reach a later tenant.
  - **Unseen event older than the current claim** (trusted occurrence time before
    the current claim's `covered_from`) or ambiguous → quarantined: liability
    recorded, content discarded, never delivered to the current tenant.
- NyxID-generated per-binding socket and SMS-receipt URLs are unchanged.

### D4. Rental billing

- New `BillingMetric::PhoneNumberDays` (`phone_number_days`), priced on the
  line's rental billing service platform lane and synced to Lago. Managed prices
  (rental, voice, premium, SMS) are **required**: missing, stale, retired or
  unsynced prices fail closed (the ordinary "unsynced extra is free" rule does
  not apply). Ordinary proxy requests can never produce this metric.
- Funding uses the existing caller-owned transaction funding primitive, extended
  through an explicit **managed lifecycle contract** (do not reclassify ordinary
  `VoiceSeconds` or other metrics as runtime allocation;
  `funding/machine.rs:502` keeps rejecting them for runtime callers).
- Coverage receipts (`managed_line_periods`): unique `{claim_id, period_index}`,
  immutable `[covered_from, covered_until)`, quantity 30, rate snapshot, billing
  request ID `managed-line-rental:{claim_id}:{period_index}`, usage-row IDs.
- **Renewal.** From 72 hours before `covered_until`, a leased sweep (existing
  billing reconcile cadence; no new env var) prepares the renewal: issues the
  renewal quote and notifies the owner once. If the price is unchanged and
  auto-renew consent exists, no further consent is needed; if the price changed,
  the owner must accept the renewal quote (web and CLI) bound to owner, claim
  generation, next period index, price revision, expiry and terms.
- There is no unfunded renewal grace. After the required price and terms
  consent, the sweep may commit the next period before the cutoff by atomically
  reserving its full rental funding and monthly-spend hold and inserting its
  immutable coverage receipt. The next interval begins at the previous
  `covered_until`. Once committed, that period follows the disclosed paid-period
  no-refund rule; release cancels only uncommitted renewals. At or after the
  cutoff, new execution requires a committed, funded receipt covering the
  admission time. Without one, the claim is `suspended{unpaid}` (one
  notification); inbound SMS still follows D9. Explicit resume begins
  prospectively and never back-charges an uncovered interval. Resolve an unknown
  commit through the original request identity before creating another renewal
  or deciding that coverage is absent. Unpaid retention 7 days, then release.
- Add the metric to every closed registry: reservation estimates, final
  quantities, stale-reservation handling, allowance authoring/discovery
  (`allowance_metrics`), usage reporting/rollups, admin usage, frontend schemas
  (`billing-metrics.ts`), CLI units.

### D5. Shared admission gate, limits and budgets

- **Admission gate** (claim, renewal, resume, call admission, SMS delivery and
  send, caller confirmation): owner admission fence, eligibility fence and
  revision, claim fence/state/generation, line fence/state, paid coverage (a
  committed, funded coverage receipt covering the admission time), flags and
  kill switches, account policy revision. These managed predicates apply only to
  `ManagedLine` effects. Fences are conditional `$inc` writes inside the admission
  transaction; every mutator of those records writes the same fence. A final live
  check runs immediately before each provider dispatch.
- **Call slots**: counted atomically in the admission transaction (prepared,
  active and unresolved possibly-live sessions hold a slot); limit = min(admin
  slot limit, provider channel limit). Per-owner `max_lines`, claim/release
  throttles, per-line call and SMS rate limits.
  M34-2: confirmed hangup or a provider terminal observation releases the call
  slot atomically with that observation; unresolved financial quantities retain
  their funding, monthly spend and underwriting holds independently.
- **Budget primitive** (`managed_budgets`), used for the monthly spend limit and
  the underwriting budget (D6): per budget key, an admission invariant
  `recognized_in_window + unresolved_holds + candidate ≤ limit`, with holds
  inserted atomically in the admission transaction and replaced idempotently by
  recognized amounts at settlement (releasing only unused capacity). Holds remain
  effective until resolved regardless of window boundaries.
- **Monthly managed spend limit** (optional, eligibility): pre-tax managed retail
  charges (rental, voice, premium, SMS) including benefit-funded shares, by UTC
  calendar month of admission. Model/tool charges are excluded and disclosed
  separately. Exceeding it refuses new admissions (calls rejected before answer).
- Admin suspend durably requests termination of live calls and reports
  "termination pending" until provider evidence confirms.
- Inbound only; no outbound dial, transfer, DTMF or media. SMS sends are limited
  to replies to inbound senders and D8 caller confirmations.

### D6. Managed call admission; Telnyx underwritten exception

**Managed call admission receipt** (`managed_call_admissions`, one per session):
deterministic billing request IDs `managed-call:{session_id}:base` and, for
premium, `managed-call:{session_id}:premium`; expected usage-row IDs; service
and rate snapshots; reserved quantity 1,800 s per meter; underwriting hold ID
(Telnyx); spend hold ID; answer-dispatch anchor and hangup deadline. All funding
(both meters), holds, the slot and the receipt commit in **one transaction**
before the answer request can leave NyxID; any failure rejects the call.
Finalization verifies every expected usage row and its settlement outcome;
missing rows are a recovery error (never an empty successful settlement with
`billing_finalized=true`). Tests cover second-meter funding failure, duplicate
admission, unknown commit and crashes between the two settlements.

**Quantities (both meters, every provider):**

- `provider_seconds`: validated provider evidence only (below).
- `billable_seconds = min(provider_seconds, 1800)` on each retail meter.
- `absorbed_seconds = provider_seconds − billable_seconds` (platform exposure;
  never billed later by recovery).
- Missing evidence keeps the session `terminated_quantity_pending`: neither zero
  nor elapsed local time. "Provider confirmed terminated" and "financially
  finalized" are separate durable states; recovery keeps fetching evidence; a
  404, expired call record, absent detail record or successful hangup never
  implies zero and never releases the underwriting hold.
- An audited platform-admin evidence resolution (`provider_seconds` with
  provider reference) is the only manual path for permanently missing evidence.

**Telnyx provider evidence** (persist `call_control_id`, `call_leg_id`,
`call_session_id` from authenticated ingress, separately from relay IDs):

1. Call status `GET /v2/calls/{call_control_id}` with `is_alive=false` and
   integer `call_duration` (existing BYOK source).
2. Authenticated `call.answered` occurrence time and `call.hangup` end time:
   `ceil(hangup.end − answered.occurred_at)`. **`ManagedLine` only.**
3. Detail record `record_type=call-control` matched by `call_leg_id` and
   connection, using `answered_at`/`finished_at`. Its schema is undocumented in
   the public spec: field availability must be verified with
   `/v2/detail_records/options` and a recorded fixture as part of underwriting
   enablement evidence; until then source 3 is disabled.

Source 2 is unavailable to `Connection`/BYOK bindings. Owner-supplied webhook
verification keys authenticate transport only; callback timestamps cannot
establish financial quantities. BYOK terminal quantities must come from the
authenticated provider API or an account-and-leg-matched, validated detail
record. If neither is available, retain `terminated_quantity_pending` and the
unsettled funding reservations; never fall back to owner-signed timestamps.
M34-1 review exception: after 72 hours from a durable server-observed termination,
unavailable BYOK evidence waives NyxID's own fee at zero, releases its reservation
and freezes audited `terminal_evidence="provider_evidence_unavailable"` without
inventing provider seconds. Credential loss does not block this recovery. A
successful authenticated provider read before waiver still determines quantity.
Human Admin evidence resolution also covers terminated BYOK calls, with the same
immutable/audited contract. Managed/platform calls never use the waiver.
Legacy rows without a server observation time start the window at recovery.

For `ManagedLine`, source 2 requires the admitted trusted-account verification key,
exact call identity, and provider-duration fixtures validating the timestamp
conversion and rounding before that source is enabled. Regression test: correctly
signed, owner-fabricated BYOK timestamps with unavailable API/CDR evidence cause
no quantity settlement and no reservation release.

Retail uses elapsed provider seconds; supplier-rounded billed duration belongs to
cost evidence (D10). Fixtures cover each source. These rules also replace the
current Telnyx fallback to elapsed time/zero (`protocol.rs:336`,
`session.rs:516`, recovery at `coordinator.rs:1101`) for every Telnyx path.

**Twilio voice** keeps its provider-enforced `TimeLimit=1800` and uses the same
admission receipt and quantity rules.

**Telnyx underwritten exception** (not reserved liability; docs/UI say
"underwritten exception, not reserved liability"). Lifted **only** for the
managed-line path; manual `Platform` allocations stay refused.

- Trusted Telnyx account `underwriting`: enabled flag, accepting platform admin,
  accepted_at, commercial acceptance text/version, budget (credits, rolling 30
  days), maximum inbound leg seconds with evidence (Telnyx written confirmation
  reference or an observed `time_limit` hangup on an inbound test call from a
  **paid-tier** account; trial evidence invalid), verified detail-record fields
  (source 3), supplier tariff revision with every per-minute component (carrier
  inbound, relay, STT) and character-priced components, frozen FX policy,
  `revision`. Any missing item keeps the refusal. Audited.
- **Per-call worst-case exposure**, from the frozen tariff revision with supplier
  increments and minimums rounded up: `(max_leg_seconds − deadline_seconds) ×
  Σ per-second supplier components`. Character-priced synthesis adds nothing
  after the deadline because NyxID sends no text after it (D7 cap applies to
  standard Telnyx voices too, so standard synthesis is bounded by the same
  pre-debited character allowance).
- Underwriting budget key = trusted account; invariant per D5. At admission the
  hold = worst-case exposure; at settlement the hold is replaced by recognized
  absorbed cost (`absorbed_seconds` × frozen per-second cost, with increments),
  recognized at settlement commit time; a loss above its hold is recognized in
  full and blocks further admissions until capacity returns (and alerts).
  Enablement and every capacity-affecting change (application registration,
  channel-limit, tariff or budget change) refuse when Σ channel limits ×
  worst-case exposure > budget.
- Hangup deadline = min(1,740 s, funded horizon − 60 s) from the persisted
  answer-dispatch anchor. The owning replica arms a timer; the recovery sweep on
  any replica enforces it.
- Replace pre-written `hangup_dispatched` with durable `hangup_attempts` and
  `hangup_confirmed` for every relay hangup. Telnyx re-issues the same derived
  `command_id` until 2xx or provider evidence shows the call dead; Twilio retries
  the `Status=completed` update against the same admitted account and Call SID
  and confirms through the provider response/status evidence. Termination
  confirmation stays separate from quantity finalization.

### D7. Premium ElevenLabs voice (Telnyx relay only)

- Relay start: `voice = "ElevenLabs.<model>.<voice_id>"`,
  `voice_settings = {type: "elevenlabs", api_key_ref}`. The ElevenLabs key is a
  Telnyx integration secret in NyxID's Telnyx account; NyxID stores only the
  reference plus allowed voices, the ElevenLabs billing account identity, plan
  tier (Scale/Business/Enterprise required), OEM terms version and admin
  acknowledgment. Twilio stays pinned to Google Standard.
- Billing: second `VoiceSeconds` meter on the premium billing service (D6
  admission receipt), settled on the same `billable_seconds`.
- **Character allowance** (applies to premium and standard Telnyx synthesis):
  Unicode scalar values. Before relay start (greeting) and before writing text,
  the writer atomically pre-debits blocks (e.g. 400 characters) from the
  per-call allowance (default 12,000, configured per account) and, for premium,
  from the shared ElevenLabs account monthly budget (UTC calendar month, keyed by
  ElevenLabs billing account identity across Telnyx accounts), under the live
  session/writer fence. Text is written only within debited blocks. Only portions
  proven never written to the socket are credited back at call end; uncertain
  writes stay consumed and are never replayed. A wrap-up reserve (e.g. 300
  characters) is set aside at admission; when the remaining allowance reaches it
  the specialist's final wrap-up is spoken within the reserve and the call ends.
- The global premium kill switch is rechecked at every block debit; when off,
  the writer stops premium synthesis and ends the call with the reserved wrap-up.
- Fallback to the standard voice is allowed only for a proven pre-premium-dispatch
  failure (relay start refused before any premium text left NyxID): the premium
  meter is released and recorded. After premium synthesis may have occurred,
  the call ends; the premium meter settles on the same
  `billable_seconds = min(provider_seconds, 1800)` as the base meter (never
  silently waived while supplier cost may exist); excess supplier cost remains
  absorbed.
- Claims with `voice_tier = premium` require `premium_allowed` and the accepted
  End User Agreement version.

### D8. Caller confirmation SMS

- Owner toggle per voice binding (`caller_confirmations`, default off). Managed:
  requires the line's SMS capability and the claim's enabled SMS binding. BYOK:
  requires an explicit same-owner association between the voice binding and an
  SMS binding on the same provider account and number, proven by provider reads
  at setup.
- A specialist tool available only inside an admitted voice-call turn on that
  binding. Destination is the carrier-verified caller number of the current call
  (never a parameter); sender is the line number; content ≤ 480 UTF-8 bytes.
- Durable confirmation operation (`managed_call_confirmations` / shared for
  BYOK): key `{session_id, tool_invocation_id, ordinal}`, payload digest,
  per-send billing IDs `call-confirmation:{operation_id}:{part}`; the per-call
  allowance of 2 is claimed atomically; uncertain sends consume their slot and
  are never retried. Stores the authorizing voice-session identity and the
  executing SMS-binding identity and revalidates both before dispatch. Managed
  lines also pass the D5 managed gate; BYOK confirmations retain the existing
  owner, connection, binding and session fences plus the provider-proven
  voice/SMS association, send funding and confirmation-operation identity. Delivery receipts use the existing receipt path with these IDs.
- The tool takes a `caller_consented` argument; audit records it as
  **model-asserted** consent tied to the current caller interaction, not proof.
  Metadata-only audit. Not available to any other turn or ingress.

### D9. Inbound SMS liability

- Every authenticated inbound SMS first creates an idempotent liability record
  (`managed_inbound_sms`, key `{trusted_account, provider_message_id}`; line,
  trusted occurrence time, parts from provider evidence) even with no claim, no
  coverage, flags off or an erased former owner — no fabricated user principal.
- Each part then gets one immutable terminal accounting outcome:
  `customer_funded {claim, billing request id}` or `absorbed`. Recovery resumes
  unfinished parts idempotently and can never bill a part already classified
  absorbed. Only customer-funded messages for an eligible current claimant reach
  an agent turn; otherwise content is discarded. Suspension stops agent work and
  is documented as not a supplier-cost cutoff.

### D10. Cost evidence and reconciliation

- `managed_comms_cost_evidence`, one row per effect (call session, SMS part set,
  caller confirmation) with components `{kind: carrier_inbound | relay | stt |
  tts | premium_tts | sms_part, source: provider_record | tariff_estimate |
  unknown, provider_record_id?, amount (decimal string, sign-normalized),
  currency, fx_policy_revision, supplier_rounded_quantity}`; absorbed seconds and
  absorbed SMS parts are classifications of the parent effect's cost, not extra
  components. Captured independently of customer settlement.
- **Inventory cost periods** (`managed_inventory_costs`): supplier rental per
  line per supplier billing cycle (explicit cycle boundaries, deduplicated by
  `{line, cycle}`), recorded whether the line is available, claimed, suspended,
  releasing, cooling down or retired-but-not-released; attributed to line/day
  reporting, including unclaimed inventory.
- Capture sweep with bounded retries: Twilio call `price` (connectivity only;
  ConversationRelay from tariff), Twilio message `price`; Telnyx detail records
  per record type (call-control when verified, messaging `cost`/`carrier_fee`);
  STT/relay/TTS and premium characters from tariff where no record exists.
- Two separate dimensions: component **coverage** (all required components
  present) and **evidence quality** (`actual` only when every component is a
  reconciled provider record). The report shows "estimated margin" whenever any
  component is estimated or unknown, and "actual margin" only for fully
  reconciled effects.
- Admin report `GET /api/v1/admin/managed-lines/report` (admin/operator read):
  per line/day billed credits, actual and estimated cost, unknown components,
  absorbed seconds/parts, inventory cost, underwriting and spend budget state.
  Linked from the Integrity page.

### D11. Erasure and retirement

- The current guard stays: unsettled call/SMS effects block erasure
  **acceptance**. Deletion preparation additionally stops renewals, releases the
  owner's claims (bindings disabled) and drains managed effects so acceptance can
  proceed once they are accounted.
- Erasure removes tenant content (bindings, sessions, transcripts, claim
  personal fields) and retains metadata-only financial and routing records
  (period receipts, admission receipts, effect-route tombstones, cost evidence,
  inbound liability, budget entries) under the billing retention policy. Late
  callbacks hit tombstones and never recreate tenant rows.
- Lines stay in platform custody and enter `cooldown` only after every effect of
  the released claim is accounted.

### D12. Rollout

- Flag `nyxbot:managed-lines` (default off), registered with security readiness
  so admin activation requires the declared, drained, upgraded roster; additive
  to `nyxbot:service-adapters` and to eligibility. Global premium kill switch and
  per-account underwriting switch are separate platform settings.
- Flag off: no new claims, renewals or call/SMS admissions on managed lines
  (inbound SMS still records liability); callbacks, settlement, cleanup and cost
  capture continue. While off, coverage is not extended; claims lapse at their
  cutoff into `suspended{unpaid}` without charge for the off period.
- New enum variants/fields are serde-defaulted where possible. Upgrade every API
  replica and worker before registering lines or authoring managed prices. No
  binary rollback after managed financial rows exist unless a reviewed
  compatibility reader ships; flag-off is the operational stop.

### D13. Surfaces

- Web, merchant (NyxBot Settings → Phone and speech connections): eligibility
  status and terms acceptance, **Get a NyxID number** (list with quote and usage
  prices, specialist, voice tier, SMS, caller confirmations, auto-renew consent),
  line detail (coverage, state, renewal quote acceptance, usage link),
  **Release**, **Resume**.
- Web, admin (Admin → Platform Credentials → phone accounts): eligibility,
  line registration/edit/suspend/resume/retire/reassignment approval,
  underwriting, premium, evidence resolution, report.
- CLI parity: `nyxid nyxbot line list|quote|claim|show|release|resume|renewal
  accept|terms accept` and `nyxid admin line register|update|suspend|resume|
  retire|approve-reassign|resolve-evidence|report`, `nyxid admin line
  eligibility grant|update|apply|revoke|list`, `nyxid admin line account
  underwriting|premium`.
- Docs: new `docs/MANAGED_PHONE_LINES.md`; update NYXBOT_SERVICE_ADAPTERS.md,
  TELNYX_INTEGRATION.md, API.md, billing docs, CLAUDE.md (authority rule,
  routes, flag, rollout); no new env vars.

### D14. Validation contract

Real MongoDB replica-set transaction tests: claim/release/resume/renewal
idempotency and unknown commits; two-replica claim and admission races;
eligibility update racing admission; renewal at the cutoff, price change,
unfunded pending renewal, release racing renewal commit, renewal unknown-commit
recovery; allowance/grant/wallet/tax splits for the new metric; second-meter
funding failure and missing-row finalization; underwriting sequential loss
consumption, loss above hold, window boundary with unresolved holds, capacity
revalidation; Telnyx evidence sources 1–3 and pending states; hangup retry with
the same `command_id`; character allowance block debits, crash with uncertain
writes, wrap-up reserve, kill switch, pre-dispatch fallback; caller confirmation
ordinals, uncertain sends, BYOK association; inbound SMS with no claim, partial
recovery, absorbed classification; callback replay across reassignment, unseen
old events, post-erasure callbacks; flag off; generic ingress refusal of managed
services (HTTP, WS, MCP, `/keys`, LLM, connect links); legacy routes refusing
custody resources. Frontend forms/schemas/hooks tests, regenerated channel-adapter
catalog fixture, CLI tests, secret-redaction tests, metadata-only audit
assertions. Controlled vendor fixtures only; live paid smoke tests are an
operator activation step.

## 5. Out of scope

On-demand number search/purchase and automated regulatory submission; Twilio
subaccounts / Telnyx Managed Accounts; outbound calling; Twilio premium voice;
hosted ElevenLabs agents; org-owned claims; raw vendor API resale; Jolly Chili
specialist behavior defects #2260–#2263; refunds of paid periods.

## 6. Operator prerequisites before activation (not code)

Supplier tariffs and invoice evidence; Singapore licensing/registration for the
chosen supplier; ElevenLabs Scale+ plan and OEM terms; Telnyx maximum inbound leg
evidence and detail-record field verification from a paid account; managed-line
terms and End User Agreement text; underwriting budget sign-off.
