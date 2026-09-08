# Managed WhatsApp Onboarding Review

Branch: `whatsapp-embedded-signup`. No push.

Implementation commits: `cd67f8fc` (backend), `661d3f01` (CLI), `3102147c` (frontend). This report and setup documentation are committed separately. Frontend preview: `http://localhost:4611/channel-bots?connect=whatsapp` (requires a running NyxID API and a user session for real use).

## Design and Storage

Adapters own `PlatformCredentialDescriptor` (provider, field labels/secrecy/validation, checklist), `ManagedOnboardingDescriptor` (provider, public bootstrap fields, accepted completion fields), completion/setup/re-registration hooks, and platform webhook handshake/target hooks. WhatsApp is the only managed adapter. `registered_adapters()` supplies the inventory used by `resolve_adapter()` and the admin listing; there is no second provider registry or IM branch in the generic backend pipeline.

`platform_credentials` has UUID-string IDs and a unique provider index, plain fields, BSON binary encrypted secret fields, `updated_by`, and BSON `updated_at`. A single MongoDB update pipeline writes fields and creates/preserves/rotates the generated encrypted platform Verify Token atomically. Decryption is on demand through `EncryptionKeys`; no decrypted AppState cache. App secrets never appear in responses, while the generated webhook Verify Token is intentionally readable only by admins. Requests, credential containers, results, and models redact secret-bearing Debug output.

Managed bots use the ordinary duplicate-check/encryption/insert path, with additive defaulted `credential_source`, optional encrypted registration PIN, and optional setup state. Legacy rows default to user credentials. Registration descriptor `platform_fallback` resolves the missing managed app secret generically. Managed Graph authentication adds app-secret proof for onboarding, verification, and replies. Per-bot verification, phone filtering, deduplication, and dispatch are reused by the shared platform receiver.

## New Routes

All routes below include their full API prefix. Existing global middleware/body limits remain in effect.

| Method | Route | Authorization |
| --- | --- | --- |
| GET | `/api/v1/admin/platform-credentials` | Authenticated platform admin via `require_admin`; operators denied; no-store |
| PUT, PATCH | `/api/v1/admin/platform-credentials/{provider}` | Authenticated platform admin; field names only in audit; no-store |
| DELETE | `/api/v1/admin/platform-credentials/{provider}` | Authenticated platform admin; metadata-only audit |
| GET | `/api/v1/channel-bots/managed-onboarding/{platform}` | Human session or first-party user access token; third-party OAuth/API key/service account/delegated/relay rejected; public bootstrap only; no-store |
| POST | `/api/v1/channel-bots/managed-onboarding/{platform}/complete` | Same human check; org write ACL when applicable; five attempts/user/minute, DB-backed |
| POST | `/api/v1/channel-bots/{id}/reregister` | Same human/rate checks plus bot owner or org write ACL |
| GET | `/api/v1/webhooks/channel/{platform}/platform` | Public; adapter opt-in and constant-time platform Verify Token handshake |
| POST | `/api/v1/webhooks/channel/{platform}/platform` | Public immediate acknowledgment; adapter opt-in, raw-body HMAC verification before background processing |

The delegated GET deny list already excludes the entire admin/channel-bots/webhooks route classes. Browser routes are `/admin/platform-credentials` (admin write-role guard) and `/channel-bots?connect=whatsapp` (authenticated dashboard). Managed completion returns JSON 201 by default; optional SSE reports actual server stages and the final result/error.

## Verification

The requested environment and commands were used:

```bash
source "$HOME/.cargo/env" 2>/dev/null
export NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27019/?directConnection=true'
cargo fmt --all -- --check
cargo clippy -p nyxid --bin nyxid-server --all-targets -- -D warnings
cargo clippy -p nyxid-cli --all-targets -- -D warnings
cargo test -p nyxid --bin nyxid-server -- --test-threads 2
cargo test -p nyxid-cli
cd frontend && npm run lint && npm run test && npm run build && npx playwright test e2e/channel-whatsapp*.spec.ts e2e/admin-platform-credentials*.spec.ts --workers=1
```

Formatting exited 0 with no output. Verbatim Clippy summary lines, backend then CLI:

```text
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 38.31s
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 9.76s
```

Verbatim final backend test summary:

```text
test result: ok. 5818 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 234.97s
```

Verbatim CLI summaries: main binary, agent-key integration, and wizard bundle freshness. The library/doctest targets also passed with zero tests.

```text
test result: ok. 1135 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.61s
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 21.71s
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.05s
```

Verbatim frontend summary lines:

```text
✖ 27 problems (0 errors, 27 warnings)
 Test Files  298 passed (298)
      Tests  2934 passed (2934)
✓ built in 852ms
✓ built in 52ms
Mock scenario footprint assertion passed; credential-accept output is present.
  7 passed (18.3s)
```

The 27 lint warnings are in existing code. The build also reports the existing malformed `text-destructive` CSS pseudo-class warning; tests emit existing mocked-network/Node deprecation diagnostics. No check failed in the final verification. After the final mobile navigation guard edit, targeted ESLint also passed (8 existing warnings). Desktop/mobile screenshots of signup, bot detail, and admin credentials were inspected. The wizard manifest has no intersection with edited frontend files, so `build:wizard` was not required; the existing freshness integration test passed in the full CLI run.

## Existing Tests Edited

No existing test was deleted, renamed, ignored, or weakened. No existing frontend test was edited. The following existing Rust helpers/tests needed mechanical initializer updates for the additive ChannelBot fields (`credential_source: "user"`, PIN/setup `None`), or the CLI managed flag/optional label type. Their prior inputs and assertions are retained.

| File | Existing helper/test | Reason |
| --- | --- | --- |
| `backend/src/handlers/channel_bots.rs` | `make_lark_bot`, `make_telegram_bot` | Add legacy ChannelBot defaults |
| `backend/src/handlers/channel_relay.rs` | `setup_reply_token_fixture`, `reply_token_context_rejects_message_conversation_bound_to_other_bot` | Add legacy ChannelBot defaults |
| `backend/src/handlers/channel_webhooks.rs` | `valid_lark_event_promotes_pending_webhook_bot_and_stores_message` | Add legacy ChannelBot defaults |
| `backend/src/models/channel_bot.rs` | `make_channel_bot` | Add legacy ChannelBot defaults; separate new BSON compatibility test verifies missing fields and managed binary round-trip |
| `backend/src/services/channel_adapters/discord.rs` | `make_test_bot` | Add legacy ChannelBot defaults |
| `backend/src/services/channel_adapters/lark.rs` | `make_test_bot` | Add legacy ChannelBot defaults |
| `backend/src/services/channel_adapters/openclaw.rs` | `make_test_bot` | Add legacy ChannelBot defaults |
| `backend/src/services/channel_adapters/slack.rs` | `make_test_bot` | Add legacy ChannelBot defaults |
| `backend/src/services/channel_adapters/telegram.rs` | `make_test_bot` | Add legacy ChannelBot defaults |
| `backend/src/services/channel_bot_service.rs` | `make_lark_bot` | Add legacy ChannelBot defaults |
| `cli/src/commands/channel_bot.rs` | `register`, `register_lark_includes_all_optional_fields`, `register_with_org_sets_target_org_id` | Add `managed: false`; wrap unchanged labels in `Some` |

New coverage includes adapter inventory, credential masking/rotation/clear/fallback, Graph success and rejections, nonfatal override failures, independently verified already-registered state, PIN reuse, coexistence, bounded WABA pagination, admin/human authorization, multi-number dispatch, unknown-number drops, retained dedup, CLI wiremock/argument tests, schema/hook/descriptor rendering tests, and desktop/mobile Playwright flows using a fake FB SDK and origin-checked messages.

## Meta Docs and Deployment Limits

- Official documentation was checked September 8, 2026; links and operational detail are in `docs/CHANNEL_BOT_RELAY.md`.
- The requested `featureType`/`sessionInfoVersion: "3"` contract is documented for legacy v2. The number 3 selects the session-event payload, not flow v3. Meta deprecates v2 on October 15, 2026 and lists v3 availability through October 2026; it recommends v4. The requested SDK options and existing Graph `v25.0` pin are preserved. Migration of the SDK launch contract remains a deployment follow-up before that deadline.
- Coexistence explicitly skips number registration. NyxID verifies live Cloud API coexistence state and initiates Meta's one-shot contact/history sync. It records requested/failed outcomes, without importing historical chats or app-sent echoes into agent conversations.
- No stable numeric already-registered error was confirmed in Meta's registration/error documentation. NyxID independently checks exact phone identity and `CONNECTED` status after a failed register request.
- Business integration system-user tokens default to never expire, but can be configured to expire or revoked. NyxID checks expiry at onboarding and requires reconnect after expiry/revocation; no automatic refresh flow is claimed.
- Meta allows omitted granular targets for all-assets grants; this implementation deliberately follows the requested stricter policy requiring explicit WABA membership in both scopes.
- No real Meta app credentials/customer account were available for live App Review, popup, callback override, coexistence, or messaging validation. Those external effects were tested with mocked Graph and browser APIs, not a production Meta tenant.
- Frontend nginx currently sets no CSP. Deployments adding one must allow the documented Meta SDK/frame/connect origins; API CSP remains unchanged.
- Setup is saved before callback subscription so Meta can verify the per-bot URL. There is no distributed transaction or durable recovery job: partial failures remain visible on the saved bot. Registration can be retried with the stored PIN; subscription repair can use Meta administration or deletion/re-onboarding. Existing immediate-ack process-loss and concurrent-dedup windows remain.
- No runtime environment variables were added. No files in the CLI wizard frontend graph were edited.

## Full File List

```text
CLAUDE.md
Cargo.lock
backend/Cargo.toml
backend/src/db.rs
backend/src/handlers/admin_platform_credentials.rs
backend/src/handlers/channel_bots.rs
backend/src/handlers/channel_managed.rs
backend/src/handlers/channel_managed_tests.rs
backend/src/handlers/channel_relay.rs
backend/src/handlers/channel_webhooks.rs
backend/src/handlers/mod.rs
backend/src/models/channel_bot.rs
backend/src/models/mod.rs
backend/src/models/platform_credential.rs
backend/src/routes.rs
backend/src/services/channel_adapters/discord.rs
backend/src/services/channel_adapters/lark.rs
backend/src/services/channel_adapters/mod.rs
backend/src/services/channel_adapters/openclaw.rs
backend/src/services/channel_adapters/slack.rs
backend/src/services/channel_adapters/telegram.rs
backend/src/services/channel_adapters/whatsapp.rs
backend/src/services/channel_adapters/whatsapp_managed.rs
backend/src/services/channel_bot_service.rs
backend/src/services/channel_managed.rs
backend/src/services/channel_platform.rs
backend/src/services/channel_registration.rs
backend/src/services/mod.rs
backend/src/services/platform_credential_service.rs
cli/src/cli.rs
cli/src/commands/admin.rs
cli/src/commands/admin_platform_credentials.rs
cli/src/commands/channel_bot.rs
cli/src/commands/mod.rs
docs/CHANNEL_BOT_RELAY.md
docs/WHATSAPP_MANAGED_ONBOARDING_REVIEW.md
frontend/e2e/admin-platform-credentials.spec.ts
frontend/e2e/channel-whatsapp-managed.spec.ts
frontend/e2e/managed-onboarding-fixtures.ts
frontend/src/components/channels/managed-whatsapp.tsx
frontend/src/components/dashboard/sidebar.tsx
frontend/src/components/layout/dashboard-layout.tsx
frontend/src/hooks/use-admin-platform-credentials.test.tsx
frontend/src/hooks/use-admin-platform-credentials.ts
frontend/src/hooks/use-channel-managed.ts
frontend/src/lib/channel-platforms.ts
frontend/src/lib/meta-embedded-signup.ts
frontend/src/pages/admin-platform-credentials.test.tsx
frontend/src/pages/admin-platform-credentials.tsx
frontend/src/pages/channel-bot-detail.tsx
frontend/src/pages/channel-bots.tsx
frontend/src/pages/lazy.ts
frontend/src/router.tsx
frontend/src/schemas/admin-platform-credentials.ts
frontend/src/schemas/channel-managed.test.ts
frontend/src/schemas/channel-managed.ts
frontend/src/types/admin.ts
frontend/src/types/channels.ts
skills/nyxid/references/admin.md
skills/nyxid/references/channels.md
```
