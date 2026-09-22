# Aurinko compatibility and architecture review

Reviewed the Aurinko implementation after rebasing onto `main` at `efc20d245a7a6feabe75e5c085fe88d9b9c64ebb` (v0.23.0). This review was performed by the primary reviewer without additional agents. The compatibility requirement is that existing providers, requests, stored records, and authentication paths retain their behavior while Aurinko adds a new AI Service and channel adapter.

The review found compatibility issues and corrected them:

| Area | Finding | Result |
|---|---|---|
| CLI deletion | Deletion first fetched the bot, adding a read permission and availability dependency. | One DELETE accepts either legacy HTTP 204 or the Aurinko cleanup JSON. Existing bots still produce `{"ok":true}`. |
| Bot lifecycle | Inactive-bot checks and registration state checks affected existing adapters. | Lifecycle serialization and the new guards apply only to adapters that opt in. Existing adapters retain their previous update and registration paths. |
| Personal deletion | Cleanup expanded to unrelated legacy channel records. | New bot, conversation, and message cleanup is restricted to `platform: "aurinko"`; new email collections are cleaned by owner. Organization deletion retains its existing active-resource blockers. |
| Adapter discovery | Aurinko changed the existing registry prefix. | Aurinko is appended after existing adapters; their ordering and default policies remain intact. |
| CLI environment defaults | Lark environment-variable handling changed for existing platforms. | The additional exclusion applies only to Aurinko, preserving previous behavior for existing platforms. |
| Webhook ingress | Shared POST handling gained query parsing for Aurinko. | Query parsing occurs only in retry-aware ingress. Existing webhook handlers do not acquire this dependency. |

Regression coverage rejects any GET before bot deletion and exercises removed, failed, and legacy 204 deletion results. Real MongoDB tests verify inactive legacy bot updates/manual registration, mixed-platform owner cleanup, preservation of another owner's data, and repeatable index setup over existing duplicate legacy platform identities. HTTP coverage verifies that unrelated Aurinko query parameters do not affect a Slack challenge. Adapter coverage checks every existing provider's lifecycle, reply, and webhook policies.

The rebase also preserves the newer channel architecture on `main`:

- The backend platform catalog supplies Aurinko's name, registration fields, hints, and capabilities. The frontend uses the current dynamic descriptor model.
- Shared reply handling retains the centralized credential resolver and attachment authorization. Aurinko advertises text replies only and rejects attachments at both the capability gate and its provider-send boundary.
- Telegram New retains its operation lock, retirement/cancellation updates, and owned-webhook cleanup. Aurinko's separate lifecycle/ingress guard wraps only the opt-in deletion path.
- Legacy reply tokens retain their existing consumption behavior, including non-consuming attachment authorization. Aurinko uses its durable send barrier and additionally revalidates current route authority.
- Current X polling, initiated-send behavior, all existing provider seeds, and the first nine adapter registry entries are preserved.

The implementation follows the existing NyxID boundaries:

- AI Service onboarding uses the provider/catalog seeds and the existing unified endpoint, encrypted credential, and service records. The curated overlay adds twelve discoverable operations through existing MCP machinery. Generic OAuth, proxy credential resolution, approval enforcement, and agent service scopes are unchanged.
- Channel HTTP parsing and response DTOs stay in handlers; lifecycle, delivery, and reply behavior stay in services and adapter hooks. New hooks have defaults that preserve the legacy methods. Existing inbound message storage still generates a fresh UUID-v4; the added helper lets Aurinko reuse a stable message ID during retries.
- Email models contain coordination metadata, UUID-string identifiers, collection constants, and BSON datetime helpers. Existing models gain no required fields. No migration renames, drops, or rewrites existing data. Indexes target new collections or use a partial filter for active Aurinko bots.
- Credentials use existing encryption and person/organization ownership. AI Service credentials and channel credentials remain independent. Account-token onboarding avoids changing NyxID's PKCE requirement or generic OAuth implementation.
- ADR-013 is preserved: email content remains transient, persistent additions contain routing/coordination metadata, and there is no mailbox poller, durable content queue, or background resend worker.
- Existing REST paths, required request fields, legacy deletion status, and legacy reply-token consumption remain intact. Frontend additions use existing platform descriptors, Zod schemas, query hooks, and form components.

This is additive compatibility at the API, data, and behavior boundaries. Shared integration files necessarily change to register and invoke the new adapter. The review does not equate additive behavior with a diff containing only inserted lines.

Local validation ran after the main channel-architecture rebase at `9a3938364a11ab65f3b341e69e2b8c124aefba81`. The subsequent clean rebase onto `efc20d245a7a6feabe75e5c085fe88d9b9c64ebb` adds the independently merged service-account scope picker and options API. Its shared-file changes add module registrations, an owner index, CLI help text, and the rebuilt wizard; they do not alter Aurinko contracts. PR CI validates the final combined revision.

Local results:

- The full backend/CLI run executed 7,450 tests with no skips: 7,447 passed and three assertions required updates for the current catalog and message schema. Two now expect ten registered platforms; the privacy test checks that the schema's attachment-metadata array is empty while retaining the checks that message text, body, subject, and raw payloads are absent. All three updated assertions passed on the targeted rerun, resolving every failure from the full run. The suite covers 6,133 backend tests and 1,317 CLI tests.
- All 3,232 frontend tests passed across 322 files. Coverage passed at 67.01% against the 15% line threshold. The production build and full ESLint run passed with zero lint errors.
- Strict workspace Clippy passed for all targets with warnings treated as errors.
- Rust formatting, diff whitespace, and the committed wizard's source-closure freshness check passed.
- All twelve curated Aurinko operations match the current official OpenAPI specification.

Rust validation uses an isolated MongoDB 8.0 replica set with `minSnapshotHistoryWindowInSeconds=0`; its journal commit interval was reduced to 10 ms during the full run to accelerate index-heavy fixtures while retaining journaling. Provider and callback HTTP are mocked. No live mailbox or recipient delivery was exercised.

```sh
NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27129/?replicaSet=aurinko-test' \
  cargo nextest run -p nyxid -p nyxid-cli --profile ci --test-threads 4
NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27129/?replicaSet=aurinko-test' \
  cargo nextest run -p nyxid --profile ci --test-threads 4 \
    -E 'test(channel_platform_catalog) | test(aurinko_filters_threads_and_automation_without_retaining_body)'
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
git diff --check
npm --prefix frontend run test -- --maxWorkers=2
npm --prefix frontend run test:coverage -- --maxWorkers=2
npm --prefix frontend run lint
npm --prefix frontend run build
python3 scripts/check-catalog-spec-drift.py --overlay aurinko.openapi.json
```

The local Rust runs use `CARGO_INCREMENTAL=0`, `CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_PROFILE_TEST_DEBUG=0`, and `CARGO_BUILD_JOBS=1`. Frontend validation uses Node 22, matching `.node-version`.

See [the integration guide](../AURINKO_INTEGRATION.md) for the implemented onboarding, lifecycle, webhook, and reply contracts. The earlier [feasibility assessment](AURINKO_EMAIL_CHANNEL_FEASIBILITY.md) is preserved as historical research.
