# RBAC/OIDC Code Review & Security Review

**Reviewer:** code-reviewer + security-reviewer agents
**Date:** 2026-02-11
**Scope:** All new and modified files for RBAC, Groups, Consents, and OIDC enhancements

---

## CRITICAL

### 1. Token Introspection Endpoint Has No Caller Authentication
**File:** `backend/src/handlers/oauth.rs` (introspect handler, ~line 563)
**Issue:** RFC 7662 requires that the introspection endpoint authenticate the calling client (via client_id/client_secret). The current implementation accepts any POST with a `token` field and returns active token metadata to the caller without verifying who is asking. An attacker who obtains any valid token string can query full token metadata (user ID, scopes, roles, expiration).
**Fix:** Validate `client_id` and `client_secret` from the request body against the `oauth_clients` collection before returning introspection results. Return `{"active": false}` for unauthenticated or unauthorized callers.

### 2. Token Revocation Endpoint Has No Caller Authentication
**File:** `backend/src/handlers/oauth.rs` (revoke handler, ~line 637)
**Issue:** RFC 7009 requires that the revocation endpoint authenticate the calling client. The current implementation allows any caller to revoke any token by value. A malicious actor who intercepts a token can revoke it to cause denial of service for the legitimate user.
**Fix:** Validate `client_id` and `client_secret` from the request body. Only allow a client to revoke tokens that were issued to it (or the resource owner to revoke their own tokens).

### 3. RBAC Data Is Never Injected Into Tokens
**File:** `backend/src/services/token_service.rs` (line ~59, ~179) and `backend/src/services/oauth_service.rs` (line ~260)
**Issue:** Both `create_session_and_issue_tokens` and `refresh_tokens` in token_service.rs pass `None` for the `rbac` parameter to `generate_access_token`. Similarly, `exchange_authorization_code` in oauth_service.rs passes `None`. Despite the full RBAC infrastructure being built (models, services, JWT claim fields, scope filtering), **no token ever contains roles, groups, or permissions claims**. The entire RBAC-in-tokens feature is non-functional.
**Fix:** In `create_session_and_issue_tokens`, `refresh_tokens`, and `exchange_authorization_code`, call `rbac_helpers::resolve_user_rbac(db, user_id).await?` to build `RbacClaimData` and pass `Some(&rbac_data)` to `generate_access_token`. Apply scope filtering to determine which RBAC fields to include.

---

## HIGH

### 4. `require_admin` Only Checks `is_admin` Flag, Not Role Assignment
**Files:** `backend/src/handlers/admin_roles.rs` (~line 91), `backend/src/handlers/admin_groups.rs` (~line 110)
**Issue:** The plan states that admin access should check both the `is_admin` flag AND the "admin" role assignment. The current `require_admin` helper only checks `is_admin`. This means if the system migrates to pure RBAC-based admin access in the future, the flag-only check creates a divergence.
**Fix:** Also check whether the user has the "admin" role slug via `rbac_helpers::resolve_user_rbac`. At minimum, document that `is_admin` is the canonical admin check and the role is informational.

### 5. Duplicated Helper Functions Across Admin Handlers
**Files:** `backend/src/handlers/admin_roles.rs` and `backend/src/handlers/admin_groups.rs`
**Issue:** `require_admin`, `extract_ip`, `extract_user_agent`, and response-building helpers are duplicated verbatim between these two modules. This violates DRY and increases the risk of the two copies diverging (e.g., one gets a security fix but the other doesn't).
**Fix:** Extract shared helpers into a common module (e.g., `handlers/admin_helpers.rs` or `handlers/common.rs`) and import from both.

### 6. `create_group` Does Not Validate That `role_ids` Exist
**File:** `backend/src/services/group_service.rs` (create_group function)
**Issue:** When creating a group with `role_ids`, the service inserts the group document without verifying that the referenced role IDs actually exist in the `roles` collection. This can lead to groups referencing non-existent roles, which will silently produce incomplete RBAC resolution.
**Fix:** Before insert, query the `roles` collection to verify all provided `role_ids` exist. Return an error if any are missing.

### 7. `update_group` Does Not Validate `role_ids` or `parent_group_id` Exist
**File:** `backend/src/services/group_service.rs` (update_group function)
**Issue:** Similar to create, the update path does not validate that new `role_ids` reference existing roles or that a new `parent_group_id` references an existing group (beyond the circular hierarchy check).
**Fix:** Validate referenced IDs exist before applying the update.

---

## MEDIUM

### 8. Token Endpoint Hardcodes Scope in Response
**File:** `backend/src/handlers/oauth.rs` (~line 471)
**Issue:** The token endpoint response always returns `scope: "openid profile email"` regardless of what scopes were actually requested or consented to. This misrepresents the actual token scope and can confuse relying parties.
**Fix:** Return the actual granted scopes from the authorization request / consent record.

### 9. Consent Expiration Is Never Checked
**File:** `backend/src/services/consent_service.rs` (check_consent function)
**Issue:** The `Consent` model has an `expires_at: Option<DateTime<Utc>>` field, but `check_consent` only verifies scope coverage. It never checks whether the consent has expired. An expired consent will still be treated as valid.
**Fix:** In `check_consent`, if `expires_at` is `Some(t)` and `t < Utc::now()`, treat the consent as not granted.

### 10. No Pagination on List Endpoints
**Files:** `backend/src/services/role_service.rs` (list_roles), `backend/src/services/group_service.rs` (list_groups, get_members)
**Issue:** List endpoints fetch all documents from the collection without pagination. For deployments with many roles, groups, or group members, this could cause performance issues and excessive memory usage.
**Fix:** Add `skip`/`limit` parameters (or cursor-based pagination) to list operations. Expose `page`/`per_page` query parameters in handlers.

### 11. `get_user_roles` Fetches All Roles to Compute Inherited Roles
**File:** `backend/src/handlers/admin_roles.rs` (get_user_roles handler, ~line 270)
**Issue:** To compute inherited roles from groups, this handler fetches ALL roles from the database and filters in-memory. This is O(total_roles) regardless of how many the user actually has.
**Fix:** Use a targeted query with `$in` filter on the inherited role IDs instead of fetching all roles.

### 12. Missing Input Validation on Role/Group Slug Format in Services
**Files:** `backend/src/services/role_service.rs`, `backend/src/services/group_service.rs`
**Issue:** While the frontend Zod schema enforces slug format (`/^[a-z0-9_-]+$/`), the backend services do not validate slug format. A direct API caller (bypassing frontend) could create roles/groups with arbitrary slug strings (including spaces, special characters).
**Fix:** Add slug format validation in the service layer (or via an Axum extractor/middleware).

### 13. System Role Protection Is Incomplete
**File:** `backend/src/services/role_service.rs` (delete_role, update_role)
**Issue:** `delete_role` correctly checks `is_system` and refuses to delete system roles. However, `update_role` does not prevent modification of system role fields (name, slug, permissions). A caller could rename or change permissions of the "admin" system role.
**Fix:** In `update_role`, refuse changes to `name`, `slug`, and potentially `permissions` for system roles, or reject the update entirely for system roles.

### 14. `delete_group` Orphans Children Instead of Blocking Deletion
**File:** `backend/src/services/group_service.rs` (delete_group function)
**Issue:** When a group with children is deleted, the service sets `parent_group_id: null` on all children. This silently orphans them without warning the caller. Some deployments may prefer to block deletion of groups that have children.
**Fix:** Consider returning an error if the group has children, requiring the caller to either move or delete children first. Or at minimum, return metadata about orphaned children in the response.

---

## LOW

### 15. `AuthUser.scope` Is Empty String for Non-Bearer Auth
**File:** `backend/src/mw/auth.rs`
**Issue:** For session-based and API key authentication, `scope` is set to an empty string. This means scope-dependent logic (like RBAC claim filtering in userinfo) will never include RBAC data for session-authenticated users. This may be intentional but should be documented.
**Recommendation:** Add a comment explaining the design decision, or consider defaulting to a full scope set for session auth.

### 16. Circular Hierarchy Check Has Hard-Coded Depth Limit
**File:** `backend/src/services/group_service.rs` (check_circular_hierarchy)
**Issue:** The max depth is hardcoded to 10. While reasonable, this is not configurable and not documented to API consumers.
**Recommendation:** Extract to a constant with a descriptive name, and document the limit in API docs or error messages.

### 17. Frontend Permissions Stored as Comma-Separated String
**File:** `frontend/src/schemas/rbac.ts`
**Issue:** The form schema stores permissions as a comma-separated string that gets split into an array on submit. This works but makes validation of individual permission strings harder (e.g., no whitespace validation per permission).
**Recommendation:** Consider using a tag-input or multi-select component for permissions, or add per-item validation after splitting.

### 18. Group Detail Page Adds Members by Raw User ID Input
**File:** `frontend/src/pages/admin-group-detail.tsx`
**Issue:** Adding a member requires typing a raw user ID (UUID). This is error-prone and provides no user feedback if the ID is invalid or doesn't exist (the backend will return an error, but a user-search dropdown would be better UX).
**Recommendation:** Add a user search/autocomplete component for member addition.

### 19. No Loading/Error States for Role Assignment in User Detail
**File:** `frontend/src/pages/admin-user-detail.tsx`
**Issue:** The role assignment dropdown and revoke buttons show toast on error but don't disable during mutation, allowing double-clicks or rapid repeated actions.
**Recommendation:** Disable action buttons while mutations are pending using the `isPending` state from `useMutation`.

### 20. OIDC Discovery Lists Scopes That May Not Be Fully Implemented
**File:** `backend/src/handlers/oidc_discovery.rs`
**Issue:** The discovery document advertises `roles`, `groups`, and `permissions` scopes in `scopes_supported`, but since RBAC data is never injected into tokens (see CRITICAL #3), these scopes are non-functional.
**Recommendation:** Either implement the token injection (see CRITICAL #3 fix) or remove these scopes from discovery until implemented.

---

## Summary

| Severity | Count |
|----------|-------|
| CRITICAL | 3     |
| HIGH     | 4     |
| MEDIUM   | 7     |
| LOW      | 6     |

### Priority Fix Order
1. **CRITICAL #3** - Wire up RBAC data in token generation (core feature is non-functional without this)
2. **CRITICAL #1 & #2** - Add client authentication to introspection and revocation endpoints
3. **HIGH #5** - Extract duplicated helpers (reduces risk of divergent fixes)
4. **HIGH #6 & #7** - Add referential integrity checks for role_ids and parent_group_id
5. **HIGH #4** - Align admin check with plan (flag + role)
6. **MEDIUM #8** - Fix hardcoded scope in token response
7. **MEDIUM #9** - Check consent expiration
8. **MEDIUM #13** - Protect system roles from modification
9. Remaining MEDIUM and LOW items

### Positive Observations
- Model layer follows all MongoDB conventions correctly (bson datetime helpers, no skip_serializing, COLLECTION_NAME constants)
- Error variants are well-structured with unique codes and appropriate HTTP status mappings
- Frontend types are properly readonly with comprehensive Zod schemas
- TanStack Query hooks follow existing patterns with correct cache invalidation
- Consent handler correctly uses `auth_user.user_id` preventing IDOR
- Group circular hierarchy detection is sound (walks parent chain with depth limit)
- Index definitions in db.rs are comprehensive (unique slugs, compound indexes for consent)
- System role seeding at startup is idempotent (checks existence before insert)
- Frontend disables editing name/slug for system roles (good UX protection)
