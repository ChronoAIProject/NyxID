# WP-2B — Wave 2 team package: services, connections, providers (7 verbs)

**Master plan:** `docs/chat/waves234-plan.md`. **Prerequisites merged:** WP-0A, WP-0B,
WP-2PM scaffolding.

**Verbs:** `service.update`, `service.delete`, `service.route`,
`service.rotate_credential`, `connection.revoke`, `provider.set_app_credentials`,
`provider.disconnect`.

**Owned files:**

- `backend/src/handlers/assistant_evidence/services.rs` (extend the WP-0A reference
  projection — coordinate timing with 0A's author; you are the only post-0A writer),
  `providers.rs`, `connections.rs` (+ tests). PM adds dispatch lines.
- `frontend/src/components/assistant/journeys/service-update.tsx`,
  `service-delete.tsx`, `service-route.tsx`, `service-rotate-credential.tsx`,
  `connection-revoke.tsx`, `provider-set-app-credentials.tsx`,
  `provider-disconnect.tsx` + tests.

**Verify before building:** every endpoint against `origin/main`
`backend/src/routes.rs`; the `UserService` vs legacy `DownstreamService`+
`UserProviderToken` split (CLAUDE.md §8 — proxy resolution falls back to legacy for
unmigrated users; your verbs operate on the **unified** rows via `/keys` and
`/user-services`, with legacy `connection.revoke` and `provider.*` explicitly the
legacy-surface verbs).

## Verb specifications

Evidence kinds: `user_service` (WP-0A reference projection), `provider_connection`
(new), `service_connection` (new, legacy `UserServiceConnection`).

### `service.update` — grant
- Params: `{userServiceId}`; edits in-dialog (name, custom User-Agent, identity
  propagation, default headers — whatever `PUT /api/v1/keys/{key_id}`
  (`handlers/keys.rs::update_key`) accepts; verify the request struct and expose only
  fields the dashboard `/keys` page already exposes).
- Journey: preflight `GET /keys/{userServiceId}` (substrate secret-free pattern) →
  dialog → `runDirectMutation`.
- Postcondition (kind `user_service`): `id` match ∧ `is_active` ∧ `updated_at`
  present. Resource: `{userService: {userServiceId}}`.
- Negative fixtures: stale id, unauthorized (other user's service), org service
  without admin rights (server 403/404 → blocked), secret-shaped edits (the dialog must
  not offer credential fields — credential changes are `service.rotate_credential`).

### `service.delete` — destructive
- Endpoint: `DELETE /api/v1/keys/{key_id}` (unified delete; verify it removes the
  UserService and what happens to the UserApiKey/UserEndpoint — surface the actual
  cascade in the confirm dialog text; `TODO — not investigated:` read
  `unified_key_service::delete_key` before writing the copy).
- Delete pattern: confirm-every-time → mutate → `reportCompletedAfterDeleteVerify`
  (evidence 404). Resource: `{userService: {userServiceId}}`.
- Negatives: already-deleted, unauthorized, replay.

### `service.route` — grant
- Params: `{userServiceId, viaNodeId?}` (absent `viaNodeId` = the dialog asks; an
  explicit clear-routing choice must exist in-dialog — params cannot express null in
  the wire grammar).
- Endpoint: `PUT /api/v1/keys/{key_id}` setting `node_id` (node routing absorbed into
  `UserService.node_id`, CLAUDE.md §8; verify the update struct accepts it — if
  routing lives on a different field/endpoint, follow the handler, not this brief, and
  correct the brief in your PR).
- Preflight must verify the node exists and is owned/visible
  (`GET /api/v1/nodes/{node_id}`) and show node status (routing to an offline node is
  allowed but warned).
- Postcondition (kind `user_service`): `node_id == viaNodeId` (or null after clear).
  This is an exact-effect predicate — do not accept status-only evidence.
- Negatives: unknown node, node owned by someone else, stale service id.

### `service.rotate_credential` — grant (secret-bearing journey)
- Params: `{userServiceId}`.
- Endpoint: verify which surface rotates the unified credential —
  `PUT /api/v1/keys/{key_id}` accepts credential material on update, and
  `PUT /api/v1/connections/{service_id}/credential`
  (`handlers/connections.rs::update_connection_credential`) is the legacy path.
  `TODO — not investigated:` pick the unified path after reading
  `unified_key_service`; the journey targets unified rows only. New secret typed in
  the dialog only. For OAuth-backed services, this verb must **block** with a note
  pointing at `service.reauthorize` (do not build a second OAuth path — Wave-1
  contract owns that journey).
- Postcondition (kind `user_service`): identity ∧ `is_active` ∧
  `credential_version`-style correlate (same investigation as WP-2A's
  `external_key.rotate`; share the mechanism — coordinate via PM so both projections
  expose the same field name).
- Negatives: OAuth service → blocked-with-pointer (test it), empty secret, stale id.

### `connection.revoke` — destructive (legacy surface)
- Params: `{serviceId}` (a `DownstreamService` id — legacy connections key off the
  catalog service).
- Endpoint: `DELETE /api/v1/connections/{service_id}`
  (`handlers/connections.rs::disconnect_service`).
- Evidence kind `service_connection` — projection over the legacy
  `UserServiceConnection`: `service_id`, `connected` (bool), `updated_at`. Success
  evidence for revoke: `connected == false` **or** clean 404 (verify what disconnect
  actually does to the row — flag in the projection tests).
- Negatives: never-connected service (blocked "nothing to revoke"), stale id, replay.

### `provider.set_app_credentials` — grant (secret-bearing; Aevatar parser pre-built)
- Params: `{providerSlug}` — **exactly this; Aevatar's shipped parser pins it**
  (`EnsureOnlyProperties(root, "providerSlug")`, max 128).
- Preflight: resolve slug → provider via `GET /api/v1/providers` list is forbidden
  (A8); verify a single-entry lookup exists (`GET /providers/{provider_id}` is
  id-keyed; `TODO — not investigated:` slug→id resolution without a list fetch — if
  none exists, add a `?slug=` filter to the provider list handler in *your* backend
  scope, documented as the one non-evidence backend edit of this package).
- Endpoint: `PUT /api/v1/providers/{provider_id}/credentials`
  (`handlers/user_credentials.rs::set_my_credentials`) — the per-user OAuth **app**
  credentials (client id/secret typed in-dialog, never in chat).
- Postcondition (kind `provider_connection`): projection exposes
  `has_own_app_credentials` (bool derived from the row's presence) — predicate:
  `true` after the mutation. No secret-adjacent field names in the projection
  (`clientsecret` is forbidden; the boolean is the entire evidence).
- Negatives: unknown slug, provider that does not support user app credentials
  (server error → blocked), secret in params (schema-impossible; test).

### `provider.disconnect` — destructive (legacy surface)
- Params: `{providerSlug}`.
- Endpoint: `DELETE /api/v1/providers/{provider_id}/disconnect`
  (`handlers/user_tokens.rs::disconnect_provider`).
- Confirm dialog must state the blast radius: disconnecting the provider token affects
  every legacy connection using it (verify handler behavior for the exact copy).
- Postcondition (kind `provider_connection`): `connected == false` for this user +
  provider. Negatives: not-connected (blocked), stale slug, replay.

## Evidence projections you own

- `services.rs` (`kind=user_service`): WP-0A reference + `node_id`, `endpoint_id`,
  `credential_version`-correlate (per investigation), `auth_method` (closed enum
  string). Fully-baited test: `ws_frame_injections` template with
  `Bearer ${credential}`, `default_request_headers` with bait value, bait label — the
  A1 reproduction, must pass.
- `providers.rs` (`kind=provider_connection`, keyed by provider id, scoped to the
  session user): `provider_id`, `provider_slug`, `connected`, `connection_status`,
  `has_own_app_credentials`, `last_authorized_at`, `updated_at`.
- `connections.rs` (`kind=service_connection`): `service_id`, `connected`,
  `updated_at`.

## Acceptance criteria

Same bar as WP-2A: predicate-false harness case per verb, destructive confirm
every-time tests, baited projection tests, no timestamp-only completion predicates, no
edits outside owned files, full backend + frontend suites green.

## Test commands

```bash
source "$HOME/.cargo/env" 2>/dev/null
cargo test assistant_evidence && cargo test
npm --prefix frontend run test && npm --prefix frontend run lint && npm --prefix frontend run build
```
