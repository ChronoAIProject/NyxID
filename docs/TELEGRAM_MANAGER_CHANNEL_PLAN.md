# Telegram manager as a public channel bot

## Outcome and scope

An existing, configured Telegram manager can be connected through the ordinary
Telegram bot token flow and assigned a public default agent or explicit chat
routes. Its single manager webhook continues handling bot creation, recovery,
and management events while ordinary messages reach the assigned agent and
asynchronous replies return through the same Telegram identity.

Fable and the implementation lead reviewed the design. An independent Astra
review at xhigh must assess the final implementation against the acceptance
criteria below, and findings must be fixed and re-reviewed before completion.

## Decisions

- Keep one webhook and the live manager credential. Registration, verification,
  and channel deletion must never replace or remove the manager webhook.
- Preserve existing registration authorization: a valid bot token and the
  authenticated person's personal/org write access. Permit one active channel
  binding for the remote manager identity. Token rotation is not revocation of
  that binding; deleting the binding revokes its routes.
- Treat every manager channel as a public-service interface. Its callbacks have
  signed delivery authentication and message-bound reply tokens, but no owner
  relay access token. Explicit routes have the same rule as the default route.
  The operator must separately scope any credentials held by the receiving agent.
- Reserve manager commands and callback queries. A plain `/start` offers chat
  and bot creation when the manager channel is connected. Existing setup deep
  links, addressed commands, consent, and recovery continue to work.
- Strip embedded quoted setup material and redact recognizable claim material
  from ordinary messages before any agent callback; never persist message bodies.
- Authentication, unclassifiable updates, and management-event failures remain
  fail-closed. Once authenticated and classified as ordinary chat, relay failures
  are acknowledged and logged without poisoning Telegram's management-delivery
  checks. Those checks retain their pending-update and delivery-error fences.
- Keep the existing 32 concurrent background deliveries per process and timeout.
  Document drop-on-full, process-loss, possible duplicates, and unordered delivery.
  This work adds no durable queue, replay, arbitrary Telegram callback support,
  per-sender NyxID login binding, or configurable welcome-message editor.

## Acceptance criteria

| ID | Required behavior | Verification |
| --- | --- | --- |
| A1 | Connect a configured existing manager using the standard token path; expose `telegram_manager` without a duplicate token or shared webhook secret. | Transaction-backed registration, duplicate identity and ownership checks; desktop/mobile browser registration. |
| A2 | Verify live webhook URL and all five required subscriptions without modifying the webhook; surface unready manager configuration. | Simulated Telegram API success, URL drift, missing subscription, saved readiness, and zero mutation assertions. |
| A3 | Ordinary messages and asynchronous replies work through public default and exact routes while creation and recovery continue. | Backend integration with real MongoDB transactions and simulated Telegram/agent HTTP endpoints. |
| A4 | Every manager callback omits owner access delegation and retains a valid message-bound reply token; unrelated channels retain current behavior. | Callback header/token assertions, multiple senders, wrong-agent/reply destination checks. |
| A5 | Plain and addressed `/start`, setup payloads, recovery, creation events, and setup callbacks remain isolated from agents. | Welcome/deep-link/recovery tests and interleaved manager/chat integration. |
| A6 | Setup quote/claim content cannot leak through the supported normalized or raw callback fields covered by the protocol. | Text, caption, reply, quote, external reply, pinned/nested service messages, and claim-link regression cases, including encoded query parameter names and values. |
| A7 | Auth/control failures remain errors; ordinary post-auth lookup/processing failures acknowledge without manager effects. | Failure injection and unchanged provisioning delivery-fence tests. |
| A8 | Live token rotation, channel deletion, configuration clear blocking, and disabled routes preserve their lifecycle contracts. Stale ordinary-channel operations cannot replace the manager webhook after migration. | Rotation/delete/clear integration, old reply-token rejection, and migration tests for deleted rows, concurrent provider operations, and mismatched token identities. |
| A9 | Dashboard and CLI explain credential source, public routing, verification failure, and deletion behavior; exact-route input cannot silently enable public routing. | Desktop/mobile browser flows, wildcard rejection, positive/negative chat IDs, frontend checks, CLI show regression. |
| A10 | Existing creation and ordinary-channel behavior pass relevant regressions; independent review has no unresolved findings. | Focused backend/frontend/CLI suites, formatting/static checks, Astra xhigh review and follow-up. |

## Validation boundary

Automated end-to-end coverage uses a local transaction-capable MongoDB and
simulated Telegram HTTP responses, plus the real frontend in a browser with API
fixtures. Real Telegram client behavior, external bot ownership events, and
production throughput require staging credentials; automated results must not be
described as live Telegram validation. No production deployment is part of this
implementation.

## Completion record

Complete on 2026-09-20 for the implementation and automated validation scope
defined above. All acceptance criteria A1–A10 are satisfied, with no unresolved
findings in the independent review. No live Telegram operation or deployment
was performed.

Completed checks:

- `cargo test -p nyxid --bin nyxid-server telegram_new -- --test-threads=4`:
  59 passed, none failed or ignored, using an isolated MongoDB replica set and
  simulated Telegram/agent HTTP endpoints. This includes the public chat/reply
  integration, concurrent creation, privacy regressions, live webhook checks,
  and persisted failure/recovery after a revoked manager token.
- `cargo test -p nyxid --bin nyxid-server channel_ -- --test-threads=4`:
  479 passed, none failed or ignored, against the same isolated replica set.
  This broader run covers channel handlers, routing, replies, credentials,
  adapters, and existing channel platforms. It overlaps the focused run.
- Frontend channel hooks, list, detail helpers, and schemas: 55 tests passed.
  TypeScript and production builds passed. ESLint reported no errors; 27 existing
  warnings are outside the changed files, and changed-file lint was clean.
- The final desktop/mobile browser flow passed at 1440×1000 and 390×844,
  including wildcard rejection and successful private/group routes.
  The CLI's three `commands::channel_bot::tests::show_` tests passed, including
  manager credential-source and readiness presentation.
- `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo fmt --all -- --check`, and `git diff --check` passed.

The backend runs used `NYXID_TEST_DATABASE_URL` pointing to the isolated replica
set, with `CARGO_INCREMENTAL=0` and `CARGO_PROFILE_TEST_DEBUG=0`. Workspace Clippy
used `CARGO_INCREMENTAL=0` and `CARGO_PROFILE_DEV_DEBUG=0`. Frontend checks were
the four relevant Vitest files, `npx tsc -b`, ESLint, `npm run build`, and
`e2e/channel-telegram-manager.spec.ts` using an isolated Vite server and API
fixtures. The implementation lead inspected desktop/mobile screenshots.

Independent Astra review at xhigh found and verified fixes for nested setup
content in callbacks, encoded claim links, and stale persisted health after
failed token verification. Its follow-up found a wildcard chat ID could create
a public route while the explicit public-route switch was off. That final UI
fix and its regression checks passed: neither `*` nor ` * ` submits a route
in exact mode, including a direct form submission with an existing public default.
The independent Astra reviewer at xhigh then re-reviewed the final source and
reported no unresolved findings against A1–A10. The implementation lead reviewed
the final backend/frontend changes. All required local validation checks passed.
The accepted 32-delivery cap, transient delivery behavior, and live-provider
validation boundary remain documented limitations of this version.

## Follow-up implementation review

A fresh independent Astra reviewer at xhigh reviewed the published feature and
its integration with main's X webhook billing changes on 2026-09-20. The review
identified an existing ordinary-Telegram lifecycle defect relevant to migration:
Verify on a deleted channel could replace the manager's shared webhook. The
fix coordinates ordinary registration, Verify, deletion, and manager
configuration through the same remote-identity lease, checks the current row
and actual token identity before webhook mutations, and keeps the matching
status and secret-hash writes inside the operation. Repeated deletion still
finishes local route cleanup without changing the manager webhook.

The implementation lead also identified a delayed creation-failure update that
could overwrite a concurrent successful Verify. That Telegram update now only
matches an active, pending, unregistered row. Four migration regressions cover
deleted rows, paused provider operations, mismatched stored token identities,
an unready manager, legacy credential-source fields, matching installed/stored
secrets, and delayed failure writes. The final backend run of
`cargo test -p nyxid --bin nyxid-server -- telegram_new channel_ --test-threads=4`
passed **593 tests, with zero failures or ignored tests**, using an isolated
MongoDB replica set and simulated provider HTTP endpoints.
Workspace Clippy with `--all-targets -- -D warnings`, workspace formatting,
and `git diff --check` also passed on the final source.

Astra re-reviewed both fixes and reported no remaining actionable findings
against A1–A10, including X billing attribution and Aurinko lifecycle
compatibility. This was a source and test-design review; the implementation
lead ran the tests. Fable's final implementation review was retried but could
not run because its provider reported exhausted usage credits. Fable's earlier
design consultation remains recorded; a final Fable implementation sign-off
is still outstanding.
