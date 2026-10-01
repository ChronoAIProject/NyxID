# NyxBot schedules and triggers: agents that act on their own

Status: implemented contract. The original request and S1–S8 decisions are retained
below; implementation details, test coverage and measurements follow them.

## What the user asked for

> can we also add scheduler so nyxbot can be trigger to do something, so nyxbot
> as usual should be able to help user to set this and can set for specialise
> agent or nyxbot itself, and we have triggers in nyxid not sure if we can use
> that with scheduler, basically nyxbot which is the nyx assistant whole system
> will be a competitor to muse/grok bot and dots etc with all this feature in
> place, we need to ensure the performance as well

NyxBot and its specialists can be set to act by themselves on a schedule (e.g.
"every weekday at 8:00 summarise my inbox and send it to me on Telegram",
"every Friday run the test suite on my machine and report failures", "remind me
in 2 hours"), or when an external event arrives on a webhook ("when GitHub
calls, have the coder specialist triage the issue"). The owner sets these up by
asking NyxBot, or on the web. This builds on NyxID **Triggers** rather than
adding a parallel system.

Nothing here may break existing triggers (webhook ingress, webhook, device agent
and notification deliveries, redelivery, rotation), NyxBot, specialists, channel
bots or billing.

## Existing pieces to build on (read first)

- **Triggers:** `backend/src/models/trigger.rs`,
  `backend/src/services/trigger_service.rs`, `backend/src/handlers/` trigger
  routes, CLAUDE.md "Connection Webhooks and Triggers". Today a trigger is an
  inbound webhook (token or HMAC verified) delivered to a webhook URL, a device
  agent conversation, or a notification.
- **NyxBot event turns:**
  - `assistant_team_service` / `handlers::assistant_team`: `pending_events`,
    `AgentEvent`, the owner turn pool, and the guards `EVENT_TURNS_PER_HOUR` (20)
    and `MAX_EVENT_STREAK` (3 consecutive event turns per NyxBot thread without
    a user message);
  - `nyxbot_watches` and the live change stream (`services/assistant_live.rs`);
  - `reply_channel` and `nyxid__post_to_chat` for delivering to chat apps;
  - docs `docs/chat/08-nyxagent-engine.md` and
    `docs/chat/09-nyxbot-orchestrator.md`.
- **Exactly-once periodic work:** billing credit schedules
  (`services/billing/schedules*`): UUID v5 identity per `{schedule, period}`,
  claimed periods, leases as derived progress, and catch-up that mints only the
  current window.

## Decisions

### S1. One trigger model: a source and a delivery

A trigger gains a **source**, next to its existing delivery:

| Source | Meaning |
|---|---|
| `webhook` (existing, the default for existing rows) | An inbound HTTP event at `/webhooks/triggers/{id}`, verified as today |
| `schedule` (new) | Fires on a schedule (S2); no ingress URL and no inbound secret |

And a new **delivery**:

| Delivery | Meaning |
|---|---|
| `assistant` (new) | Wake a NyxBot agent (NyxBot or a specialist) with the trigger's instruction, plus the event payload for webhook triggers |

- **Existing deliveries** (webhook, device agent, notification) keep working
  and may be combined with a `schedule` source where that makes sense. A
  scheduled notification is a plain reminder, delivered as a notification.
- **Compatibility.** Existing trigger documents deserialize unchanged: an
  absent source means `webhook`. Model changes are additive and serde-defaulted.
  Replicas that predate this must not misread new rows. Choose representations
  so an old replica ignores schedule triggers, fails closed, or cannot reach
  them; document which.

### S2. Schedules

- **Kinds:**
  - `cron`: a 5-field cron expression with an IANA timezone;
  - `every`: an interval (minutes, hours or days) with an anchor;
  - `at`: a one-off time.
- **Options:** an optional start, end and maximum number of runs; paused or
  active.
- **Validation.** Specs are validated server-side (the timezone exists, cron
  parses, and the minimum interval is 5 minutes by default, owner-configurable
  upward only), and DST is handled correctly: no duplicate or skipped
  wall-clock runs beyond the documented cron semantics. Every create or update
  response includes the next three run times in the owner's timezone, so NyxBot
  can confirm them in plain words.
- **Limits.** At most 100 schedules per owner. A disabled or deleted target
  agent pauses the schedule, with a visible reason.
- **Missed runs.** If NyxID was down or the target was busy past the window,
  run the missed occurrence once when it recovers, but only within a grace
  window (default: the smaller of the interval and 1 hour). Otherwise skip it
  and record the skip. Never a burst of catch-up runs.

### S3. Running a trigger for an agent

- **Target.** A NyxBot agent (NyxBot or a live specialist) and a
  **thread** policy:
  - `home`: the agent's home thread (default for scheduled NyxBot work);
  - `dedicated`: one thread per trigger, titled after the trigger; the default
    for specialists and for every webhook-sourced assistant delivery;
  - `new`: a new thread per run, for independent runs; old run threads stay
    listable, and retention follows existing conversation rules.
  Webhook `home` is an explicit owner choice with UI and tool warnings: untrusted
  event text persists into later full-authority owner turns, including private
  channel chats. The webhook run's confirmation policy does not protect those
  later turns. One source-aware helper resolves defaults for ingress and scheduler
  thread selection. Dedicated/per-run automation threads are excluded from home
  adoption at creation, turn admission and recovery after home deletion. The
  private setup prefill carries the owner's thread choice.
- **Instruction.** A stored instruction text (≤ 8 KB), written as if the owner
  had asked. The turn is told it was started by trigger `<label>` at `<time>`,
  and for webhooks it gets the event payload (bounded, untrusted, marked as
  such, never treated as instructions).
- **Authority.** Runs act with the owner's identity, billing and the target
  agent's normal authority: NyxBot's full access, or the specialist's grants,
  machines and saved logins. It is exactly as if the owner had sent the message
  now; there is no guest mode for triggers.
- **Confirmations.** Scheduled runs retain normal owner confirmations. Webhook
  assistant runs default to `confirmation_policy = changes`, requiring an owner
  action card for every changing call. HTTP semantics, NyxID markers and catalog
  contracts share the guest classifier; there are no tool-name word lists.
  The owner may explicitly choose `destructive` after a warning, permitting
  ordinary changes without a card. The run waits for the owner and notifies them;
  this policy cannot be bypassed by the global skip-destructive setting.
- **Delivery of results.** Optional `deliver_to`:
  - a chat of one of the owner's channel bots (by chat ID from
    `nyxid__list_channel_chats`, only chats the owner may post to);
  - a push notification;
  - the web thread only (default).

  The run's final reply is delivered there, reusing
  `reply_channel`/`post_to_chat` mechanics.
- **Guards.**
  - Scheduled and webhook runs are **not** counted against `MAX_EVENT_STREAK`,
    otherwise a daily schedule would stop after three days.
  - They have their own per-owner budget (default 30 runs/hour and 300/day,
    owner-configurable within bounds) and a per-trigger overlap policy: `skip`
    (default) if the previous run of that trigger is still active, or `queue`
    one.
  - They use the owner turn pool as today; when it is full, a run waits its
    grace window, then is recorded as skipped.
  - Budget exhaustion pauses runs, not the triggers, and tells the owner once.
- **Webhook triggers to agents** reuse existing verification, rate limiting,
  dedup claims and payload limits. Event payloads are never persisted beyond
  what the turn transcript naturally stores (existing ADR-013 rules).

### S4. Exactly-once runs, safe across replicas

- **The worker.** A leased, fenced scheduler worker per replica claims due
  occurrences with an indexed query on the next due time, never a scan.
- **Run identity.** Each occurrence's identity is a UUID v5 of
  `{trigger_id}:{scheduled_at_ms}`; the run record's unique `_id` makes a run
  happen at most once across replicas and restarts.
- **Next run.** Computed and stored atomically with the claim.
- **Recovery.**
  - A replica dying mid-claim leaves a lease that expires.
  - A run that started a turn is not started again. The turn's own
    `turn_lost` recovery applies.
- **Run history.** Every run is recorded with scheduled time, started, outcome
  (started, completed, failed, skipped with reason), thread ID and duration. It
  is metadata only, bounded per trigger and TTL-expired (e.g. 90 days).

### S5. NyxBot sets it up (a breeze)

- **Tools (NyxBot-only team tools):**
  - `nyxid__create_schedule`: target agent, thread policy, instruction,
    schedule (NyxBot turns the owner's words into cron, interval or at, with the
    owner's timezone from settings or the browser), optional `deliver_to`,
    overlap and limits. It returns the next runs, for NyxBot to read back in
    plain words ("every weekday at 8:00 Singapore time, starting tomorrow").
  - `nyxid__list_schedules`, `nyxid__update_schedule` (including pause and
    resume) and `nyxid__delete_schedule`. Listing and updating also expose
    assistant webhook automations and their confirmation policy.
  - `nyxid__run_schedule_now`: run once now, to test.
- **Webhook triggers to agents.** Creating one needs its inbound secret shown
  exactly once. Secrets never pass through chat, so NyxBot hands out a
  prefilled Automations page link (`nyxid__settings_link` area `automations`,
  extended with a server-stored, owner-authenticated setup watch). There the owner sees the URL and secret
  once. The page is watched, so NyxBot is told when the trigger exists and can
  explain how to connect it (e.g. the GitHub webhook settings).
- **Timezone.** Owners have a timezone: an existing setting if there is one,
  otherwise an additive `timezone` in NyxBot settings, defaulted from the
  browser. NyxBot asks when it is unknown and schedules are involved.
- **Instructions.** NyxBot's instructions explain schedules and triggers
  briefly: when to offer them, confirming times, and preferring `deliver_to`
  for things the owner wants pushed. Specialists ask NyxBot rather than
  managing schedules themselves.
- **Guests.** Guests can never create, see or change triggers (`nyxid__`
  tools are refused for guests today; keep it so).

### S6. Web

- **Automations page.** One page lists every trigger (scheduled and webhook)
  with target, next run, last run outcome, and pause/resume, edit, run now and
  delete. The existing Triggers page becomes or links to it.
- **Create/edit form.** Source (schedule or webhook), schedule builder
  (presets plus a cron field with a human-readable preview and next runs),
  target agent and thread policy, instruction, delivery, and limits.
- **Run history per trigger,** linking to the thread of each run.
- **Agent pages** show that agent's schedules.
- **Owner-only.** Human routes for creating, editing and deleting from the web;
  agent keys keep today's trigger API rules (CLAUDE.md).

### S7. Performance (applies to everything here, and is measured)

The user: "we need to ensure the performance as well". This PR must show, not
assume, that schedules and triggers do not slow NyxID or NyxBot down:

- **Scheduler cost.**
  - The due-occurrence claim uses a covering index. Record the explain plan in
    a test or the doc.
  - With 10,000 active schedules, a scheduler tick costs O(due) queries, not
    O(schedules).
  - Idle ticks are one cheap indexed query.
  - Measure and report.
- **No hot-path regressions.** Proxy, MCP `tools/list`/`tools/call`, turn start
  and channel inbound paths gain no extra database round trips for owners
  without triggers. For owners with triggers, list the added queries and keep
  them indexed.
- **Turn start.** A due run starts its turn within 5 s of its scheduled time
  under normal load; measure scheduled-to-turn-started latency in a test with
  a fake NyxAgent.
- **Benchmarks.** Add repeatable timing tests or benchmarks for the claim
  query at 10k schedules, next-run computation, and the Automations list API
  at 100 schedules. Report the numbers in the final report and in this
  document.

### S8. Audit, errors, docs

- **Audit.** Metadata-only audit for trigger create/update/delete, pause and
  resume, and each run's outcome. No instruction text or payloads in audit or
  logs.
- **Errors.** Reuse the trigger error block (11600–11606). Add codes there, or
  in a new reserved block, and update CLAUDE.md's table.
- **Docs.** Update CLAUDE.md (Triggers rule, NyxAgent paragraph, key routes),
  `docs/chat/09-nyxbot-orchestrator.md`, and this document.

## Acceptance

- **Scheduled task.** In the app and in a linked chat app, "every weekday at
  8am summarise my GitHub notifications and send it to me on Telegram"
  creates a schedule. NyxBot reads back the next runs, the run happens at 8:00
  in the owner's timezone, the summary arrives in the Telegram chat, and the
  run appears in history.
- **Specialists.** The same works for a specialist target, with its grants
  only.
- **Webhook triggers.** A webhook trigger to a specialist wakes it with the
  payload and runs the instruction.
- **Reliability.** A daily schedule runs every day, unaffected by
  `MAX_EVENT_STREAK`. Missed runs follow the grace policy. Two replicas never
  run an occurrence twice.
- **Controls.** Pause, resume, edit, run now and delete work from chat and web.
- **Performance.** The numbers are reported.
- **Tests.** Every decision has tests:
  - cron, interval and one-off computation, including DST transitions;
  - validation and limits;
  - exactly-once claims across concurrent workers;
  - lease expiry;
  - missed-run grace;
  - overlap policy;
  - budgets;
  - authority (NyxBot, specialist grants, no guests);
  - `deliver_to` chat and notification;
  - webhook-to-agent;
  - compatibility with existing trigger rows;
  - the old-replica representation;
  - frontend forms and preview;
  - the benchmarks.
- **Existing tests.** The existing trigger, NyxBot, channel and billing suites
  stay green. Run `cargo fmt`, `clippy -D warnings` (Rust 1.98.1), the CLI
  tests if touched, and the frontend lint, tests and build.

## Implementation and operations

The API extends `/api/v1/triggers`; there is no parallel scheduler API. Create
accepts `source`, `schedule`, `overlap` and the tagged `assistant` delivery. Patch
changes authored configuration, status and limits without replacing runtime
progress. Source is immutable. `POST /triggers/preview` returns a description and
next three times; create/patch also return `next_runs`, `next_run_at`, the last
terminal run and any pause reason. `POST /triggers/{id}/run` admits an immediate
test occurrence without moving the schedule. `GET /triggers/{id}/runs` returns
100 metadata rows per page and a `(before, before_id)` cursor, preserving ties.
The web UI exposes older pages and thread links. Human web management and
NyxBot-only tools share trigger validation and persistence.

Examples of schedule specifications:

```json
{"kind":"cron","expression":"0 8 * * 1-5","timezone":"Asia/Singapore"}
{"kind":"every","amount":5,"unit":"minutes","anchor":"2026-10-01T00:00:00Z"}
{"kind":"at","at":"2026-10-01T12:00:00+08:00"}
```

Optional fields on each specification are `start`, `end`, `max_runs`, and
`grace_seconds`. API instants require RFC3339 offsets; storage uses UTC BSON dates.
Start/end are inclusive. Missing cron wall times fire at the first valid instant
after a DST gap. Several missing times and a real match there collapse into one
occurrence. A repeated wall time uses its earlier instant; intervals measure elapsed UTC time.
The minimum interval is 5–1,440 minutes in owner settings. Explicit grace is
1–86,400 seconds; default grace uses the underlying cadence even at the final
occurrence. A one-off defaults to one hour. `max_runs` (1–1,000,000) counts selected
scheduled occurrence attempts, including a selected occurrence that eventually
skips or fails; coalesced older missed times and manual test runs do not consume
that limit. Editing or resuming preserves this counter. Raising the maximum can
allow more occurrences. A completed one-off has no next time.

There are at most 100 schedule records per owner, including paused/completed
ones. Creation serializes on the owner's limit document inside a transaction.
Destroying the target, disabling its assistant engine, or deactivating its owner
pauses future work with a reason. A database/transient lookup error defers work
instead of disabling the trigger. Owner timezone is an additive NyxBot setting;
`owner_timezone` on create_schedule saves an explicitly supplied answer when
NyxBot asks. Web settings default the field from the browser.
The list retains one denormalized last-outcome snapshot on each trigger so it
needs no per-row history queries; that status snapshot lasts until a later run
or trigger deletion. The paged run-history collection itself has the 90-day TTL.

Each replica ticks once per second, selects at most 100 jobs, and processes at
most 16 concurrently. Work discovery is one covered index query over `at` and
`_id`. A CAS moves the job 30 seconds forward and assigns a random fence. Schedule
advancement, the UUID-v5 occurrence row and its work item commit together. Turn
admission validates the current fence, authored trigger revision, owner, overlap
and run budget inside `begin_turn`'s existing transaction. Run and conversation
writes cannot be separated by a crash. There is no replay after admission; the
existing turn TTL and transcript determine completion versus `turn_lost`.
Pre-admission skips and automatic pauses are fenced too.

Recovery selects only the newest missed occurrence for possible execution and
records earlier times as one `missed_window` summary with count, first and last
missed instants, and one `trigger_occurrences_missed` audit event. It never launches a
catch-up burst. Metadata history retains at most 1,000 rows per trigger and has
a 90-day TTL index. Webhook occurrences additionally use the existing fenced
webhook dedup claim and a stable UUID derived from trigger/event ID. Their run,
transcript event and work item are committed together; no payload is stored in
work, run or audit collections. Setup watches store the owner-authored prefill,
never webhook payloads.

The default owner budget is 30 initial run admissions per UTC hour and 300 per
UTC day, configurable to 1–300/hour and 1–3,000/day. Only admitted runs consume
budget. Busy threads, full pools and queued overlap back off from one to 30
seconds, capped at the grace deadline. Exhausted budgets wait for the next UTC
hour/day window, also capped at that deadline. Expiry records a visible skip. Budget notices are deduplicated per exhausted owner/hour or owner/day
window; daily exhaustion does not repeat its notice at each hourly reset. `skip` admits no additional
outstanding work; `queue` keeps one successor, ordered by scheduled time and ID.
Waiting confirmations count as active for overlap. Confirmation continuation
uses the same run and does not debit the occurrence budget again. Waiting work
is due at its earliest pending card expiry, not every second. Deciding a card
wakes the work row in the same transaction; settlement rereads card state inside
its transaction so a simultaneous decision cannot lose a wakeup.
Changing away from assistant delivery while a run awaits confirmation ends that
run with `target_changed` after the pending decision is resolved.

Run outcomes are `pending`, `started`, `waiting`, `completed`, `failed`, or
`skipped`. Run records hold identifiers, timing, confirmation IDs, destination
metadata and outcome only. The complete answer lives in the assistant transcript.
`deliver_to` is `{"type":"thread"}`, `{"type":"notification"}` (push only), or
`{"type":"chat","chat_id":"..."}`. Chat access is checked both at configuration
and delivery, through the existing post-to-chat path. Long replies respect the
destination platform limit and explicitly say they were shortened, with a full
thread link. Push has a concise preview and thread ID.
Final delivery is claimed once. If a replica dies around a remote send, recovery
reports `result_delivery_interrupted` rather than duplicating an ambiguous send.
Schedule-to-webhook delivery retains the existing signed delivery record and
optional encrypted replay envelope. Plain notification/device deliveries use
their existing transport paths.

Rolling compatibility fails closed: absent source still means webhook; new
schedule rows use the unknown-to-old-replicas `verification.mode = schedule`,
and assistant delivery is also a new enum variant. Old replicas cannot deliver
these rows: the entire owner listing and direct row loads (including assistant
webhook ingress) fail deserialization with an internal error. This is request
failure, not a safe per-row omission or a 404. Trigger-origin
assistant state also requires the new binary. Upgrade every replica before
creating these automations. No old trigger polling/retry sweep scans these
rows: outbound retries run only for a webhook record already admitted by that
replica, and manual replay first loads its trigger and fails before mutation.
Retention is a MongoDB TTL index on `trigger_deliveries`, not a trigger deletion
sweep. Old replicas neither crash-loop nor delete automation rows. New replicas
read legacy and automation rows together. Existing webhook-only rows retain their wire and
storage contracts, secrets, HMAC verification, rotation and replay behavior.

Webhook assistant deliveries have `confirmation_policy`: `changes` (default)
requires an owner action card for every changing call; `destructive` permits
other changes and requires explicit owner selection with a warning. Service
classification shares guest access's HTTP method and catalog contract rules,
including NyxID markers; names and payload instructions never authorize changes.
Native tools use their closed effect inventory. Policy is snapshotted at run
admission and persists across confirmation continuations. Scheduled turns retain
normal owner authority. The global skip-destructive preference cannot bypass a
webhook policy. NyxBot tools expose and explain this choice.

Automation REST operations (including previews, setup prefills, runs and list
filtering) and timezone/budget settings use the shared first-party human check.
All `/assistant/nyxagent/*` routes reject developer OAuth client access tokens,
alongside API-key, delegated, relay and service-account credentials. Web sessions,
CLI device login and mobile first-party access tokens have no OAuth client ID.

Webhook setup links are `/assistant/automations?setup=<watch-id>`. The owner-authenticated
`GET /triggers/setup/{id}` returns label, instruction, agent and policy only for
an owned, pending, unexpired, unused watch. Creation validates and consumes the
watch in the trigger-insert transaction, so concurrent submissions cannot reuse
it. No instruction or label enters the URL. The Triggers developer page remains
available for inbound secrets and replay; Automations sits next to Assistant.

## Performance measurements

Repeat with Rust 1.98.1 and the replica-set MongoDB at port 27020:

```sh
NYXID_TEST_DATABASE_URL="mongodb://127.0.0.1:27020/?directConnection=true" \
  cargo +1.98.1 test -p nyxid --bin nyxid-server -- \
  trigger_schedule --test-threads 2 --nocapture
```

Recorded on the local macOS development machine, unoptimized test build. Timings
are observations, not a production capacity guarantee. The fixture inserts
10,000 active schedule documents and their derived work, with 100 belonging to
the list-request owner. Database round trips and BSON work are included. The
list measurement calls the API handler and includes response JSON encoding;
it excludes HTTP socket/TLS and authentication middleware overhead:

| Measurement | Result | Sample |
| --- | ---: | --- |
| Covered due discovery, 10,000 schedules / 1 due | 0.435 ms/query | 100 queries |
| Fenced CAS claim, 10,000 schedules | 4.279 ms/claim | 100 claims |
| Idle tick, 10,000 schedules / 0 due | 0.418 ms/tick | 100 ticks |
| Automations list API, 100 interval schedules + previews + JSON encoding | 9.508 ms/request | 20 requests |
| Next cron occurrence, weekday 08:00 Singapore | 91.361 µs/op | 10,000 computations |
| Scheduled time → committed turn admission | 246 ms | fake NyxAgent; two concurrent replica ticks |
| Confirmation decision → completed continuation | 487.193 ms | persisted owner decision, scheduler tick, fake NyxAgent |
| Three-day outage, five-minute cadence | 111.843 ms | 864 missed occurrences; one execution, one summary, one summary audit |

The 26 targeted scheduler tests that produce these numbers pass in 17.88 seconds.

The explain test asserts `PROJECTION_COVERED → IXSCAN`, index
`trigger_work_due {at:1,_id:1}`, `totalDocsExamined = 0`, and
`totalKeysExamined = 1` for the one-due fixture. Discovery never scans trigger
records. The mutating CAS fetches the selected job by `_id`; a write itself
cannot be a covered read. Idle ticks issue exactly one query. Active work costs
a bounded set of queries/transaction writes per due occurrence, independent of
idle schedules. Downtime adds one summary insert and one audit, regardless of
missed count. History checks a projected per-trigger counter; only excess history
opens a bounded trim transaction with one delete operation.

Proxy and channel inbound gain no scheduler lookup. Ordinary MCP calls gain no
scheduler lookup; a trigger chat key adds one indexed run lookup to recover the
snapshotted webhook policy. Discovery advertises the optional action-card ID;
execution strips it before preparing a downstream request. Ordinary `begin_turn` only tests an in-memory optional claim; ordinary settlement
only tests `active_turn.trigger_run_id`. Thus owners without triggers gain zero
database round trips on these paths. For a trigger turn, additional reads/writes
are by job/run/trigger/owner `_id`, the trigger overlap index, existing agent/thread
indexes, and owner settings/budget `_id`. Listing adds one owner-settings lookup
and computes previews locally; history uses `{trigger_id,scheduled_at,_id}`.
The existing metadata-only assistant change stream adds watched trigger inserts;
it does not create another stream or ingress query.

## Acceptance coverage

| Decision / acceptance | Tests |
| --- | --- |
| S1 legacy documents and old replicas | `models::trigger::compatibility_tests::old_documents_default_to_webhook_and_old_replicas_fail_closed`; existing trigger service/ingress/history tests |
| S2 cron, every, at, DST, limits and grace | `trigger_schedule::compute::tests` including spring gap, fall fold, sparse date cadence, final-interval grace and validation; `schedule_tools_controls_limits_and_owner_timezone` races creation at the 100-record limit |
| S3 normal NyxBot/specialist authority, guests, thread policies | `schedule_fake_agent_latency_authority_and_event_streak`; `schedule_specialist_authority_threads_and_guest_refusal`; existing agent credential/grant suites |
| S3 confirmations and destinations | `schedule_confirmation_waits_and_resumes_without_another_budget` also covers changed delivery while waiting; `schedule_result_delivers_to_telegram_and_push_with_live_acl` uses local TLS with actual Telegram/FCM serializers and verifies one budget notice per exhausted hour/day; frontend continuation tests cover scheduler-owned cards |
| S3 budgets, overlap and pool | `schedule_missed_grace_overlap_and_budgets`; `schedule_queue_one_busy_grace_and_edit_fence`; `schedule_full_owner_pool_waits_and_expires_without_charging_budget` |
| S4 exactly-once admission / recovery | concurrent replica latency test; `schedule_lease_expiry_and_duplicate_claim_are_fenced`; `schedule_long_downtime_records_skips_without_a_catchup_burst`; `schedule_started_turn_is_never_replayed_and_audit_is_metadata_only`; revision-fence and queue tests |
| S5 chat controls and webhook setup | `schedule_tools_controls_limits_and_owner_timezone`; `schedule_webhook_prefill_watch_and_human_api_boundary`; `schedule_webhook_to_specialist_is_atomic_deduplicated_and_untrusted`; team-tool authority tests |
| S6 forms and previews | `frontend/src/pages/automations.test.tsx` covers create, preview, webhook prefill/one-time secrets, edit, pause/resume, run now, deletion, history paging/thread links, invalid preferences and late-loaded owner timezones without overwriting edits; `schemas/automations.test.ts`; existing Triggers, NyxBot settings/details and confirmation tests |
| S7 measurements | `schedule_benchmark_10000_covering_discovery_and_100_list`, `next_run_timing_benchmark`, concurrent fake-NyxAgent latency test |
| S8 audit/privacy/compatibility | metadata assertions in webhook/watch/authority tests, existing trigger audit and redaction tests, full regression suites |

### Assistant workspace placement

Automations lives at `/assistant/automations` in the assistant Workspace sidebar,
next to Machines, Plugins and Approvals, for both assistant engines. Agent details
link here with `?agent=…`; webhook setup links use `?setup=…`. The previous
`/automations` page redirects here preserving both parameters. Studio's Developer
→ Triggers remains for webhook secrets and replay and links to the assistant for
automation management. Browser links share `services::assistant_links`; API
endpoints and webhook confirmation policies are unchanged.
