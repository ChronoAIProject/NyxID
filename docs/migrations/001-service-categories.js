// Migration 001: Add service_category and requires_user_credential fields
// Run against the MongoDB database before deploying the new backend.
//
// Usage:
//   mongosh "mongodb://localhost:27017/nyxid" 001-service-categories.js

// 1. OIDC services -> provider category
db.downstream_services.updateMany(
  { $or: [{ auth_method: "oidc" }, { auth_type: "oidc" }] },
  {
    $set: {
      service_category: "provider",
      requires_user_credential: false,
    },
  },
);

// 2. All other services without a category -> connection (default)
db.downstream_services.updateMany(
  { service_category: { $exists: false } },
  {
    $set: {
      service_category: "connection",
      requires_user_credential: true,
    },
  },
);

// 3. Add credential_type and credential_label to existing connections
db.user_service_connections.updateMany(
  { credential_type: { $exists: false } },
  {
    $set: {
      credential_type: null,
      credential_label: null,
    },
  },
);

// 4. Create new index for category-filtered service queries
db.downstream_services.createIndex(
  { service_category: 1, is_active: 1 },
);

print("Migration 001 complete: service categories applied.");
