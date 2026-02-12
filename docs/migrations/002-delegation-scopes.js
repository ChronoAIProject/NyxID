// Migration: Add delegation_scopes to oauth_clients
// Non-breaking: existing clients get empty delegation_scopes (token exchange disabled)
//
// Idempotent: safe to run multiple times. The { $exists: false } filter ensures
// only documents missing the field are updated; re-running is a no-op.
db.oauth_clients.updateMany(
  { delegation_scopes: { $exists: false } },
  { $set: { delegation_scopes: "" } }
);
