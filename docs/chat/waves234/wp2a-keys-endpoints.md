# WP-2A — Wave 2 team package: keys, external keys, endpoints (8 verbs)

**Master plan:** `docs/chat/waves234-plan.md`. **Prerequisites merged:** WP-0A, WP-0B,
WP-2PM scaffolding (your descriptors, Zod schemas, `ActionCardParams` variants, journey
ids already exist — you implement journeys, evidence projections, and tests).

**Verbs:** `key.update`, `key.delete`, `key.extend_scope`, `key.bind_credential`,
`external_key.rotate`, `external_key.delete`, `endpoint.update`, `endpoint.delete`.

**Owned files (yours alone — never edit PM-owned files listed in master plan §4.1):**

- `backend/src/handlers/assistant_evidence/api_keys.rs`, `external_keys.rs`,
  `endpoints.rs` (+ their tests) — PM adds your three dispatch lines to `mod.rs`.
- `frontend/src/components/assistant/journeys/key-update.tsx`, `key-delete.tsx`,
  `key-extend-scope.tsx`, `key-bind-credential.tsx`, `external-key-rotate.tsx`,
  `external-key-delete.tsx`, `endpoint-update.tsx`, `endpoint-delete.tsx` + tests.
- Any new shared dialog you need under `frontend/src/components/assistant/journeys/`
  (do NOT edit dashboard pages/dialogs; import and reuse them, or build journey-local
  dialogs).

**Verify before building** (worktrees go stale): each endpoint below against
`origin/main` `backend/src/routes.rs` and the named handler; the WP-0B out-of-band
spike table (none of your verbs should need it — all eight are direct-mutation).

## Verb specifications

Resource-kind names used with the evidence route: `api_key` (NyxID `ApiKey`,
`nyxid_ag_` keys), `external_key` (`UserApiKey` external credentials), `user_endpoint`.

### `key.update` — grant
- Params: `{keyId, name?, platform?}` (params seed the dialog; user edits in-dialog).
- Endpoint: `PUT /api/v1/api-keys/{key_id}` (`handlers/api_keys.rs::update_key`).
  **Hard rule (pending master-plan §7 Q-G confirmation):** the journey must not send
  `allowed_service_ids` / `allow_all_services` / `allow_all_nodes` /
  `allowed_node_ids` — widening authority is `key.extend_scope`'s job. Strip these
  fields from whatever dialog you reuse; add a test asserting the PUT body never
  contains them.
- Journey: preflight `GET /api-keys/{key_id}` with the substrate secret-free read
  pattern → dialog (reuse the editing affordances from
  `frontend/src/pages/api-key-detail.tsx` as reference, not by import if it drags the
  page along) → `runDirectMutation`.
- Postcondition predicate (evidence kind `api_key`): `id == keyId`,
  `is_active == true`, `updated_at` present. Resource report: `{key: {keyId}}`.
- Negative fixtures: stale id (404 → blocked, never completed), unauthorized (other
  user's key id → blocked), secret-shaped param rejection (schema test — PM's Zod
  schema already rejects; add the journey-level test), replay (second submit of the
  same card → the dialog is closed; re-CTA runs a fresh preflight).

### `key.delete` — destructive
- Endpoint: `DELETE /api/v1/api-keys/{key_id}`.
- Journey: preflight read → **confirm-every-time dialog** (name the key by its server
  name from preflight, show platform + bindings count; destructive confirm pattern per
  DESIGN.md) → `runDirectMutation` → `reportCompletedAfterDeleteVerify` (evidence 404
  = success; WP-0A contract). Resource: `{key: {keyId}}`.
- Negative fixtures: delete of already-deleted id (404 preflight → blocked "already
  gone", never completed), unauthorized, replay (card cannot re-complete after
  resolution — dedupe comes from the existing action-report path; add the test).

### `key.extend_scope` — grant, never remember
- Params: `{keyId, addAllowedServiceIds[]}`.
- Endpoint: `PUT /api/v1/api-keys/{key_id}` with the widened
  `allowed_service_ids` = current ∪ requested. Read current via preflight
  `GET /api-keys/{key_id}`; compute the union client-side; show the delta explicitly
  ("this key will additionally reach: …", resolving slugs via single-entry
  `GET /keys/{id}` reads, not the whole catalog — A8 rule).
- Postcondition predicate: evidence `allowed_service_ids ⊇ requested` (Ordinal
  compare) ∧ `is_active`. This is the A2 lesson verbatim — the predicate checks the
  *effect*, not a timestamp.
- Negative fixtures: requested id the user cannot see (server rejects → blocked with
  the failing id), `allow_all_services` already true (journey blocks as a no-op with
  explanation rather than reporting completed), duplicate ids in params (Zod rejects).

### `key.bind_credential` — grant, never remember
- Params: `{keyId, serviceSlug, credentialLabel}`.
- Endpoint: `POST /api/v1/api-keys/{key_id}/bindings`
  (`handlers/agent_bindings.rs::create_binding`; body shape — verify against the
  handler: it maps `(api_key_id, user_service_id)` → override `user_api_key_id` per
  CLAUDE.md §9). Resolve `serviceSlug` → `user_service_id` and `credentialLabel` →
  `user_api_key_id` in the preflight (single-entry reads; blocked note listing valid
  labels is NOT allowed — that would enumerate credentials; block with "label not
  found" only).
- Postcondition predicate (evidence kind `api_key`): `bindings_count` increased is NOT
  sufficient (A3 — someone else's binding could land); the mutation response carries
  the created binding id — evidence projection for `api_key` must include
  `binding_ids` (ids only) and the predicate checks the new id is present.
- Negative fixtures: unknown slug, unknown label, duplicate binding (server error →
  blocked), unauthorized key.

### `external_key.rotate` — grant (secret-bearing journey)
- Params: `{externalKeyId}`.
- Endpoint: `PUT /api/v1/api-keys/external/{id}`
  (`handlers/user_api_keys_external.rs::update_external_api_key`) with the
  **replacement upstream credential typed into the browser dialog** — the secret exists
  only in the dialog state and the PUT body; never in params, reports, or logs. CLI
  parity: `cli/src/commands/external_key.rs` `rotate` (its wiremock tests show the
  body shape — `credential` in body).
- Postcondition predicate (evidence kind `external_key`): identity match ∧ evidence
  carries a rotation-correlate. `TODO — not investigated:` whether the PUT response or
  the `UserApiKey` row exposes a usable correlate (key preview/hash suffix/updated_at
  alone is A3-weak). Investigate the handler; if none exists, add
  `credential_version`-style monotonic counter to the evidence projection from the
  row's update — do not fall back to bare `updated_at` advancement without recording it
  as `baseline-only` in the journey table.
- Negative fixtures: empty credential (client rejects before PUT — CLI has the same
  rule), secret pasted into any params-visible field (impossible by schema; test the
  dialog does not echo the secret into the DOM after submit), stale id.

### `external_key.delete` — destructive
- Endpoint: `DELETE /api/v1/api-keys/external/{id}`. Note the CLI supports
  `--keep-upstream` semantics via token scope flags — the journey exposes the same
  choice **in-dialog** if the handler supports it (verify; otherwise plain delete).
- Journey/postcondition/negatives: same delete pattern as `key.delete`, evidence kind
  `external_key`. Warn in-dialog when services currently reference this credential
  (preflight `GET /keys?…` filtered server-side if such a filter exists; if not,
  `TODO — not investigated`, omit the warning rather than fetching everything).

### `endpoint.update` — grant
- Params: `{endpointId}`; edits (URL etc.) in-dialog.
- Endpoint: `PUT /api/v1/endpoints/{endpoint_id}`
  (`handlers/user_endpoints.rs::update_endpoint`). URL inputs use the same https-only
  validation as the custom-service connect journey (`safeEndpointUrl` pattern in
  `action-registry.ts` — copy the rules, not the function, it is PM-owned).
- Postcondition (evidence kind `user_endpoint`): identity ∧ `is_active`-equivalent ∧
  the mutation response echoing the new URL host (evidence projection carries
  `endpoint_host` — host only, never full URL with path/query, which can embed
  secrets).
- Negative fixtures: http:// URL rejected client-side; URL with userinfo/query
  rejected; stale id; unauthorized.

### `endpoint.delete` — destructive
- Endpoint: `DELETE /api/v1/endpoints/{endpoint_id}`. Delete pattern as above,
  evidence kind `user_endpoint`. In-dialog warning when services reference the
  endpoint (same caveat as external_key.delete).

## Evidence projections you own (WP-0A rules apply — read that brief)

- `api_keys.rs` (`kind=api_key`): extends the WP-0A reference projection; ensure
  `binding_ids`, `allowed_service_ids`, `allow_all_services`, `allow_all_nodes`
  present. Fully-baited test: name + platform set to `Bearer …` bait; projection must
  not contain them (`name_present` boolean only).
- `external_keys.rs` (`kind=external_key`): `id`, `is_active`, `status`,
  `credential_type`, `connection_status`, `granted_scopes`, `last_authorized_at`,
  `created_at`, `updated_at` (+ rotation correlate per the investigation above).
  Baited test: label/token_scopes bait.
- `endpoints.rs` (`kind=user_endpoint`): `id`, `is_active`-equivalent, `endpoint_host`,
  `from_catalog` (bool), `created_at`, `updated_at`. Baited test: URL containing
  userinfo + `Bearer` bait in any free-text model field.

## Acceptance criteria

- Every verb: journey test suite includes the harness `expectNeverCompletes`
  predicate-false case + the listed negative fixtures; destructive verbs show the
  confirm dialog every time (test re-invocation).
- Backend: `cargo test assistant_evidence` green with baited fixtures;
  full `cargo test` green (replica set required).
- Frontend: `test`, `lint` (restricted-syntax rule — no direct
  `report("completed"`), `build` (tsc -b) all green.
- No edits outside your owned-file list (PM enforces at review).

## Test commands

```bash
source "$HOME/.cargo/env" 2>/dev/null
cargo test assistant_evidence && cargo test
npm --prefix frontend run test && npm --prefix frontend run lint && npm --prefix frontend run build
```
