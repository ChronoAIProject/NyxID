import { billingAgentUsage, billingAllowance, billingCatalog, billingGrant, billingRow, billingUsage, billingWallet } from "./billing-fixture";
import { currentBill, usageInvoices } from "./billing-bill-fixture";
import { remoteMachineFixture } from "./remote-machine-fixture";

export const reviewOwner = {
  id: "owner", display_name: "Reader", email: "reader@example.com", email_verified: true,
  has_password: true, mfa_enabled: false, is_admin: false, is_active: true,
  avatar_url: null, created_at: "2026-09-01T00:00:00Z",
  capabilities: { billing_available: true, enabled_features: ["assistant:model-controls"] },
};
export const reviewAgent = {
  id: "agent-researcher", kind: "specialist", name: "researcher", description: "Finds urgent issues", status: "idle",
  created_by: "nyxbot", services: ["github"], account_read: false, guest_access: { github: "read" },
  pending_requests: [], last_reply: null, home_conversation_id: null, memory_count: 0,
  created_at: "2026-09-01T00:00:00Z", last_active_at: "2026-09-30T00:00:00Z", destroyed_at: null, pending_acknowledgements: 0, channels: [],
};
export const reviewBenefits = {
  summary: { remaining: "27", unreserved: "12.000000000001", held: "3", pending_activation: "5", pending_reclaim: "2", expired_awaiting_cleanup: "4", restricted: "6", generated_at: "2026-10-01T00:00:00Z" },
  grants: [
    billingGrant({ id: "active", reason: "Active grant", expires_at: null, available: "12.000000000001", held: "3", remaining: "15" }),
    billingGrant({ id: "pending", reason: "Pending grant", activation_state: "pending_activation", expires_at: null, remaining: "5" }),
    billingGrant({ id: "reclaim", reason: "Reclaim grant", revoke_requested_at: "2026-09-29T00:00:00Z", expires_at: null, remaining: "2" }),
    billingGrant({ id: "expired", reason: "Expired grant", status: "expired", remaining: "4" }),
    billingGrant({ id: "revoked", reason: "Revoked grant", status: "revoked", remaining: "1" }),
    billingGrant({ id: "exhausted", reason: "Exhausted grant", status: "consumed", remaining: "0" }),
  ],
  allowances: [billingAllowance("requests"), billingAllowance("input_tokens")],
  page: 1, per_page: 50, total_grants: 6,
};
export const reviewRuntime = {
  node_id: "machine-1", run_id: "run-1", state: "stop_pending", rate_version: "rate-v1",
  accepted_credits_per_second: "0.000040", covered_until: "2026-10-14T08:15:00Z", cumulative_seconds: 4,
  settled_credits: "0.000000000001", reserved_credits: "0.02", pending_credits: "0.01",
  funds: { wallet_available: "3", wallet_reserved: "0.02", pending_debits: "0", pending_expiry: "0", matching_allowance_seconds: 0, available_grants: "0", estimated_seconds: "75000", shared_account_estimate_only: true },
  intervals: [], next_interval_offset: null, attribution: "machine_runtime", service_costs_included: false,
};
export const reviewUsage = billingUsage([billingRow(), billingRow({ estimated_credits_micros: null }), billingRow({ service_slug: "free-service", billable: false, estimated_credits_micros: 0 })]);

/** Real hook/API boundary fixtures, shared by base-capture and current regression tests. */
export function settingsReviewResponse(path: string): unknown {
  const url = new URL(path, "https://nyxid.test");
  const key = url.pathname.replace(/^\/api\/v1/, "");
  const fixtures: Record<string, unknown> = {
    "/users/me": reviewOwner, "/sessions": [], "/orgs": { orgs: [] },
    "/catalog": { entries: billingCatalog }, "/keys": { keys: [] }, "/nodes": { nodes: [] }, "/saved-logins": [],
    "/onboarding/bindings": { bindings: [] }, "/analytics/preferences": { enabled: false },
    "/billing/wallet": billingWallet({ owner_id: "owner", available: "95.123456789123", balance: "100.123456789123", pending_expiry: "1", overdraft_cap: "2" }),
    "/billing/benefits": reviewBenefits,
    "/billing/usage": { ...reviewUsage, owner_id: "owner" },
    "/billing/bill": currentBill, "/billing/invoices": usageInvoices,
    "/billing/topups": { owner_id: "owner", topups: [{ id: "purchase", created_at: "2026-09-25T00:00:00Z", amount_credits: 100, status: "paid", receipt_available: true, lago_invoice_id: "invoice", invoice_number: "INV-1", credits_expire_at: "2027-09-25T00:00:00Z" }], page: 1, per_page: 10, total: 1 },
    "/api-keys/usage": { usage: [billingAgentUsage()], days: 7, since: "2026-10-01T00:00:00Z" },
    "/remote-machines": { machines: [remoteMachineFixture], next_cursor: null },
    "/remote-machines/machine-1/billing": reviewRuntime,
    "/remote-machines/machine-1/billing/runs": { runs: [], next_cursor: null },
    "/assistant/nyxagent/agents/agent-researcher": { agent: reviewAgent, memory: [], threads: [] },
    "/assistant/nyxagent/settings": { access_mode: "nyxmode", skip_destructive_confirmation: false, max_live_subagents_limit: 32, max_concurrent_subagent_turns_limit: 8, max_live_subagents: 8, max_concurrent_subagent_turns: 3, specialist_access_requests: "denied" },
    "/assistant/nyxagent/channels": { channel_agents: [] },
    "/assistant/nyxagent/grant-services": { services: [] },
    "/assistant/nyxagent/agents/agent-researcher/machines": { machines: [] },
    "/assistant/nyxagent/agents/agent-researcher/skills/history": { items: [] },
    "/channel-bots": { bots: [] },
    "/assistant/nyxagent/models": [], "/triggers": { triggers: [] },
    "/assistant/nyxagent/agents/agent-researcher/skills": { revision: 0, skills: [], metadata: {} },
    "/assistant/nyxagent/agents/agent-researcher/connections": { connections: [], services: [] },
    "/assistant/nyxagent/agents/agent-researcher/guest-readiness": { status: "not_ready", fix: "Send one message to this assistant yourself." },
    "/assistant/nyxagent/agents/agent-researcher/operations": [],
    "/assistant/nyxagent/agents/agent-researcher/guest-operations": [],
    "/assistant/nyxagent/agents/agent-researcher/voice-service": { service: null },
    "/assistant/nyxagent/agents/agent-researcher/learning": { enabled: true, threshold: 15 },
    "/assistant/nyxagent/agents/agent-researcher/learning/proposals": { proposals: [] },
  };
  return fixtures[key] ?? {};
}
