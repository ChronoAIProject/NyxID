# Implementation Plan: RBAC and Enhanced OIDC for NyxID

## Overview

Add Keycloak-like Role-Based Access Control (RBAC) and enhanced OIDC features to NyxID. This includes new MongoDB collections for roles, groups, and consents; enhanced JWT claims with roles/groups/permissions; new admin endpoints for managing roles and groups; token introspection (RFC 7662) and revocation (RFC 7009); and frontend admin pages for managing RBAC entities.

## Architecture Decisions

### 1. Separate Collections vs Embedded Documents

**Decision: Separate collections for roles, groups, and consents. Embed `role_ids` and `group_ids` arrays in the User document.**

Rationale:
- Roles and groups are first-class entities that need independent CRUD and querying
- Embedding `role_ids` / `group_ids` in User gives fast single-query user lookups at login time
- Consent records are per-user-per-client and can grow unbounded -- separate collection with TTL indexes
- This hybrid approach avoids N+1 queries at token time while keeping the role/group catalog independent

### 2. Native RBAC vs casbin-rs

**Decision: Native RBAC for v1.** casbin-rs adds policy DSL overhead with minimal benefit at this scale. The permission model is simple: roles have permissions (string tags), groups inherit roles. We can evaluate casbin later if complex policy rules emerge.

### 3. Claim Mapper Approach

**Decision: Hardcoded claim mappers in v1.** A `ClaimMapper` trait can be introduced later for configurable mappers. For now, the token service directly injects roles/groups/permissions into JWT claims based on requested scopes.

### 4. Default Roles Strategy

**Decision:** Two system roles seeded at startup:
- `admin` (system role, maps to `is_admin = true`)
- `user` (system role, default for all new users)

Existing `is_admin` flag is preserved for backwards compatibility. The `require_admin()` check in handlers will be updated to also accept users with the `admin` system role.

---

## Data Models

### New Model: `Role` (collection: `roles`)

**File:** `backend/src/models/role.rs`

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "roles";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Role {
    #[serde(rename = "_id")]
    pub id: String,
    /// Human-readable name (e.g., "Admin", "Editor")
    pub name: String,
    /// URL-safe slug for API references (e.g., "admin", "editor")
    pub slug: String,
    /// Optional description
    pub description: Option<String>,
    /// List of permission strings (e.g., ["users:read", "users:write"])
    pub permissions: Vec<String>,
    /// If true, auto-assigned to all new users
    pub is_default: bool,
    /// System roles cannot be deleted or renamed (e.g., "admin", "user")
    pub is_system: bool,
    /// If Some, this role is scoped to a specific OAuth client
    pub client_id: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}
```

### New Model: `Group` (collection: `groups`)

**File:** `backend/src/models/group.rs`

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "groups";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    #[serde(rename = "_id")]
    pub id: String,
    /// Human-readable name (e.g., "Engineering")
    pub name: String,
    /// URL-safe slug (e.g., "engineering")
    pub slug: String,
    /// Optional description
    pub description: Option<String>,
    /// Roles inherited by all members of this group
    pub role_ids: Vec<String>,
    /// Optional parent group ID for hierarchy (None = top-level)
    pub parent_group_id: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}
```

### New Model: `Consent` (collection: `consents`)

**File:** `backend/src/models/consent.rs`

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::bson_datetime;

pub const COLLECTION_NAME: &str = "consents";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Consent {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub client_id: String,
    /// Space-separated list of granted scopes
    pub scopes: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub granted_at: DateTime<Utc>,
    #[serde(default, with = "bson_datetime::optional")]
    pub expires_at: Option<DateTime<Utc>>,
}
```

### Modified Model: `User` (collection: `users`)

**File:** `backend/src/models/user.rs` -- add two new fields:

```rust
// Add after `is_admin`:
/// Directly-assigned role IDs (not including group-inherited roles)
#[serde(default)]
pub role_ids: Vec<String>,
/// Group membership IDs
#[serde(default)]
pub group_ids: Vec<String>,
```

The `#[serde(default)]` ensures backwards compatibility -- existing User documents without these fields will deserialize with empty Vecs.

### Modified Model: `OauthClient` (collection: `oauth_clients`)

**File:** `backend/src/models/oauth_client.rs` -- add optional scopes:

```rust
// No changes needed for v1. The `allowed_scopes` field already exists as a
// space-separated string. We will expand the default allowed scopes to include
// "roles groups" when creating/seeding clients.
```

---

## Enhanced JWT Claims

### Before (current)

```rust
pub struct Claims {
    pub sub: String,
    pub iss: String,
    pub aud: String,
    pub exp: i64,
    pub iat: i64,
    pub jti: String,
    pub scope: String,
    pub token_type: String,
}

pub struct IdTokenClaims {
    pub sub: String,
    pub iss: String,
    pub aud: String,
    pub exp: i64,
    pub iat: i64,
    pub email: Option<String>,
    pub email_verified: Option<bool>,
    pub name: Option<String>,
    pub picture: Option<String>,
    pub nonce: Option<String>,
    pub at_hash: Option<String>,
}
```

### After (enhanced)

```rust
pub struct Claims {
    pub sub: String,
    pub iss: String,
    pub aud: String,
    pub exp: i64,
    pub iat: i64,
    pub jti: String,
    pub scope: String,
    pub token_type: String,
    // --- NEW FIELDS ---
    /// User's roles (present when "roles" scope is requested)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roles: Option<Vec<String>>,
    /// User's groups (present when "groups" scope is requested)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub groups: Option<Vec<String>>,
    /// Flattened permissions from all roles (present when "roles" scope is requested)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permissions: Option<Vec<String>>,
    /// Session ID (stable across token refreshes)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sid: Option<String>,
}

pub struct IdTokenClaims {
    pub sub: String,
    pub iss: String,
    pub aud: String,
    pub exp: i64,
    pub iat: i64,
    pub email: Option<String>,
    pub email_verified: Option<bool>,
    pub name: Option<String>,
    pub picture: Option<String>,
    pub nonce: Option<String>,
    pub at_hash: Option<String>,
    // --- NEW FIELDS ---
    /// User's roles (present when "roles" scope is requested)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roles: Option<Vec<String>>,
    /// User's groups (present when "groups" scope is requested)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub groups: Option<Vec<String>>,
    /// Authentication Context Class Reference
    #[serde(skip_serializing_if = "Option::is_none")]
    pub acr: Option<String>,
    /// Authentication Methods References
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amr: Option<Vec<String>>,
    /// Time of authentication (Unix timestamp)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_time: Option<i64>,
    /// Session ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sid: Option<String>,
}
```

### New Scopes

| Scope | Effect |
|-------|--------|
| `roles` | Include `roles` and `permissions` claims in access token and ID token |
| `groups` | Include `groups` claim in access token and ID token |

These scopes must be added to:
1. The `allowed_scopes` for OAuth clients (default: `"openid profile email roles groups"`)
2. The `scopes_supported` in OIDC discovery responses
3. The `claims_supported` in OIDC discovery responses

---

## New Services

### `role_service` -- `backend/src/services/role_service.rs`

```rust
/// Create a new role.
pub async fn create_role(
    db: &mongodb::Database,
    name: &str,
    slug: &str,
    description: Option<&str>,
    permissions: &[String],
    is_default: bool,
    client_id: Option<&str>,
) -> AppResult<Role>

/// Get a role by ID.
pub async fn get_role(db: &mongodb::Database, role_id: &str) -> AppResult<Role>

/// List all roles (with optional client_id filter).
pub async fn list_roles(
    db: &mongodb::Database,
    client_id: Option<&str>,
) -> AppResult<Vec<Role>>

/// Update a role (non-system roles only; system roles allow description and permissions update).
pub async fn update_role(
    db: &mongodb::Database,
    role_id: &str,
    name: Option<&str>,
    slug: Option<&str>,
    description: Option<&str>,
    permissions: Option<&[String]>,
    is_default: Option<bool>,
) -> AppResult<Role>

/// Delete a role (non-system roles only). Removes from all users and groups.
pub async fn delete_role(db: &mongodb::Database, role_id: &str) -> AppResult<()>

/// Assign a role to a user (adds role_id to user.role_ids).
pub async fn assign_role_to_user(
    db: &mongodb::Database,
    user_id: &str,
    role_id: &str,
) -> AppResult<()>

/// Revoke a role from a user (removes role_id from user.role_ids).
pub async fn revoke_role_from_user(
    db: &mongodb::Database,
    user_id: &str,
    role_id: &str,
) -> AppResult<()>

/// Get all directly-assigned roles for a user.
pub async fn get_user_roles(
    db: &mongodb::Database,
    user_id: &str,
) -> AppResult<Vec<Role>>

/// Get effective roles (direct + group-inherited) for a user.
/// This is the function called at token generation time.
pub async fn get_user_effective_roles(
    db: &mongodb::Database,
    user_id: &str,
) -> AppResult<Vec<Role>>

/// Seed system roles ("admin", "user") if they don't exist.
pub async fn seed_system_roles(db: &mongodb::Database) -> AppResult<()>
```

### `group_service` -- `backend/src/services/group_service.rs`

```rust
/// Create a new group.
pub async fn create_group(
    db: &mongodb::Database,
    name: &str,
    slug: &str,
    description: Option<&str>,
    role_ids: &[String],
    parent_group_id: Option<&str>,
) -> AppResult<Group>

/// Get a group by ID.
pub async fn get_group(db: &mongodb::Database, group_id: &str) -> AppResult<Group>

/// List all groups.
pub async fn list_groups(db: &mongodb::Database) -> AppResult<Vec<Group>>

/// Update a group.
pub async fn update_group(
    db: &mongodb::Database,
    group_id: &str,
    name: Option<&str>,
    slug: Option<&str>,
    description: Option<&str>,
    role_ids: Option<&[String]>,
    parent_group_id: Option<Option<&str>>,
) -> AppResult<Group>

/// Delete a group. Removes group_id from all users.
pub async fn delete_group(db: &mongodb::Database, group_id: &str) -> AppResult<()>

/// Add a user to a group (adds group_id to user.group_ids).
pub async fn add_member(
    db: &mongodb::Database,
    group_id: &str,
    user_id: &str,
) -> AppResult<()>

/// Remove a user from a group (removes group_id from user.group_ids).
pub async fn remove_member(
    db: &mongodb::Database,
    group_id: &str,
    user_id: &str,
) -> AppResult<()>

/// Get all members of a group.
pub async fn get_members(
    db: &mongodb::Database,
    group_id: &str,
) -> AppResult<Vec<User>>

/// Get all groups a user belongs to.
pub async fn get_user_groups(
    db: &mongodb::Database,
    user_id: &str,
) -> AppResult<Vec<Group>>
```

### `consent_service` -- `backend/src/services/consent_service.rs`

```rust
/// Grant consent for a user to a client with specific scopes.
/// Upserts: if consent exists for (user_id, client_id), updates scopes.
pub async fn grant_consent(
    db: &mongodb::Database,
    user_id: &str,
    client_id: &str,
    scopes: &str,
) -> AppResult<Consent>

/// Check if a user has granted consent for the requested scopes to a client.
/// Returns Some(Consent) if all requested scopes are covered.
pub async fn check_consent(
    db: &mongodb::Database,
    user_id: &str,
    client_id: &str,
    requested_scopes: &str,
) -> AppResult<Option<Consent>>

/// Revoke consent for a specific client.
pub async fn revoke_consent(
    db: &mongodb::Database,
    user_id: &str,
    client_id: &str,
) -> AppResult<()>

/// List all consents for a user.
pub async fn list_user_consents(
    db: &mongodb::Database,
    user_id: &str,
) -> AppResult<Vec<Consent>>

/// List all consents for a client.
pub async fn list_client_consents(
    db: &mongodb::Database,
    client_id: &str,
) -> AppResult<Vec<Consent>>
```

### Modified: `token_service` -- `backend/src/services/token_service.rs`

Changes:
1. `create_session_and_issue_tokens` -- accepts optional roles/groups/permissions to embed in access token
2. New helper: `build_user_claims` -- fetches user's effective roles, groups, and permissions and returns them as claim data
3. `refresh_tokens` -- re-fetches user roles/groups at refresh time (roles may have changed)

### Modified: `oauth_service` -- `backend/src/services/oauth_service.rs`

Changes:
1. `validate_scopes` -- add "roles" and "groups" to recognized scopes
2. `exchange_authorization_code` -- fetch user roles/groups and embed in access token and ID token based on requested scopes

### New: `rbac_helpers` -- `backend/src/services/rbac_helpers.rs`

A shared utility for collecting the claims data:

```rust
/// Resolved RBAC data for a user, ready to inject into JWT claims.
pub struct UserRbacData {
    pub role_slugs: Vec<String>,
    pub group_slugs: Vec<String>,
    pub permissions: Vec<String>,
}

/// Fetch and resolve all RBAC data for a user.
pub async fn resolve_user_rbac(
    db: &mongodb::Database,
    user_id: &str,
) -> AppResult<UserRbacData>
```

---

## New Handlers / Endpoints

### Admin Role Endpoints -- `backend/src/handlers/admin_roles.rs`

| Method | Path | Handler | Description |
|--------|------|---------|-------------|
| GET | `/api/v1/admin/roles` | `list_roles` | List all roles |
| POST | `/api/v1/admin/roles` | `create_role` | Create a new role |
| GET | `/api/v1/admin/roles/{role_id}` | `get_role` | Get role details |
| PUT | `/api/v1/admin/roles/{role_id}` | `update_role` | Update a role |
| DELETE | `/api/v1/admin/roles/{role_id}` | `delete_role` | Delete a role |

#### Request/Response Shapes

```rust
// POST /api/v1/admin/roles
#[derive(Debug, Deserialize)]
pub struct CreateRoleRequest {
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub permissions: Vec<String>,
    pub is_default: Option<bool>,
    pub client_id: Option<String>,
}

// Response for all role endpoints
#[derive(Debug, Serialize)]
pub struct RoleResponse {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub permissions: Vec<String>,
    pub is_default: bool,
    pub is_system: bool,
    pub client_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct RoleListResponse {
    pub roles: Vec<RoleResponse>,
}

// PUT /api/v1/admin/roles/{role_id}
#[derive(Debug, Deserialize)]
pub struct UpdateRoleRequest {
    pub name: Option<String>,
    pub slug: Option<String>,
    pub description: Option<String>,
    pub permissions: Option<Vec<String>>,
    pub is_default: Option<bool>,
}
```

### Admin Role Assignment Endpoints -- in `backend/src/handlers/admin_roles.rs`

| Method | Path | Handler | Description |
|--------|------|---------|-------------|
| GET | `/api/v1/admin/users/{user_id}/roles` | `get_user_roles` | Get user's assigned roles |
| POST | `/api/v1/admin/users/{user_id}/roles/{role_id}` | `assign_role` | Assign role to user |
| DELETE | `/api/v1/admin/users/{user_id}/roles/{role_id}` | `revoke_role` | Revoke role from user |

```rust
// GET /api/v1/admin/users/{user_id}/roles
#[derive(Debug, Serialize)]
pub struct UserRolesResponse {
    pub direct_roles: Vec<RoleResponse>,
    pub inherited_roles: Vec<RoleResponse>,
    pub effective_permissions: Vec<String>,
}

// POST/DELETE responses
#[derive(Debug, Serialize)]
pub struct RoleAssignmentResponse {
    pub message: String,
}
```

### Admin Group Endpoints -- `backend/src/handlers/admin_groups.rs`

| Method | Path | Handler | Description |
|--------|------|---------|-------------|
| GET | `/api/v1/admin/groups` | `list_groups` | List all groups |
| POST | `/api/v1/admin/groups` | `create_group` | Create a new group |
| GET | `/api/v1/admin/groups/{group_id}` | `get_group` | Get group details |
| PUT | `/api/v1/admin/groups/{group_id}` | `update_group` | Update a group |
| DELETE | `/api/v1/admin/groups/{group_id}` | `delete_group` | Delete a group |
| GET | `/api/v1/admin/groups/{group_id}/members` | `get_members` | List group members |
| POST | `/api/v1/admin/groups/{group_id}/members/{user_id}` | `add_member` | Add user to group |
| DELETE | `/api/v1/admin/groups/{group_id}/members/{user_id}` | `remove_member` | Remove user from group |

```rust
// POST /api/v1/admin/groups
#[derive(Debug, Deserialize)]
pub struct CreateGroupRequest {
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub role_ids: Vec<String>,
    pub parent_group_id: Option<String>,
}

// Response for all group endpoints
#[derive(Debug, Serialize)]
pub struct GroupResponse {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub roles: Vec<RoleResponse>,
    pub parent_group_id: Option<String>,
    pub member_count: u64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct GroupListResponse {
    pub groups: Vec<GroupResponse>,
}

// PUT /api/v1/admin/groups/{group_id}
#[derive(Debug, Deserialize)]
pub struct UpdateGroupRequest {
    pub name: Option<String>,
    pub slug: Option<String>,
    pub description: Option<String>,
    pub role_ids: Option<Vec<String>>,
    pub parent_group_id: Option<String>,
}

// GET /api/v1/admin/groups/{group_id}/members
#[derive(Debug, Serialize)]
pub struct GroupMembersResponse {
    pub members: Vec<GroupMemberItem>,
    pub total: u64,
}

#[derive(Debug, Serialize)]
pub struct GroupMemberItem {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct GroupMembershipResponse {
    pub message: String,
}
```

### Admin User Groups Endpoint -- in `backend/src/handlers/admin_groups.rs`

| Method | Path | Handler | Description |
|--------|------|---------|-------------|
| GET | `/api/v1/admin/users/{user_id}/groups` | `get_user_groups` | Get user's group memberships |

```rust
#[derive(Debug, Serialize)]
pub struct UserGroupsResponse {
    pub groups: Vec<GroupResponse>,
}
```

### OAuth Introspection -- `backend/src/handlers/oauth.rs` (add to existing file)

| Method | Path | Handler | Description |
|--------|------|---------|-------------|
| POST | `/oauth/introspect` | `introspect` | RFC 7662 Token Introspection |

```rust
// POST /oauth/introspect (form-encoded per RFC 7662)
#[derive(Debug, Deserialize)]
pub struct IntrospectRequest {
    pub token: String,
    pub token_type_hint: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct IntrospectResponse {
    pub active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exp: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iat: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iss: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jti: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roles: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub groups: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permissions: Option<Vec<String>>,
}
```

### OAuth Revocation -- `backend/src/handlers/oauth.rs` (add to existing file)

| Method | Path | Handler | Description |
|--------|------|---------|-------------|
| POST | `/oauth/revoke` | `revoke` | RFC 7009 Token Revocation |

```rust
// POST /oauth/revoke (form-encoded per RFC 7009)
#[derive(Debug, Deserialize)]
pub struct RevokeRequest {
    pub token: String,
    pub token_type_hint: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
}
// Response: 200 OK with empty body (per RFC 7009, always returns 200)
```

### Consent Endpoints -- `backend/src/handlers/consent.rs`

| Method | Path | Handler | Description |
|--------|------|---------|-------------|
| GET | `/api/v1/users/me/consents` | `list_my_consents` | List user's granted consents |
| DELETE | `/api/v1/users/me/consents/{client_id}` | `revoke_my_consent` | Revoke consent for a client |

```rust
#[derive(Debug, Serialize)]
pub struct ConsentItem {
    pub id: String,
    pub client_id: String,
    pub client_name: String,
    pub scopes: String,
    pub granted_at: String,
    pub expires_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ConsentListResponse {
    pub consents: Vec<ConsentItem>,
}

#[derive(Debug, Serialize)]
pub struct ConsentRevokeResponse {
    pub message: String,
}
```

### Modified: UserInfo Endpoint -- `backend/src/handlers/oauth.rs`

The existing `userinfo` handler needs to be updated to support scope-filtered responses:

```rust
// Updated UserinfoResponse (add optional fields)
#[derive(Debug, Serialize)]
pub struct UserinfoResponse {
    pub sub: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email_verified: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub picture: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roles: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub groups: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permissions: Option<Vec<String>>,
}
```

---

## Error Codes (continuing from existing 3002)

| Code | Variant | Description |
|------|---------|-------------|
| 4000 | `RoleNotFound` | Role not found |
| 4001 | `GroupNotFound` | Group not found |
| 4002 | `ConsentNotFound` | Consent not found |
| 4003 | `RoleAlreadyAssigned` | Role is already assigned to the user |
| 4004 | `GroupMembershipExists` | User is already a member of the group |
| 4005 | `SystemRoleProtected` | Cannot delete or rename a system role |
| 4006 | `DuplicateSlug` | A role or group with this slug already exists |
| 4007 | `CircularGroupHierarchy` | Setting this parent would create a cycle |

**File:** `backend/src/errors/mod.rs` -- add new variants to `AppError`:

```rust
#[error("Role not found: {0}")]
RoleNotFound(String),

#[error("Group not found: {0}")]
GroupNotFound(String),

#[error("Consent not found")]
ConsentNotFound,

#[error("Role already assigned")]
RoleAlreadyAssigned,

#[error("User already a member of this group")]
GroupMembershipExists,

#[error("Cannot modify system role: {0}")]
SystemRoleProtected(String),

#[error("Duplicate slug: {0}")]
DuplicateSlug(String),

#[error("Circular group hierarchy detected")]
CircularGroupHierarchy,
```

---

## Database Indexes

**File:** `backend/src/db.rs` -- add to `ensure_indexes()`:

```rust
// -- roles --
let roles = db.collection::<Document>("roles");
roles.create_index(
    IndexModel::builder()
        .keys(doc! { "slug": 1 })
        .options(IndexOptions::builder().unique(true).build())
        .build(),
).await?;
roles.create_index(
    IndexModel::builder()
        .keys(doc! { "client_id": 1, "is_active": 1 })
        .build(),
).await?;

// -- groups --
let groups = db.collection::<Document>("groups");
groups.create_index(
    IndexModel::builder()
        .keys(doc! { "slug": 1 })
        .options(IndexOptions::builder().unique(true).build())
        .build(),
).await?;
groups.create_index(
    IndexModel::builder()
        .keys(doc! { "parent_group_id": 1 })
        .build(),
).await?;

// -- consents --
let consents = db.collection::<Document>("consents");
consents.create_index(
    IndexModel::builder()
        .keys(doc! { "user_id": 1, "client_id": 1 })
        .options(IndexOptions::builder().unique(true).build())
        .build(),
).await?;
consents.create_index(
    IndexModel::builder()
        .keys(doc! { "user_id": 1 })
        .build(),
).await?;
```

---

## Route Changes

**File:** `backend/src/routes.rs`

Add to `admin_routes`:
```rust
// Roles
.route("/roles", get(handlers::admin_roles::list_roles)
    .post(handlers::admin_roles::create_role))
.route("/roles/{role_id}", get(handlers::admin_roles::get_role)
    .put(handlers::admin_roles::update_role)
    .delete(handlers::admin_roles::delete_role))
// Role assignment
.route("/users/{user_id}/roles", get(handlers::admin_roles::get_user_roles))
.route("/users/{user_id}/roles/{role_id}",
    post(handlers::admin_roles::assign_role)
    .delete(handlers::admin_roles::revoke_role))
// Groups
.route("/groups", get(handlers::admin_groups::list_groups)
    .post(handlers::admin_groups::create_group))
.route("/groups/{group_id}", get(handlers::admin_groups::get_group)
    .put(handlers::admin_groups::update_group)
    .delete(handlers::admin_groups::delete_group))
.route("/groups/{group_id}/members", get(handlers::admin_groups::get_members))
.route("/groups/{group_id}/members/{user_id}",
    post(handlers::admin_groups::add_member)
    .delete(handlers::admin_groups::remove_member))
// User groups
.route("/users/{user_id}/groups", get(handlers::admin_groups::get_user_groups))
```

Add to `oauth_routes`:
```rust
.route("/introspect", post(handlers::oauth::introspect))
.route("/revoke", post(handlers::oauth::revoke))
```

Add to `user_routes`:
```rust
.route("/me/consents", get(handlers::consent::list_my_consents))
.route("/me/consents/{client_id}", delete(handlers::consent::revoke_my_consent))
```

---

## OIDC Discovery Updates

**File:** `backend/src/handlers/oidc_discovery.rs`

Update `scopes_supported`:
```json
["openid", "profile", "email", "roles", "groups"]
```

Update `claims_supported`:
```json
["sub", "iss", "aud", "exp", "iat", "email", "email_verified", "name",
 "picture", "nonce", "at_hash", "roles", "groups", "permissions",
 "acr", "amr", "auth_time", "sid"]
```

Add to `oauth_authorization_server_metadata`:
```json
"introspection_endpoint": "{base}/oauth/introspect",
"revocation_endpoint": "{base}/oauth/revoke",
```

---

## Startup Seeding

**File:** `backend/src/main.rs`

Add after `seed_default_providers`:
```rust
services::role_service::seed_system_roles(&db)
    .await
    .expect("Failed to seed system roles");
```

The seed function creates:
1. Role `admin` (slug: "admin", permissions: ["*"], is_system: true, is_default: false)
2. Role `user` (slug: "user", permissions: [], is_system: true, is_default: true)

---

## Frontend Changes

### New Types -- `frontend/src/types/rbac.ts`

```typescript
export interface Role {
  readonly id: string;
  readonly name: string;
  readonly slug: string;
  readonly description: string | null;
  readonly permissions: readonly string[];
  readonly is_default: boolean;
  readonly is_system: boolean;
  readonly client_id: string | null;
  readonly created_at: string;
  readonly updated_at: string;
}

export interface RoleListResponse {
  readonly roles: readonly Role[];
}

export interface Group {
  readonly id: string;
  readonly name: string;
  readonly slug: string;
  readonly description: string | null;
  readonly roles: readonly Role[];
  readonly parent_group_id: string | null;
  readonly member_count: number;
  readonly created_at: string;
  readonly updated_at: string;
}

export interface GroupListResponse {
  readonly groups: readonly Group[];
}

export interface GroupMember {
  readonly id: string;
  readonly email: string;
  readonly display_name: string | null;
}

export interface GroupMembersResponse {
  readonly members: readonly GroupMember[];
  readonly total: number;
}

export interface UserRolesResponse {
  readonly direct_roles: readonly Role[];
  readonly inherited_roles: readonly Role[];
  readonly effective_permissions: readonly string[];
}

export interface UserGroupsResponse {
  readonly groups: readonly Group[];
}

export interface Consent {
  readonly id: string;
  readonly client_id: string;
  readonly client_name: string;
  readonly scopes: string;
  readonly granted_at: string;
  readonly expires_at: string | null;
}

export interface ConsentListResponse {
  readonly consents: readonly Consent[];
}
```

### New Hooks -- `frontend/src/hooks/use-rbac.ts`

```typescript
// Admin Role hooks
useRoles() -> useQuery for GET /admin/roles
useRole(roleId) -> useQuery for GET /admin/roles/{roleId}
useCreateRole() -> useMutation for POST /admin/roles
useUpdateRole() -> useMutation for PUT /admin/roles/{roleId}
useDeleteRole() -> useMutation for DELETE /admin/roles/{roleId}

// Admin Role Assignment hooks
useUserRoles(userId) -> useQuery for GET /admin/users/{userId}/roles
useAssignRole() -> useMutation for POST /admin/users/{userId}/roles/{roleId}
useRevokeRole() -> useMutation for DELETE /admin/users/{userId}/roles/{roleId}

// Admin Group hooks
useGroups() -> useQuery for GET /admin/groups
useGroup(groupId) -> useQuery for GET /admin/groups/{groupId}
useCreateGroup() -> useMutation for POST /admin/groups
useUpdateGroup() -> useMutation for PUT /admin/groups/{groupId}
useDeleteGroup() -> useMutation for DELETE /admin/groups/{groupId}
useGroupMembers(groupId) -> useQuery for GET /admin/groups/{groupId}/members
useAddGroupMember() -> useMutation for POST /admin/groups/{groupId}/members/{userId}
useRemoveGroupMember() -> useMutation for DELETE /admin/groups/{groupId}/members/{userId}
useUserGroups(userId) -> useQuery for GET /admin/users/{userId}/groups
```

### New Hooks -- `frontend/src/hooks/use-consents.ts`

```typescript
useMyConsents() -> useQuery for GET /users/me/consents
useRevokeConsent() -> useMutation for DELETE /users/me/consents/{clientId}
```

### New Schemas -- `frontend/src/schemas/rbac.ts`

```typescript
export const createRoleSchema = z.object({
  name: z.string().min(1).max(100),
  slug: z.string().min(1).max(100).regex(/^[a-z0-9_-]+$/),
  description: z.string().max(500).optional().or(z.literal("")),
  permissions: z.array(z.string()).default([]),
  is_default: z.boolean().default(false),
  client_id: z.string().optional().or(z.literal("")),
});

export const updateRoleSchema = createRoleSchema.partial();

export const createGroupSchema = z.object({
  name: z.string().min(1).max(100),
  slug: z.string().min(1).max(100).regex(/^[a-z0-9_-]+$/),
  description: z.string().max(500).optional().or(z.literal("")),
  role_ids: z.array(z.string()).default([]),
  parent_group_id: z.string().optional().or(z.literal("")),
});

export const updateGroupSchema = createGroupSchema.partial();
```

### New Pages

| File | Route | Description |
|------|-------|-------------|
| `frontend/src/pages/admin-roles.tsx` | `/admin/roles` | List, create, edit, delete roles |
| `frontend/src/pages/admin-role-detail.tsx` | `/admin/roles/$roleId` | Role detail with permission editor |
| `frontend/src/pages/admin-groups.tsx` | `/admin/groups` | List, create, edit, delete groups |
| `frontend/src/pages/admin-group-detail.tsx` | `/admin/groups/$groupId` | Group detail with member management |
| `frontend/src/pages/consents.tsx` | `/settings/consents` | User consent management (view, revoke) |

### Modified Pages

| File | Change |
|------|--------|
| `frontend/src/pages/admin-user-detail.tsx` | Add "Roles" and "Groups" tabs showing user's assigned roles/groups with assign/revoke buttons |
| `frontend/src/pages/settings.tsx` | Add link/section for consent management |

### Router Changes -- `frontend/src/router.tsx`

```typescript
// New imports
import { AdminRolesPage } from "@/pages/admin-roles";
import { AdminRoleDetailPage } from "@/pages/admin-role-detail";
import { AdminGroupsPage } from "@/pages/admin-groups";
import { AdminGroupDetailPage } from "@/pages/admin-group-detail";
import { ConsentsPage } from "@/pages/consents";

// New routes under adminLayout
const adminRolesRoute = createRoute({
  path: "roles",
  getParentRoute: () => adminLayout,
  component: AdminRolesPage,
});

const adminRoleDetailRoute = createRoute({
  path: "roles/$roleId",
  getParentRoute: () => adminLayout,
  component: AdminRoleDetailPage,
});

const adminGroupsRoute = createRoute({
  path: "groups",
  getParentRoute: () => adminLayout,
  component: AdminGroupsPage,
});

const adminGroupDetailRoute = createRoute({
  path: "groups/$groupId",
  getParentRoute: () => adminLayout,
  component: AdminGroupDetailPage,
});

// New route under dashboardLayout
const consentsRoute = createRoute({
  path: "/settings/consents",
  getParentRoute: () => dashboardLayout,
  component: ConsentsPage,
});

// Update routeTree:
adminLayout.addChildren([
  adminUsersRoute,
  adminUserDetailRoute,
  adminRolesRoute,        // NEW
  adminRoleDetailRoute,   // NEW
  adminGroupsRoute,       // NEW
  adminGroupDetailRoute,  // NEW
])
```

---

## Migration Strategy (Backwards Compatible)

### Phase 1: Non-Breaking Schema Addition

1. Add `role_ids: Vec<String>` and `group_ids: Vec<String>` to User model with `#[serde(default)]` -- existing documents deserialize cleanly with empty Vecs
2. Create `roles`, `groups`, `consents` collections and indexes -- empty collections have zero impact
3. Seed system roles ("admin", "user")
4. Update `oauth_client` default scopes to include "roles groups"

### Phase 2: Token Enhancement (Non-Breaking)

1. Add new optional fields to `Claims` and `IdTokenClaims` with `#[serde(skip_serializing_if = "Option::is_none")]` -- existing tokens remain valid, new tokens are a superset
2. Existing JWT verification (`verify_token`) works unchanged because new fields are optional
3. `generate_access_token` signature expands but callers pass `None` until RBAC data is wired in

### Phase 3: Admin Endpoints (Additive)

1. New routes are purely additive -- no existing endpoint changes
2. The `require_admin` check remains `is_admin`-based but is updated to also check for "admin" role assignment

### Phase 4: Consent & Introspect/Revoke (Additive)

1. New endpoints only
2. OAuth authorize flow updated to check consent for third-party clients (first-party auto-grants preserved)

### Data Migration Script (Optional CLI)

For existing deployments, a CLI command `--migrate-rbac` could:
1. Find all users with `is_admin = true` and add the "admin" system role ID to their `role_ids`
2. Add the "user" default role ID to all users' `role_ids`
3. This is optional -- the `get_user_effective_roles` function handles the `is_admin` fallback

---

## File-by-File Change List

### Backend -- New Files

| File | Type | Description |
|------|------|-------------|
| `backend/src/models/role.rs` | Model | Role struct + COLLECTION_NAME + tests |
| `backend/src/models/group.rs` | Model | Group struct + COLLECTION_NAME + tests |
| `backend/src/models/consent.rs` | Model | Consent struct + COLLECTION_NAME + tests |
| `backend/src/services/role_service.rs` | Service | Role CRUD + assignment + seeding |
| `backend/src/services/group_service.rs` | Service | Group CRUD + membership |
| `backend/src/services/consent_service.rs` | Service | Consent CRUD |
| `backend/src/services/rbac_helpers.rs` | Service | Shared RBAC resolution utility |
| `backend/src/handlers/admin_roles.rs` | Handler | Admin role CRUD + user role assignment endpoints |
| `backend/src/handlers/admin_groups.rs` | Handler | Admin group CRUD + membership endpoints |
| `backend/src/handlers/consent.rs` | Handler | User consent list/revoke endpoints |

### Backend -- Modified Files

| File | Changes |
|------|---------|
| `backend/src/models/mod.rs` | Add `pub mod role;`, `pub mod group;`, `pub mod consent;` |
| `backend/src/models/user.rs` | Add `role_ids: Vec<String>` and `group_ids: Vec<String>` fields with `#[serde(default)]` |
| `backend/src/services/mod.rs` | Add `pub mod role_service;`, `pub mod group_service;`, `pub mod consent_service;`, `pub mod rbac_helpers;` |
| `backend/src/handlers/mod.rs` | Add `pub mod admin_roles;`, `pub mod admin_groups;`, `pub mod consent;` |
| `backend/src/crypto/jwt.rs` | Add optional `roles`, `groups`, `permissions`, `sid` to `Claims`; add `roles`, `groups`, `acr`, `amr`, `auth_time`, `sid` to `IdTokenClaims`; update `generate_access_token` and `generate_id_token` signatures |
| `backend/src/services/token_service.rs` | Add RBAC data fetching in `create_session_and_issue_tokens` and `refresh_tokens`; pass roles/groups to JWT generation |
| `backend/src/services/oauth_service.rs` | Add "roles"/"groups" to recognized scopes; update `exchange_authorization_code` to fetch and embed RBAC claims |
| `backend/src/handlers/oauth.rs` | Add `introspect` and `revoke` handlers; update `UserinfoResponse` with optional roles/groups/permissions; update `userinfo` to scope-filter claims |
| `backend/src/handlers/oidc_discovery.rs` | Update `scopes_supported`, `claims_supported`, add `introspection_endpoint` and `revocation_endpoint` |
| `backend/src/routes.rs` | Add admin role/group routes, oauth introspect/revoke routes, user consent routes |
| `backend/src/db.rs` | Add indexes for `roles`, `groups`, `consents` collections |
| `backend/src/errors/mod.rs` | Add new error variants (4000-4007) |
| `backend/src/main.rs` | Add `role_service::seed_system_roles()` call at startup |

### Frontend -- New Files

| File | Type | Description |
|------|------|-------------|
| `frontend/src/types/rbac.ts` | Types | Role, Group, Consent TypeScript types |
| `frontend/src/hooks/use-rbac.ts` | Hook | TanStack Query hooks for roles, groups, assignments |
| `frontend/src/hooks/use-consents.ts` | Hook | TanStack Query hooks for user consents |
| `frontend/src/schemas/rbac.ts` | Schema | Zod schemas for role/group forms |
| `frontend/src/pages/admin-roles.tsx` | Page | Admin roles management (list, create, edit, delete) |
| `frontend/src/pages/admin-role-detail.tsx` | Page | Role detail with permission editor |
| `frontend/src/pages/admin-groups.tsx` | Page | Admin groups management (list, create, edit, delete) |
| `frontend/src/pages/admin-group-detail.tsx` | Page | Group detail with member management |
| `frontend/src/pages/consents.tsx` | Page | User consent management |

### Frontend -- Modified Files

| File | Changes |
|------|---------|
| `frontend/src/router.tsx` | Add routes for admin roles, admin groups, admin role detail, admin group detail, consents |
| `frontend/src/pages/admin-user-detail.tsx` | Add Roles tab and Groups tab with assign/revoke UI |
| `frontend/src/pages/settings.tsx` | Add link to consent management |
| `frontend/src/types/admin.ts` | Add role_ids and group_ids to AdminUser interface |

---

## Testing Strategy

### Backend Unit Tests

- `backend/src/models/role.rs` -- BSON roundtrip, collection name
- `backend/src/models/group.rs` -- BSON roundtrip, collection name
- `backend/src/models/consent.rs` -- BSON roundtrip, collection name
- `backend/src/crypto/jwt.rs` -- New claims serialize/deserialize, optional fields skip when None
- `backend/src/errors/mod.rs` -- New error variants have unique codes, correct HTTP status

### Backend Integration Tests

- Role CRUD service tests (requires MongoDB)
- Group CRUD + membership service tests
- Consent grant/check/revoke service tests
- `get_user_effective_roles` with direct roles + group-inherited roles
- Token generation with RBAC claims
- Introspection with valid/invalid/revoked tokens
- Revocation of access and refresh tokens

### Frontend Tests

- `frontend/src/schemas/rbac.test.ts` -- Zod schema validation for role/group forms
- Hook tests via MSW mocking (existing pattern)

---

## Risks and Mitigations

### Risk: Performance impact of role resolution at token time

**Mitigation:** The `resolve_user_rbac` function performs at most 3 MongoDB queries (user, roles by IDs, groups by IDs). With proper indexes and typical role/group counts (< 50), this adds < 5ms to token generation. For high-traffic deployments, add in-memory caching (LRU with 60s TTL) in a future phase.

### Risk: Group hierarchy cycles

**Mitigation:** The `update_group` function validates that setting a `parent_group_id` does not create a cycle by walking up the chain (max depth 10). Returns `CircularGroupHierarchy` error if cycle detected.

### Risk: Breaking existing JWT consumers

**Mitigation:** All new claim fields use `#[serde(skip_serializing_if = "Option::is_none")]`. Existing tokens continue to validate. New fields only appear when requested via scopes.

### Risk: `is_admin` flag and role-based admin check diverge

**Mitigation:** `require_admin()` checks both `is_admin` flag AND "admin" role assignment. Migration script syncs `is_admin` users to have the "admin" role. Long-term, `is_admin` is deprecated in favor of role-based checks.

---

## Success Criteria

- [ ] System roles ("admin", "user") seeded at startup
- [ ] Admin can CRUD roles and groups via API
- [ ] Admin can assign/revoke roles to users
- [ ] Admin can manage group membership
- [ ] Access tokens include `roles`, `groups`, `permissions` claims when `roles`/`groups` scopes requested
- [ ] ID tokens include RBAC claims and standard claims (acr, amr, auth_time, sid)
- [ ] Token introspection endpoint returns active/inactive with claims
- [ ] Token revocation endpoint works for access and refresh tokens
- [ ] UserInfo endpoint returns scope-filtered RBAC claims
- [ ] OIDC discovery metadata includes new scopes, claims, and endpoints
- [ ] Users can view and revoke their OAuth consents
- [ ] Frontend admin pages for roles, groups, and consent management
- [ ] Existing tokens and auth flows continue working without changes
- [ ] All new code has unit tests; integration tests for service layer
- [ ] No security regressions (RBAC checks on all admin endpoints)

---

## Implementation Phases

### Phase 1: Models, Services, Indexes (Backend Foundation)
1. New model files (role, group, consent)
2. User model changes (role_ids, group_ids)
3. Error variants
4. Database indexes
5. Service files (role_service, group_service, consent_service, rbac_helpers)
6. Startup seeding
7. Unit tests for models and errors

### Phase 2: JWT Enhancement (Backend Token Layer)
1. Update Claims and IdTokenClaims structs
2. Update generate_access_token and generate_id_token signatures
3. Wire RBAC data into token_service
4. Wire RBAC data into oauth_service (authorization code exchange)
5. Update OIDC discovery metadata
6. JWT unit tests

### Phase 3: Admin Endpoints (Backend API Layer)
1. Admin role handlers
2. Admin group handlers
3. Role assignment handlers
4. Route registration
5. Handler tests

### Phase 4: OAuth Enhancements (Backend)
1. Token introspection handler
2. Token revocation handler
3. Consent handler
4. UserInfo scope filtering
5. OAuth authorize consent flow (for third-party clients)

### Phase 5: Frontend
1. Types and schemas
2. Hooks (use-rbac, use-consents)
3. Admin roles page
4. Admin groups page
5. Admin user detail RBAC tabs
6. Consent management page
7. Router updates
