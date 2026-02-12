// Migration: Add delegation token injection fields to downstream_services
// Non-breaking: existing services get inject_delegation_token=false (no tokens injected)
//
// Idempotent: safe to run multiple times. The { $exists: false } filter ensures
// only documents missing the field are updated; re-running is a no-op.
db.downstream_services.updateMany(
  { inject_delegation_token: { $exists: false } },
  { $set: { inject_delegation_token: false, delegation_token_scope: "llm:proxy" } }
);
