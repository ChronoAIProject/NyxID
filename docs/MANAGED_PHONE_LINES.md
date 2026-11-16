# NyxID-managed phone lines

NyxID resells an outcome, not vendor API access: an approved merchant claims a
NyxID-owned number, picks a NyxBot specialist to answer it and pays in NyxID
credits. Merchants never see a Telnyx, Twilio or ElevenLabs account, key or
webhook. The binding specification is
[docs/plans/2026-11-12-managed-comms-resale.md](plans/2026-11-12-managed-comms-resale.md);
routes are in [API.md](API.md#nyxid-managed-phone-lines). User-owned (BYOK)
carrier connections are described in [NYXBOT_SERVICE_ADAPTERS.md](NYXBOT_SERVICE_ADAPTERS.md).

Out of scope: number search/purchase and automated regulatory submission,
provider subaccounts, outbound calling, transfer/DTMF/media, Twilio premium
voice, hosted ElevenLabs agents, org-owned claims, raw vendor API resale and
refunds of paid periods.

## Authority model

- Each trusted provider account has exactly one canonical credential catalog
  service. Registration marks it and the line's billing services with a
  server-controlled `managed_comms_role` (`credential`, `rental`, `voice`,
  `sms`, `premium`). These must be dedicated private internal rows with no
  connections, grants, provider configuration or enabled `platform_key`.
- Only `call_channels::managed` decrypts the canonical credential, after the
  shared admission gate (owner, eligibility, claim, line, paid coverage, flags,
  account fence). This is the single documented exception to "platform-bound
  resolvers call the shared ACL". Generic HTTP/WS proxy, MCP, `/keys`
  provisioning, LLM gateway, connect links and catalog connect refuse managed
  services.
- Merchant execution rights are the claim. Numbers and applications stay in
  platform custody (`ResourceClaim`s owned by a line sentinel) for the line's
  whole life, including after retirement. Legacy allocate/verify/remove paths
  refuse custody resources.
- Admitted sessions and receipts store an immutable admitted reference. Hangup,
  status, detail-record and message reads of that exact effect keep working
  after release, revocation or erasure; they never authorize a new effect.
- While lines or unfinished effects depend on them, NyxID refuses account
  identity changes, credential-service rebinding, deactivation, unsafe policy
  or destination changes, enabling `platform_key`, and deletion. Credential
  rotation is permitted only through the verified rotation route.

## Operator activation checklist

Prerequisites (not code; record each as evidence before activation):

1. Supplier tariffs and invoice evidence for every charge you resell.
2. Licensing/registration for the number's country with the chosen supplier
   (Singapore for the pilot) and the SMS sender registration where offered.
3. ElevenLabs Scale, Business or Enterprise plan and the OEM minimum terms, if
   premium voices will be offered.
4. Telnyx maximum inbound leg evidence (written Telnyx confirmation, or an
   observed `time_limit` hangup on an inbound test call from a paid-tier
   account) and detail-record field verification (`/v2/detail_records/options`
   plus a recorded fixture) from a paid account.
5. Managed-line terms text and, for premium, the End User Agreement text, each
   with a version label.
6. Underwriting budget sign-off for every Telnyx account.

Activation:

1. Upgrade every API replica and worker. Declare and drain the roster for
   `nyxbot:managed-lines` ([SECURITY_ROLLOUT_READINESS.md](SECURITY_ROLLOUT_READINESS.md)).
   No new environment variables are required.
2. Create dedicated private internal catalog services: one credential service
   per provider account (encrypted master credential, fixed provider origin,
   operation policy) and separate price-only services for rental, voice, SMS and
   premium. A Telnyx credential service's operation policy must also allow
   `GET /v2/messages/{message_id}`, the exact-message read that settles
   outbound SMS parts whose delivery webhook never arrived; without it, those
   parts settle at zero after 72 hours. Author platform-lane prices
   (`phone_number_days`, `voice_seconds`, `requests`) and let them sync to
   Lago. Managed prices are required: missing, stale, retired or unsynced
   prices fail closed.
3. Trust the provider account (Admin → Platform Credentials → Trusted accounts).
4. Telnyx only: save underwriting and speech settings (below), then turn on the
   global premium switch if premium voices are offered.
5. **Register a line**: *Prepare* allocates the line ID and prints the stable
   URLs `/api/v1/webhooks/managed-lines/{id}/voice` and `/sms`. Configure them as
   the provider application's inbound URLs (POST); leave Twilio voice fallback
   empty; give Telnyx call-control applications a non-empty inbound channel
   limit. Then register with regulatory evidence, supplier tariffs, call-slot
   and rate limits. Registration proves everything with provider reads.
6. Save a cost policy per line (FX rates, attested detail-record schemas,
   supplier rental cycle) so reconciliation can produce actual margins.
7. Grant merchant eligibility; the merchant accepts the terms in NyxBot Settings.
8. Enable `nyxbot:service-adapters` and `nyxbot:managed-lines` for the merchant.
9. Perform a live paid smoke test (inbound call, SMS reply, caller confirmation)
   and confirm the report shows the effects.

CLI equivalents: `nyxid admin line prepare|register|update|list`,
`nyxid admin line eligibility grant|update|apply|revoke|list` and
`nyxid admin line account list|rotate|underwriting|premium|premium-switch|cost-policy`.
The line and account lists follow every page; a stored record the server could
not read is skipped, and its ID is printed as a one-line stderr warning (table
output) or returned in `unreadable_ids` (JSON) so it can be repaired.
Files are JSON or YAML without secrets; the rotation credential is read only
from `--credential-env` or `--credential-file`. Exact credit amounts
(underwriting `components[].credits_per_second`, `rolling_budget`) are quoted
decimal strings such as `"0.0002"`, and the cost-policy `inventory.cycle_anchor`
is an RFC 3339 time; numbers and BSON extended JSON are refused.

## Merchant flow

NyxBot Settings → **NyxID phone numbers** (shown when both flags are on), or
`nyxid nyxbot line …`.

- **Eligibility and terms**: the merchant sees approval status, limits and the
  required versions, and accepts them explicitly. Merchants cannot self-approve.
- **Get a NyxID number**: choosing an available line returns a 15-minute quote
  with the rental per 30 days and per-day rate, call price per second (and per
  minute), SMS price per part and, when allowed, the premium surcharge. The
  claim form selects the specialist, voice tier, standard voice and language,
  SMS, caller confirmations and auto-renew (off until the merchant turns it on),
  and requires explicit consent to the prices and to each terms version
  (including the ElevenLabs End User Agreement for premium). The first period is
  charged on confirmation and is not refunded.
- **Request IDs**: each claim, renewal, resume and release carries a UUID v4
  request ID. The web keeps one per dialog; the CLI prints it before sending.
  Retrying with the same ID replays the original outcome.
- **Line detail**: state, paid coverage `[covered_from, covered_until)`,
  specialist, voice (changeable only when no call is live), SMS, caller
  confirmations toggle, automatic renewal, renewal quote, a link to Billing &
  Usage, and Release.
- **Renewal**: from 72 hours before `covered_until` the sweep issues one renewal
  quote and notifies once. With unchanged price and terms and auto-renew
  consent, the next period is funded before the cutoff. A changed price needs
  the merchant to accept a renewal quote. There is no unfunded grace.
- **Unpaid**: without a funded period at the cutoff the claim becomes
  `suspended{unpaid}`: calls are rejected before answer without charge, inbound
  SMS still records liability. Resume within seven days with a resume quote;
  the new period starts now and suspended time is never charged. After seven
  days the claim is released.
- **Release** stops new effects immediately; the line enters a cooldown of at
  least 30 days once every effect is accounted, then waits for operator
  reassignment approval.

## Billing units

| Charge | Metric | Billing identity | Notes |
| --- | --- | --- | --- |
| Line rental | `phone_number_days` | `managed-line-rental:{claim_id}:{period_index}` | 30 number-days per prepaid period, funded with the period receipt in one transaction through the managed lifecycle contract. |
| Calls | `voice_seconds` | `managed-call:{session_id}:base` | 1,800 s reserved before answer; `billable = min(provider seconds, 1800)`; the excess is absorbed. |
| Premium voice | `voice_seconds` (premium service) | `managed-call:{session_id}:premium` | Second meter in the same admission transaction, settled on the same billable seconds. |
| SMS | `requests` per provider part | `managed-inbound:{liability_id}:{part}`, replies `managed-sms-out:{session_id}:{part}`, `call-confirmation:{operation_id}:{part}` | Inbound parts are recognized as incurred; each part is either customer-funded or absorbed, never both. Managed meters are never waived by stuck-request recovery. |

Funding follows allowances → grants → wallet with tax, ledger and Lago like
other usage. The optional monthly managed spend limit (eligibility) counts
pre-tax managed retail charges by UTC month of admission, including
benefit-funded shares; model and tool charges are separate. Exhausted spend or
underwriting capacity returns 13400 `managed_budget_exhausted` (HTTP 429).
See [BILLING_EXACT_ACCOUNTING.md](BILLING_EXACT_ACCOUNTING.md) and
[BILLING_OPERATIONS.md](BILLING_OPERATIONS.md).

## Telnyx underwriting

Telnyx voice is an **underwritten exception, not reserved liability**: Telnyx
cannot cap an inbound leg, so NyxID absorbs supplier cost beyond the 1,800-second
retail limit within a per-account rolling 30-day budget. Manual `Platform`
allocations of Telnyx voice remain refused.

The policy records the accepting admin and time, acceptance text and version,
paid-tier evidence, maximum inbound leg seconds with its evidence, verified
detail-record fields, supplier tariff revision, frozen FX policy revision,
per-second exposure components (carrier inbound, relay, STT) and the
character-priced synthesis tariff. Any missing item keeps the refusal.

- Per-call worst case = `(max_leg_seconds − deadline_seconds) × Σ per-second
  components`, with supplier increments and minimums rounded up.
- Admission holds the worst case; settlement replaces it with the recognized
  absorbed cost. A loss above its hold is recognized in full and blocks new
  admissions until capacity returns.
- Enablement, registration, line edits and tariff/budget changes refuse when
  Σ channel limits × worst case exceeds the budget.
- The hangup deadline is `min(1,740 s, funded horizon − 60 s)` from the answer
  dispatch; any replica's recovery sweep enforces it. Hangups are retried with
  the same derived command ID until confirmed.

Telnyx quantity evidence: the call-status API, then authenticated
`call.answered`/`call.hangup` occurrence times (managed lines only), then a
validated `call-control` detail record when enabled. Missing evidence keeps the
call `terminated_quantity_pending`; it never becomes zero or elapsed time. An
audited admin evidence resolution is the only manual path.

## Speech, premium voice and the character allowance

- NyxID speaks no greeting unless one is configured per account.
- Every Telnyx managed call has a character allowance (default 12,000 Unicode
  scalar values). The writer pre-debits 400-character blocks before writing
  text and writes only within debited blocks; uncertain writes stay consumed
  and are never replayed; only provably unwritten text is credited back.
- 300 characters are reserved for the wrap-up, which is localized for en-US,
  zh-CN and zh-TW (or an account override of at most 300 characters). When the
  allowance reaches the reserve, the wrap-up is spoken and the call ends.
- Premium ElevenLabs voices (Telnyx relay only; Twilio stays on Google Standard)
  require `premium_allowed`, the accepted End User Agreement version, the
  account's premium configuration and the global switch. The ElevenLabs key is a
  Telnyx integration secret; NyxID proves the identifier exists and stores only
  the reference, allowed voices, billing identity, plan tier and OEM terms
  version. A monthly character budget per ElevenLabs billing identity is shared
  across Telnyx accounts.
- Turning the global switch off stops premium synthesis at the next block: the
  call speaks its wrap-up and ends. Falling back to a standard voice happens only
  when relay start was refused before any premium text left NyxID; the premium
  meter is then released.

## Caller confirmations

The owner toggles caller confirmations per voice binding (default off; managed
lines need the claim's SMS binding). The specialist tool
`call_channel__text_current_caller` exists only inside an admitted voice-call
turn on that binding. The destination is the carrier-verified caller of the
current call, the sender is the line, text is at most 480 UTF-8 bytes and each
call allows at most two sends. Sends are durable operations keyed by session,
tool invocation and ordinal; uncertain sends consume their slot and are never
retried. `caller_consented` is recorded as model-asserted consent. Audit is
metadata only.

## Outbound SMS settlement

Replies (`managed-sms-out:`) and caller confirmations (`call-confirmation:`,
managed or owner-funded) reserve one meter per provider part before admission;
admission forwards every part in its own transaction. Provably unsent parts are
released. A part dispatched with an unknown outcome stays held until provider
evidence arrives: a delivery callback, or a read of the exact provider message
(fixed origin, canonical credential, no redirects, bounded response) when its
message ID is known. The Telnyx read (`GET /v2/messages/{message_id}`) also
requires the credential service's operation policy to allow it and settles on
the message's `parts` and recipient status, as the signed `message.finalized`
webhook does; the Twilio read uses `num_segments`. Without provider-confirmed
evidence 72 hours after dispatch, the part settles at zero and its send receipt
and cost evidence record `terminal_evidence = "provider_evidence_unavailable"`.
The supplier cost of such a part is unknown and absorbed in the report, the
settlement is audited metadata-only, and a callback arriving afterwards records
delivery but never charges. The send still consumes its slot and is never
retried.

## Inbound SMS and routing

Every authenticated inbound SMS first records a liability row keyed by trusted
account and provider message ID, even with no claim, no coverage, flags off or
an erased owner. Only customer-funded messages for an eligible current claimant
reach an agent; otherwise content is discarded. Callbacks for existing effects
resolve through metadata-only effect-route tombstones and never reach a later
claimant. Unseen events older than the current claim, or ambiguous ones, are
quarantined.

## Reconciliation and the report

Cost evidence is captured per effect with component coverage and evidence
quality kept separate: margins are **estimated** whenever any component is a
tariff estimate or unknown, and **actual** only when every component is a
reconciled provider record. Supplier rental is recorded per line per supplier
billing cycle in every line state until the supplier release is recorded.
Capture never requires readiness.

Admin → Platform Credentials → NyxID-managed phone lines links to the
**Managed lines report** (`/admin/managed-lines/report`; admin or operator), and
the Integrity page shows pending cost capture and lines without a rental policy.
CLI: `nyxid admin line report --from YYYY-MM-DD --until YYYY-MM-DD [--line ID]
[--effects]`.

## Operations

| Task | Web (Admin) | CLI |
| --- | --- | --- |
| Edit limits, regulatory evidence, tariffs; revalidate provider and capacity | Edit and revalidate | `nyxid admin line update LINE --revision N --file f.yaml` |
| Suspend; requests hangups and reports pending termination | Suspend | `nyxid admin line suspend LINE --revision N` |
| Resume an admin suspension | Resume | `nyxid admin line resume LINE --revision N` |
| Retire unclaimed inventory / record supplier release | Retire / Record supplier release | `nyxid admin line retire LINE --revision N [--supplier-released-at T]` |
| Return a cooled-down line to inventory | Approve reassignment | `nyxid admin line approve-reassign LINE --revision N` |
| Release a merchant's claim | Release claim | `nyxid admin line release LINE --yes` |
| Standard voice allowlist | Standard voices | `nyxid admin line voices LINE --revision N --option VOICE:LANGUAGE` |
| Resolve permanently missing call evidence | Report → Effects → Resolve evidence | `nyxid admin line resolve-evidence SESSION --provider-seconds S --provider-reference R` |
| Rotate an account credential | Rotate credential | `nyxid admin line account rotate ACCOUNT --revision N --credential-env VAR` |
| Find stored records that could not be read | Warning banner in Inventory / Accounts | `nyxid admin line list` / `nyxid admin line account list` (stderr warning; `unreadable_ids` in JSON) |

Suspension and retirement only reduce authority and work while readiness is
unavailable. Retired lines stop counting against Telnyx underwriting capacity
(suspended, cooling-down and pending lines still count because they can carry
calls again). Credential rotation re-proves every line whose number the supplier
still holds, retired ones included, and skips supplier-released numbers; when
none remain it proves the provider account identity instead (Telnyx
`/v2/user_profile`, Twilio `/2010-04-01/Accounts/{sid}.json`, which the
credential service's operation policy must allow). A suspended claim's paid period keeps running and is not renewed;
resuming after its coverage ended leaves it `suspended{unpaid}` with a fresh
seven-day retention window. Editing a line advances its revision, which
invalidates outstanding claim quotes; an accepted automatic renewal is not
affected.

## Rollout and rollback

- Flag off: no new claims, renewals or call/SMS admissions on managed lines;
  inbound SMS still records liability; callbacks, settlement, cleanup and cost
  capture continue. Claims lapse at their cutoff into `suspended{unpaid}`
  without charge for the off period.
- New fields and enum values are serde-defaulted where possible. Upgrade every
  API replica and worker before registering lines or authoring managed prices.
- There is no binary rollback after managed financial rows exist unless a
  reviewed compatibility reader ships; flag-off is the operational stop.

## Troubleshooting

| Symptom | Cause and fix |
| --- | --- |
| `409` "Managed phone line admission unavailable" on quote or claim | Eligibility not approved/revoked, terms not accepted, line not available or still cooling down, flag or readiness off, or a stale quote. Request a new quote. |
| Claim refused after an admin edit | The line revision advanced; quotes bind it. Request a new quote. |
| "Provider channel limit changed; operator revalidation required" | Telnyx channel limit changed at the provider. Use *Edit and revalidate*; underwriting capacity is rechecked. |
| "Provider inbound URL must equal the stable managed line URL" | Configure the exact stable URL (POST) on the provider application and clear Telnyx failover URLs that differ. |
| Calls rejected before answer | No eligible funded claim, no free call slot, rate limit, spend or underwriting budget exhausted (13400), suspension, or flag off. |
| Premium voice not offered | Eligibility lacks `premium_allowed` or the End User Agreement version, the account has no premium configuration, or the global switch is off. |
| Call stuck in pending quantity | Provider evidence unavailable. Recovery keeps reading; resolve with referenced provider evidence if it is permanently missing. |
| Report shows only estimated margin | Some components are tariff estimates or unknown; save attested detail-record schemas and FX rates in the line's cost policy. |
