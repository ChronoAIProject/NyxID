# Database Migrations

This directory contains MongoDB migration scripts for NyxID schema changes that require data backfilling.

**Note:** NyxID creates collections and indexes automatically on startup via `db::ensure_indexes()`. These migration scripts are only needed when existing documents need field updates that cannot be handled by `serde(default)` alone.

---

## Migration 001: Service Categories

**File:** `001-service-categories.js`

**When to run:** Before deploying the service proxy overhaul (the update that adds `service_category` and `requires_user_credential` to `DownstreamService`, and `credential_type` / `credential_label` to `UserServiceConnection`).

**What it does:**

1. Sets `service_category = "provider"` and `requires_user_credential = false` on all OIDC services (where `auth_method` or `auth_type` is `"oidc"`).
2. Sets `service_category = "connection"` and `requires_user_credential = true` on all remaining services that lack a `service_category` field.
3. Adds `credential_type = null` and `credential_label = null` to all existing `user_service_connections` documents that lack these fields.
4. Creates a compound index on `{ service_category: 1, is_active: 1 }` for efficient category-filtered queries.

**How to run:**

```bash
# Against a local development database
mongosh "mongodb://localhost:27017/nyxid" 001-service-categories.js

# Against a remote database (with authentication)
mongosh "mongodb://user:password@host:27017/nyxid?authSource=admin" 001-service-categories.js

# Against MongoDB Atlas
mongosh "mongodb+srv://user:password@cluster.mongodb.net/nyxid" 001-service-categories.js
```

**Is this required?**

Strictly speaking, no. The backend uses `#[serde(default)]` on the new fields, so existing documents will deserialize correctly without migration:
- `service_category` defaults to `"connection"`
- `requires_user_credential` defaults to `true`

However, without the migration:
- OIDC services will incorrectly default to `"connection"` instead of `"provider"`, causing them to appear in the user connections page
- The `{ service_category, is_active }` index will not exist until the next application restart (when `ensure_indexes()` runs)

Running the migration is recommended for correct categorization of existing services.

**Idempotency:** Safe to run multiple times. The `$exists: false` filters ensure documents are only updated if they lack the target fields. The `createIndex` call is a no-op if the index already exists.

**Rollback:** To undo the migration, remove the added fields:

```javascript
db.downstream_services.updateMany(
  {},
  { $unset: { service_category: "", requires_user_credential: "" } }
);

db.user_service_connections.updateMany(
  {},
  { $unset: { credential_type: "", credential_label: "" } }
);

db.downstream_services.dropIndex("service_category_1_is_active_1");
```

---

## Adding New Migrations

When adding a new migration:

1. Name the file with a sequential number: `002-<description>.js`
2. Add a header comment with usage instructions
3. Make the migration idempotent (safe to run multiple times)
4. Document the migration in this README
5. Test against a copy of production data before running in production
