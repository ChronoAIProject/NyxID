# Issue #1307 Assistant Readiness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: `superpowers:test-driven-development`, `superpowers:verification-before-completion`, `biz-sdd-workflow`, and `biz-server-action`. Steps use checkbox (`- [ ]`) syntax for tracking. Encountering behavior not specified here requires returning to the design instead of inventing it.

**Goal:** Implement the authenticated NyxID readiness contract in `docs/superpowers/specs/2026-08-01-assistant-readiness-design.md` for M1-R01, M1-R02, M1-R03, and M1-R11.

**Architecture:** Add one focused service that composes existing safe service/key/approval evidence and one thin handler on the existing human-only assistant router. Reuse installed SHA-256, URL, MongoDB, serde, chrono, and current authorization services. Add no storage, framework, dependency, background job, or mutation.

**Tech Stack:** Rust 2024, Axum 0.8, MongoDB 3.5/BSON 2.15, serde/serde_json, chrono, sha2 0.10, url 2.5, Tokio.

## Global Constraints

- `AuthUser.user_id` is the sole caller identity; request query/body values never select a user or scope.
- Core evidence checks the exact `aevatar` runtime row and exact `chrono-llm-public` assistant model row; it never selects an arbitrary `llm-*` service.
- Preserve `status`, `connectionState`, and `grantState` as separate closed enums.
- Database failure stays an error; unknown evidence becomes `cannot_check`; authoritative denial becomes `cannot_use`.
- `connected` is available only with `granted` or `not_required`.
- Management URLs come only from HTTPS `AppConfig.frontend_url` and use `/keys`.
- The response and fixture contain no credential, token, cookie, authorization, provider payload, endpoint URL, owner/requester identity, or secret-shaped value.
- The endpoint is read-only and introduces no transaction, idempotency key, table, model, or migration.
- Every behavior change follows RED, GREEN, REFACTOR: observe the intended failure before implementation.

## Visual Baseline

None. This issue adds no UI or page.

## Requirement Coverage

| Requirement | Deliverable |
| --- | --- |
| M1-R01 | One snapshot with required model/runtime, optional visible connectors, revision, and evaluated time |
| M1-R02 | Separate closed connection and grant evidence with fail-closed status derivation |
| M1-R03 | HTTPS NyxID `/keys` management URL and no secret-bearing response fields |
| M1-R11 | Authenticated user scope, dedicated DTO, safe errors, stable digest, and contract tests |

## File Map

| Path | Rung | Responsibility | R7 reason |
| --- | --- | --- | --- |
| `backend/src/services/assistant_readiness_service.rs` | R4 | Extend the assistant domain with the smallest read-model composer using existing services | Not applicable |
| `backend/src/services/approval_service.rs` | R4 | Add a read-only org-aware policy/grant summary used by readiness instead of duplicating approval semantics | Not applicable |
| `backend/src/services/mod.rs` | R4 | Export the service module | Not applicable |
| `backend/src/handlers/assistant.rs` | R4 | Add the thin authenticated GET handler and handler-level serialization tests | Not applicable |
| `backend/src/routes.rs` | R4 | Mount the handler on the existing human-only assistant router | Not applicable |
| `backend/fixtures/assistant-readiness/v1.json` | R7 | Versioned cross-repository contract evidence has no existing fixture | Required consumer boundary |

## Deferred Simplifications

| Ceiling | Business Trigger | Backport Skill |
| --- | --- | --- |
| Connectors are optional without a typed required set | Aevatar publishes task-specific required capability IDs/scopes | none |

---

### Task 1: Define and verify the pure readiness contract

**Files:**

- Create: `backend/src/services/assistant_readiness_service.rs`
- Modify: `backend/src/services/mod.rs`

**Interfaces:**

```rust
pub enum ReadinessStatus { Available, Missing, CannotUse, CannotCheck }
pub enum ConnectionState { NotConnected, Connecting, Verifying, Connected, Expired, Revoked, Unknown }
pub enum GrantState { NotRequired, Granted, Partial, Missing, Expired, Revoked, Unknown }
pub struct ReadinessCapability { /* dedicated safe fields from the design */ }
pub struct ReadinessSnapshot { pub revision: String, pub evaluated_at: DateTime<Utc>, pub capabilities: Vec<ReadinessCapability> }
pub fn build_snapshot(capabilities: Vec<ReadinessCapability>, evaluated_at: DateTime<Utc>) -> AppResult<ReadinessSnapshot>;
```

- [x] **Step 1: Write failing pure contract tests**

Cover literal state/status tables, duplicate fail-closed aggregation, empty task-request scopes, stable digest excluding time, HTTPS management URL construction, and JSON field names/secret absence.

Run: `cargo test -p nyxid assistant_readiness_service::tests -- --nocapture`

Expected: FAIL because `assistant_readiness_service` and its types/functions do not exist.

- [x] **Step 2: Implement the minimum pure contract**

Use serde snake_case enums, a dedicated camelCase DTO shape, `url::Url` for HTTPS validation, stable sort, `serde_json::to_vec`, and installed `sha2`. Keep mapping helpers private unless Task 2 consumes them.

- [x] **Step 3: Verify GREEN and commit**

Run: `cargo test -p nyxid assistant_readiness_service::tests -- --nocapture && cargo fmt --check`

Expected: all pure readiness tests pass and formatting is clean.

```bash
git add backend/src/services/assistant_readiness_service.rs backend/src/services/mod.rs
git commit -m "feat(assistant): define readiness contract"
```

---

### Task 2: Compose authoritative user and approval evidence

**Files:**

- Modify: `backend/src/services/assistant_readiness_service.rs`
- Modify: `backend/src/services/approval_service.rs`

**Interfaces:**

```rust
pub enum ApprovalReadiness { NotRequired, Granted, Partial, Missing, Expired, Revoked, Denied, Unknown }
pub async fn summarize_approval_readiness(
    db: &mongodb::Database,
    actor_user_id: &str,
    service_owner_user_id: &str,
    service_id: &str,
    requester_type: &str,
    requester_id: &str,
    now: DateTime<Utc>,
) -> AppResult<ApprovalReadiness>;

pub async fn evaluate_readiness(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    user_id: &str,
    frontend_url: &str,
    evaluated_at: DateTime<Utc>,
) -> AppResult<ReadinessSnapshot>;
```

- [x] **Step 1: Write failing Mongo-backed service tests**

First add literal policy/grant-row tests for no-approval, unscoped grant, scoped partial grant, missing, expired, revoked, deny, and rule-dependent unknown. Then insert the exact `aevatar` and `chrono-llm-public` core rows plus user/key/endpoint/catalog/approval fixtures. Assert: only the requested user's visible services appear; inactive or missing core is `missing`; misconfigured core is `cannot_use`; core grant is `not_required`; connector grants use delegated requester `aevatar`; role denial is `cannot_use`; absent or conflicting evidence is `cannot_check`.

Run: `cargo test -p nyxid assistant_readiness_service::tests::evaluate_ -- --nocapture`

Expected: FAIL because `evaluate_readiness` and database composition do not exist.

- [x] **Step 2: Implement the minimum service composition**

Refactor the effective org/actor policy-source selection inside `approval_service` once, keeping existing operation evaluation behavior unchanged, then add the read-only summary there. A non-empty rule set is `Unknown` because no task operation descriptor exists; a simple all-service grant is `Granted` and a scoped grant is `Partial`. Reuse `assistant_service` for the Aevatar admin row, `unified_key_service::list_keys` for safe visible connector views, and `proxy_service` metadata lookup for the same effective owner/service ID the proxy selects. Never serialize `KeyView` or model structs.

- [x] **Step 3: Verify GREEN and commit**

Run: `cargo test -p nyxid assistant_readiness_service::tests -- --nocapture && cargo fmt --check`

Expected: pure and Mongo-backed readiness service tests pass; a missing local Mongo is reported only through the repository existing skip guard.

```bash
git add backend/src/services/assistant_readiness_service.rs backend/src/services/approval_service.rs
git commit -m "feat(assistant): evaluate readiness evidence"
```

---

### Task 3: Publish the authenticated endpoint and fixture

**Files:**

- Modify: `backend/src/handlers/assistant.rs`
- Modify: `backend/src/routes.rs`
- Create: `backend/fixtures/assistant-readiness/v1.json`

**Interfaces:**

```rust
pub async fn readiness(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<ReadinessSnapshot>>;
```

- [ ] **Step 1: Write failing handler, route, and fixture tests**

Assert the handler uses the authenticated UUID, serializes exact DTO keys, sets no secret fields, the real router mounts `GET /api/v1/assistant/readiness` inside the human-only class, and the versioned fixture parses into `ReadinessSnapshot` while covering every enum value.

Run: `cargo test -p nyxid assistant_readiness -- --nocapture`

Expected: FAIL because the handler, route, and fixture do not exist.

- [ ] **Step 2: Implement the thin handler, route, and fixture**

The handler converts `auth_user.user_id` to a string, passes config and clock to the service, and returns `Json`. Apply an explicit exempt billing classification because it performs no downstream request.

- [ ] **Step 3: Verify GREEN and commit**

Run: `cargo test -p nyxid assistant_readiness -- --nocapture && cargo fmt --check`

Expected: service, handler, route, and fixture tests pass with the production path registered.

```bash
git add backend/src/handlers/assistant.rs backend/src/routes.rs backend/fixtures/assistant-readiness/v1.json
git commit -m "feat(assistant): publish readiness snapshot"
```

---

### Task 4: Run release verification and synchronize the consumer

**Files:**

- Modify in the separate `nyxid-chat` worktree: its upstream fixture and readiness contract test only.

- [ ] **Step 1: Copy the committed NyxID fixture unchanged and make the consumer test read it**

Run the consumer test before changing its fixture path.

Expected: FAIL because the upstream versioned fixture is not yet present in `nyxid-chat`.

- [ ] **Step 2: Verify both repositories**

Run: `cargo fmt --check && cargo test --manifest-path backend/Cargo.toml`

Run in `nyxid-chat`: `npm test`

Expected: all tests pass with zero failures and the consumer reads the exact NyxID fixture.

- [ ] **Step 3: Read back evidence and update both GitHub issues**

Use body files for comments. Read back the new comments and links before changing issue state. Do not close the milestone mirror until the production endpoint and consumer fixture are both linked.

## Final Review Checklist

- [ ] M1-R01/R02/R03/R11 mappings are each protected by a behavior test.
- [ ] `connected + missing/partial/expired/revoked/unknown` is never available.
- [ ] Database failures do not become `missing`.
- [ ] Only HTTPS configured NyxID `/keys` URLs are serialized.
- [ ] Secret scan finds no secret-shaped field or value in response or fixture.
- [ ] `git status --short` contains no generated artifacts, database files, or unrelated changes.
- [ ] Focused tests, `cargo fmt --check`, backend full tests, and `nyxid-chat npm test` are fresh and green.

## Plan Self-Review

- Design sections 1–8 map to Tasks 1–4; all public names are consistent across tasks.
- There are no write paths, persistence changes, UI files, placeholders, or open implementation decisions.
- The only deferred behavior is the explicitly non-authoritative task-specific required set.

## Execution Handoff

Execute Tasks 1–3 in `/Users/eanzhao/Code/.worktrees/nyxid-m1-readiness`, then Task 4 in `/Users/eanzhao/Code/nyxid-chat-m1-chat-execution-loop`. Stop on a repeated verification failure and diagnose the failing test before changing its assertion.
