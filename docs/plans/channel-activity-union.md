# Typed channel activity implementation record

The implementation builds on main `4e8de717`, including PR #1640's X mentions and replies. It is implemented and personally reviewed in the existing Heca `gpt-6-astra` / xhigh / full-access session. No subagent was spawned. Runtime documentation and receiver integration instructions are in [Typed channel activity](../CHANNEL_ACTIVITY.md).

## Delivered behavior

- Adapter-described activity kinds and labels, with X selections for DMs, encrypted chat, mentions, replies and own posts.
- An activity count and latest received activity on the bot page, plus filtered activity history and callback state on bot/route pages. Visible pages refresh database reads every 15 seconds and on focus.
- An explicit receiver declaration followed by human owner consent. Declarations bind the assigned agent key revision and callback URL; identical retries preserve consent, changed contracts require consent again, and reassignment clears the declaration.
- A signed version-1 `activity` field for opted-in kinds, while legacy callback bodies retain their field sets, omissions, content types, signatures and reply behavior.
- Encrypted chat and own-post metadata notifications isolated from ordinary message storage. No plaintext, crypto payload, conversation token, owner access token, attachment authority or reply token is forwarded for these notifications. The ordinary reply API cannot resolve notification IDs, including with a full agent API key.
- Durable admission independent of callback results, bounded owner-scoped aggregation, 30-day notification retention, account/org cleanup, and preserved ordinary message-history counts and `last_message_at` semantics.
- Existing public mention/reply deduplication and billing identities preserved; encrypted UUID events use a separate admission/billing namespace.
- Real X subscription reconciliation for the new selections, using the existing shared signed webhook and live OAuth/account checks.

## Review and validation

Focused tests cover signed encrypted ingress, concurrent/redelivered events, notification consent, exact-body signing, reply rejection, callback failure, live connection removal, account/route/type/retention filters, legacy metadata, malformed identities, selection/account/tag/signature rejection, own-post binding, and subscription reconciliation. Existing X public-reply, billing, DM and channel regression tests are included in final validation.

UI validation includes component tests, TypeScript and lint checks, and the real frontend running against API fixtures in Chromium at 1440px and 390px. The fixtures explicitly show one encrypted activity alongside zero ordinary messages. Both layouts are checked for overflow and inspected visually.

The final caller review separated webhook requirements from public reply scopes. Chat and own-post selections require a working webhook during Verify and reconnect, and the polling worker refuses to substitute DM polling. They do not require `tweet.write`. The regression exercises missing platform credentials, provider setup failure and reconnect transitions for both selections; existing DM fallback behavior stays covered by the original tests.

Final runtime validation on 2026-09-25:

| Check | Result |
| --- | --- |
| Rust channel regression suite, using a local MongoDB replica set | 595 passed; 0 failed; 0 ignored |
| CLI X event flag validation and actual HTTP request body | 1 passed |
| Frontend activity, schema, platform and managed-flow tests | 40 passed across 4 files |
| Chromium X scenarios at desktop and mobile widths | 7 passed |
| TypeScript project build, affected-file ESLint, Rust formatting and diff whitespace | Passed |
| Backend Clippy, including test targets, with warnings denied | Passed |

The final Rust run used the normal stack limit. The new concurrent notification fixture initially exhausted its test stack; boxing its two complete webhook futures resolved that failure without increasing the final stack limit or disabling an assertion. The public reply regression now covers both legacy and typed callbacks with billing enabled and disabled, including a successful reply to the admitted post. No wizard bundle inputs changed. The task-owned MongoDB instance was stopped after validation.

Reproduce the Rust checks with `NYXID_TEST_DATABASE_URL` pointing to a replica set:

```sh
cargo test -p nyxid --bin nyxid-server channel_ -- --test-threads=4
cargo test -p nyxid-cli --bin nyxid x_events_tests -- --test-threads=1
cargo clippy -p nyxid --bin nyxid-server --tests -- -D warnings
cargo fmt --all -- --check
```

Frontend checks, from `frontend/`:

```sh
npx tsc -b
npm test -- src/components/channels/channel-activities.test.tsx src/schemas/channels.test.ts src/lib/channel-platforms.test.ts src/components/channels/managed-flows.test.ts
npx playwright test e2e/channel-x-managed.spec.ts --workers=1 --reporter=line
```

## Deployment boundary

No production deployment, live subscription change or message send was performed. All replicas must support the new X selections before they are enabled; the rollback order is documented. Existing bots and agents keep their legacy configuration until explicitly changed.

The incident evidence identifies encrypted X Chat alongside `dm.received` subscriptions. It does not prove that the production webhook received a `chat.received` event. Real provider delivery and the receiving agent's acceptance need the documented post-deployment verification.

Fable was not contacted: the session did not expose a Fable agent or messaging destination, and no destination was supplied. The documented receiver contract is concrete and reviewable; no external sign-off is claimed.
