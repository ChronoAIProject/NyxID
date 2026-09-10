# OAuth return configuration: requirements and blast radius

Date: 2026-09-10. Scope: named application return destinations for catalog OAuth connections. Google continues using the existing production provider callback.

## Contract selected before implementation

`OAUTH_RETURN_ROUTES` is a backend-owned JSON configuration. It maps canonical catalog service slugs, such as `api-google`, and named pages to absolute return URLs. A provider slug such as `google` and a user's generated service instance slug are different identifiers and must not be substituted.

```json
{
  "default_url": "https://nyx.chrono-ai.fun/dashboard",
  "services": {
    "api-google": {
      "default_url": "https://nyx.chrono-ai.fun/keys",
      "pages": {
        "dashboard": "https://nyx.chrono-ai.fun/dashboard",
        "onboarding": "https://nyx.chrono-ai.fun/ai-setup",
        "local-poc": "http://127.0.0.1:3003/temp"
      }
    }
  }
}
```

The create request supplies `service_slug` and an optional `return_page`. When `return_page` is supplied, the resolution order is exact service/page, service default, global default. The reserved selector `default` requests the service/global default directly. Service lookup and return lookup both trim the supplied service slug before matching. An unconfigured catalog service uses the global default, but a nonexistent catalog service still fails catalog validation. Invalid selector syntax is rejected.

Named routing is opt-in. Omitting `return_page` preserves the existing optional `callback_url` behavior, including no callback. Supplying both fields is rejected. A missing routing configuration rejects a named request before link creation. Malformed configuration fails backend startup. No Referer, arbitrary request URL, wildcard host, or caller query parameter becomes a destination.

Configured URLs permit HTTPS and explicitly configured HTTP loopback targets. Userinfo, fragments, control characters, backslashes, and unsupported schemes are rejected. URL and configuration sizes are bounded. The config is validated once at startup. Resolved URLs are still checked against the authenticated developer app's redirect policy when applicable.

Resolve the destination once and persist it in the existing `ConnectLink.callback_url`. Changing configuration does not silently reroute a transaction already in progress. Existing cancellation and expiry are the mechanisms for ending those requests.

## Safety claims and proof targets

The central claim is that named configuration changes only destination selection at link creation. It does not change who can bind a credential, what Google receives as `redirect_uri`, how OAuth state is consumed, or the existing terminal-status contract.

Two proofs are required: an actual create/complete-or-cancel round trip must preserve the selected destination and replace reserved outcome parameters; and a mapped destination must still be rejected when it violates the requesting app's redirect policy. A pure config test alone cannot prove either boundary.

| Boundary | Confirmed dependency | Failure if handled incorrectly | Required check |
| --- | --- | --- | --- |
| Google code exchange | `services/user_token_service.rs` builds the same callback at authorization and exchange | `redirect_uri_mismatch` or rejected code exchange | Provider callback construction stays outside the diff; existing callback tests |
| Existing consumers | CLI/wizard use `/keys/{id}`; chat uses `flow=cc`; admin uses service-account paths | A global default silently replaces required continuation behavior | Opt-in create parameter; existing direct OAuth call sites remain unchanged |
| App identity | Connect-link creation derives `oauth_client_id` from verified authentication | A globally configured URL bypasses app registration | Handler/service integration test with an unregistered mapped URL |
| Terminal state | Pending denial stays pending; terminal callbacks are completed/cancelled/expired only | Caller treats retryable denial or a provider-only success as completed | Retain existing status enum and callback builder; run callback/lifecycle tests |
| URL query contract | `terminal_callback_url` preserves context and replaces status/link ID | Spoofed or duplicated completion fields reach the app | Execute real builder through persisted-link terminal handler |
| Concurrent tabs | Each link stores its own destination | One tab overwrites another's return URL | Create two links for the same user/service with distinct page selectors |
| Configuration changes | OAuth may complete after a backend rollout | Completion resolves a different URL from the original request | Verify persisted destination after mutating the in-memory routing config |
| Browser authentication | Hosted return page requires hosted auth; tab token is origin scoped | Local-only initiation can require another login or lose retry context | Existing browser reproductions; config selection must not claim to solve direct return |
| Native and device flows | Custom-scheme app callbacks and device-code pollers already exist | HTTP-only named config accidentally bans existing native callbacks | Keep the existing explicit callback path; run native callback compatibility tests |
| SDK/API | SDK constructs create JSON and polls a fixed status union | New field never reaches server, or new result shape breaks callers | Additive SDK field and request tests; unchanged status union |
| Mixed backend versions | An old replica can ignore a new JSON field | New frontend assumes a mapped callback was persisted when it was not | Echo resolved destination; named-flow frontend requires acknowledgement and handles an old response explicitly |
| Deployment config | Local Vite env cannot configure production callback code | Local config appears to work while prod uses no mapping | Document backend deployment setting and exact local URL; test config parser independently of Vite |

## Requirements for callers

The frontend sends a page name, not a destination guessed from request headers. The create response acknowledges the saved destination. The initiating app stores the returned link ID before navigating; on return it verifies that ID with the authenticated status API. That browser record belongs to the initiating origin and tab. Hosted pages initiate hosted returns; the local POC initiates its localhost return. URL status parameters are notifications, not evidence of credential ownership or completion.

Registered web apps should use a fixed registered completion URI and keep dynamic onboarding state in their own attempt record. Full-URI matching includes the query string. Public-client loopback/private-scheme compatibility remains governed by the existing OAuth app policy.

The local POC can exercise named selection once production has this backend configuration. The existing hosted connect flow still owns its hosted authentication and retry token. A direct production-API return to an already-authenticated localhost page is the separate extension described in `OAUTH_CONTEXTUAL_RETURN_RESEARCH.md`; this config change does not alter the generic OAuth callback to implement it.

## Verification record

Backend integration checks used a dedicated temporary local MongoDB instance with `NYXID_TEST_DATABASE_URL` explicitly set. This prevents optional database tests from silently skipping or accessing production. The implementation and personal review ran in the existing Heca session, which reported `gpt-6-astra`, `x_high`, and full access. No additional implementation or review agent was spawned.

| Executed check | Result | Evidence |
| --- | --- | --- |
| Configuration parser/resolver | 4 passed | Page/service/global fallback, selector and URL validation, duplicate-key rejection, size limits, redacted Debug output |
| Connect-link handlers with MongoDB | 10 passed | Separate persisted destinations, trimmed catalog slug, config changes during an attempt, canonical terminal query fields, app-policy rejection and acceptance, explicit native callbacks and no-callback requests while configuration is enabled |
| Connect-link service with MongoDB | 32 passed | Existing ownership, lifecycle, terminal callback, native/app, credential completion and concurrency behavior |
| Generic provider callback with MongoDB | 10 passed | Existing OAuth callback behavior, including error and connect-link settlement cases |
| Public configuration serialization | 5 passed | Additive capability flag and existing public configuration contract |
| Frontend unit checks | 2,977 passed; 33 focused checks passed after the shared schema change | Full frontend suite plus affected schemas, hooks, onboarding and route helpers |
| OAuth SDK service checks | 10 passed; SDK build passed | Named request JSON and destination acknowledgement, existing service/connection API behavior |
| Configured-return Chromium tests | 14 passed | Dashboard, AI Setup, first-run onboarding, local login continuation, fallback recovery, separate attempts, storage failures, account isolation, mixed backend versions, denial and forged or mismatched results |
| Existing login Chromium tests | 9 passed | Agent-key login, login-code and browser device-login regressions |
| Hosted continuation browser reproduction from the initial research | 8 passed | Signed-out login continuation, success/cancellation/expiry, denial retry and missing local token behavior |
| Static and production checks | Passed | Rust formatting and Clippy with warnings denied; changed frontend ESLint; TypeScript compilation; production frontend and credential-accept builds; mock-footprint assertion |
| Production bundle inspection | Passed | Both development-only POC modules and their distinguishing UI text are absent from production output |

The central destination-persistence and app-registration claims were executed against real handlers, services and MongoDB. Browser checks ran the actual React pages with intercepted APIs; they do not prove live Google consent or deployed hosted-frontend behavior.

### Review findings resolved

| Finding | Consequence | Resolution and regression evidence |
| --- | --- | --- |
| An old backend replica can ignore `return_page` | Setup opens without the requested return destination | Creation echoes the saved callback; the POC requires it before navigation and allows cancellation when missing. Browser test covers the old response. |
| A single session-level destination can be overwritten by another attempt | Dashboard and onboarding requests return to the wrong page | Each link snapshots its destination in MongoDB. Handler test creates distinct links for the same owner/service, then changes configuration before cancellation. |
| Catalog lookup trims service slugs but the initial map lookup did not | A whitespace-padded valid slug silently selects the global fallback | Both use the trimmed slug; the handler round trip exercises padded input and confirms the page-specific destination. |
| Invalid status data could still drive terminal POC controls | A mismatched result hides cancellation and permits discarding the active request | Status-specific text and terminal controls now require the matching request/service and a connected service for completion. Browser test injects mismatched completed, cancelled and expired responses. |
| The global frontend login guard discarded connect-page continuation | Signed-out hosted entry loses the token/link destination | Only the existing connect-page shapes bypass that initial guard; the pages preserve their query string when directing through login. Route tests and the earlier browser reproduction cover the fix. |
| Dashboard and onboarding login guards could race | An expired session replaced the original return with `/login` | The global guard preserves the path/query and the layout guard reads its router location. Chromium reproductions pass for both pages. |
| The local POC login URL omitted callback fields | Reauthentication could lose the returned attempt ID | QR and login-page entry share the complete `/temp` path/query. Chromium reproduced the loss before the fix and preserves the destination afterward. |
| Page-specific storage could not restore a fallback destination | Returning to another page lost the initiating attempt | Saved attempts are indexed by account and matched by returned ID before page context. The browser test completes one request and proves another context survives. |
| Unavailable browser storage left an unresumable setup link | Leaving the page lost request correlation | Setup stays disabled after storage failure while cancellation remains available. The browser test cancels and clears the request. |

No identified implementation finding remains open in this scope. Production enablement consists of releasing this backend with the same valid configuration on all replicas and releasing the hosted frontend continuation fix. A production-connected local Vite page cannot install either change on production.

### Reproduce the focused checks

From the repository root, with `NYXID_TEST_DATABASE_URL` explicitly exported to a dedicated **local test** MongoDB:

```sh
cargo test -q -p nyxid config::oauth_return_routes::tests::
cargo test -q -p nyxid handlers::connect_links::tests::
cargo test -q -p nyxid services::connect_link_service::tests::
cargo test -q -p nyxid handlers::user_tokens::tests::generic_oauth_callback
cargo test -q -p nyxid handlers::health::tests::
cargo fmt --all --check
cargo clippy -q -p nyxid --tests -- -D warnings
```

From `frontend/`, run `npx playwright test e2e/configured-oauth-return.spec.ts` and `npm run build`. The repository browser test intercepts backend requests and starts a separate Vite server; it does not ask Google for consent or create production connections.

The live Google test remains a release validation step: open `http://127.0.0.1:3003/temp`, sign in, prepare the `local-poc` return, verify the acknowledged destination, complete hosted setup and Google consent, and confirm the original link through the authenticated local status API. Google must keep the exact existing production provider callback registered. Current `main` already includes the managed Workspace scope allowlist and presets. Google project APIs, consent settings and organization policy still need live verification. A direct return that skips hosted login remains the separate extension described in the research report.
