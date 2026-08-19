# WP-0A — Backend evidence-projection substrate

**Master plan:** `docs/chat/waves234-plan.md` (§3.1, §3.5). Read its §1 ground-truth
section first. This brief is self-contained for a fresh agent; where it cites line
numbers, re-verify on `origin/main` before editing — worktrees here go stale.

**Mission.** Land the shared secret-free evidence read surface that every Wave-2/3/4
verb's postcondition consumes, plus the golden secret-scan test helper that makes the
Wave-1 A1 defect class structurally unrepeatable. No verbs ship in this package.

**Depends on:** nothing. **Blocks:** all wave team packages.

## Why this exists (context you must not re-litigate)

Aevatar's evidence parser (`NyxIdApiAccessContracts.cs` on
`aevatarAI/aevatar` `origin/feature/integrate`) recursively walks the entire read-back
JSON and hard-fails (`ProviderReadFailure`) if any field's lowercased-alphanumeric name
is in a forbidden set (`apikey, fullkey, keyhash, credential(s), accesstoken,
refreshtoken, authorization, cookie(s), secret(s), clientsecret, password, token,
passphrase, usercode, devicecode, rawbody, rawupstreambody`) or any string value matches
`(?:Bearer\s+\S+|nyxid_(?:ag_)?[A-Za-z0-9_-]{16,})` (case-insensitive). Wave 1 pointed
that scanner at the full `GET /api/v1/keys/{id}` `KeyResponse` and got MAJOR defect A1:
user-controlled `Bearer …` strings in `ws_frame_injections[].template`,
`default_request_headers[].value`, or labels permanently brick the read. The fix
pattern (agreed in `docs/chat/wave1-service-reauthorize-actions.md` §A1) is a minimal
evidence projection. This package generalizes it to all families.

## Deliverables

### 1. Route + dispatcher (the only `routes.rs` edit any wave will ever need)

- `GET /api/v1/assistant/evidence/{kind}/{id}` mounted inside the human-only API group
  (same nest level as `/keys`). `{kind}` is a closed set parsed to an enum — unknown
  kind → 404-shaped `AppError::NotFound`, never a list of valid kinds.
- Auth posture: session JWT and delegated `account:read` GET both pass (evidence is
  secret-free by construction). Do **not** add to `delegated_read_denied_path`. Relay
  tokens and agent API keys remain rejected by the existing human-only middleware —
  verify by test, mirroring `delegated_account_read_allows_expected_management_families`
  (`mw/auth.rs`).
- Files:
  - `backend/src/handlers/assistant_evidence/mod.rs` — kind enum, dispatch, shared
    envelope. After this package merges, `mod.rs` is PM-owned (one dispatch line per
    family); family files are team-owned.
  - `backend/src/handlers/assistant_evidence/keys.rs` and `services.rs` — the two
    reference projections (below).
  - `backend/src/routes.rs` — the single route registration.

### 2. Response envelope (uniform across kinds)

```json
{
  "kind": "user_service",
  "id": "<uuid>",
  "as_of": "<rfc3339 — server time of the read>",
  "evidence": { …kind-specific, flat, whitelisted… }
}
```

Rules for every `evidence` object (enforce with the §4 test helper, restate in the
module doc comment):

- Only: stable ids, booleans, status strings from closed model enums, counts, RFC3339
  timestamps (`chrono to_rfc3339()`), and slugs already validated by NyxID's own slug
  patterns. **Never** user-authored free text (names, labels, templates, headers, URLs,
  notes) — identity is proven by ids, not labels.
- Every field always serialized — no `skip_serializing_if` anywhere in evidence structs
  (`null` for absent optionals; Aevatar's parsers require property presence).
- No field name whose ASCII-lowercased-alphanumeric normalization lands in the
  forbidden set above. `token_scopes` → name it `granted_scopes`; anything ending in
  `_token`/`_secret` must be renamed or dropped.
- Deleted/absent resource → the handler returns 404-shaped NotFound **after** the ACL
  check on the id's owner cannot resolve; this is deliberate and documented: for
  `*.delete` verbs, Aevatar's postcondition treats confirmed-404-under-valid-auth as
  the success evidence (cross-repo contract point; the per-wave Aevatar issue drafts in
  the PM briefs carry it).

### 3. Reference projections

`keys.rs` — `kind=user_service` (mirrors what Aevatar's reauthorize verify consumes,
per `wave1-service-reauthorize-plan.md` §1.3/§1.4):
`id`, `api_key_id`, `is_active`, `status` (six-value domain from
`models/user_api_key.rs`), `connection_status`, `granted_scopes` (deduped,
first-occurrence order), `last_authorized_at`, `node_id`, `endpoint_id`, `owner_kind`
(`"user"`/`"org"` — derived, not the raw user_id of an org), `updated_at`.

`services.rs` — `kind=api_key` (for `key.*` verbs; backing model `ApiKey`):
`id`, `name_present` (bool — NOT the name), `platform`, `is_active`,
`allowed_service_ids`, `allow_all_services`, `allow_all_nodes`, `allowed_node_ids`,
`bindings_count`, `rotated_from_id`, `created_at`, `updated_at`.
`TODO — not investigated:` exact `ApiKey` field names for rotation lineage — grep
`models/api_key.rs` for the predecessor/successor fields the rotate flow writes and use
those; do not invent.

ACL: resolve exactly as the underlying family handler does
(`resolve_key_read_owner` / `org_service::resolve_owner_access` pattern); unauthorized
→ NotFound-shaped, no metadata leak.

### 4. Golden secret-scan test helper

`backend/src/test_utils` (or a `#[cfg(test)]` module the family files import):

```rust
pub fn assert_evidence_secret_free(value: &serde_json::Value)
```

- Recursively checks every property name against the normalized forbidden set and every
  string value against the tripwire regex — a faithful port of Aevatar's scanner, with
  a comment pinning the source (`NyxIdApiAccessContracts.cs`).
- Companion requirement (this is the A1 test lesson — a scan over an empty struct
  proves nothing): each family module must expose a `fully_baited_fixture()` used by
  its tests — every `Option` populated, every list non-empty, and every field that is
  even indirectly user-influenced set to bait (`Bearer ${credential}`,
  `nyxid_ag_0123456789abcdef`). The keys.rs reference test must construct the
  underlying model with a `ws_frame_injections` template containing
  `"Authorization":"Bearer ${credential}"` and prove the projection still passes —
  i.e., prove the projection *omits* the field rather than sanitizes it.

### 5. Docs

Add an "Assistant evidence reads" subsection to `docs/chat/06-actions-registry.md`
(coordinate with the PM — that file is PM-owned; hand them the text) covering the
route, the envelope, the field rules, and the 404-means-deleted contract.

## Acceptance criteria

- `cargo test assistant_evidence` green; the baited keys test fails if anyone re-adds a
  user-authored field (verify by mutation: temporarily add `label` to the projection,
  watch the test fail, remove it).
- Route test: 200 + envelope for owner session; 404 for other-user id; 404 for unknown
  kind; delegated `account:read` GET passes; agent API key rejected.
- `cargo fmt --check`, `clippy -D warnings`, full `cargo test` (needs MongoDB replica
  set + `NYXID_TEST_DATABASE_URL`; thousands of instant failures = connection problem,
  not your regression).
- No frontend changes in this package.

## Test commands

```bash
source "$HOME/.cargo/env" 2>/dev/null
cargo test assistant_evidence
cargo test                      # full suite, replica set required
cargo fmt --check && cargo clippy --all-targets -- -D warnings
```
