export const ORG_ADMIN = "10000000-0000-4000-8000-000000000001";
export const ORG_MEMBER = "10000000-0000-4000-8000-000000000002";
export const ORG_VIEWER = "10000000-0000-4000-8000-000000000003";
export const walletFixture = {
  balance: "-0.000000000001", reserved: "0.1", pending_debits: "0.000000000002",
  pending_expiry: "1.234567890123", available: "-1.334567890126", overdraft_cap: "100", unit: "credits",
};
export const usageFixture = {
  owner_id: "person", period: "30d",
  rows: [
    { service_slug: "example", service_id: "service", metric: "requests", layer: "service", quantity: 10,
      estimated_credits: "9007199254740993.000000000001", wallet_credits: "0.000000000001", grant_credits: "9007199254740993", billable: true },
    { service_slug: "example", service_id: "service", metric: "input_tokens", layer: "service", quantity: 500,
      estimated_credits: "0.000000000002", wallet_credits: "0.000000000002", grant_credits: "0", billable: true },
  ],
  totals: { estimated_credits: "9007199254740993.000000000003", wallet_credits: "0.000000000003", grant_credits: "9007199254740993" },
  billing: { charging_enabled: true, rates_are_approximate: true },
};
export const grantFixture = {
  id: "grant", amount: "100.000000000001", remaining: "0.000000000001", reserved: "0.1",
  activation_state: "active", status: "active", expires_at: "2026-11-01T00:00:00Z", reason: "Development credits",
  scope: { all_services: false, service_ids: ["service"], service_slugs: ["example"] },
  service_limits: [{ key: "service", label: "Example service", service_ids: ["service"], service_slugs: ["example"],
    limit: "50.000000000001", consumed: "49", reserved: "0.000000000001", held: "0.000000000001" }],
};
export const allowanceFixture = {
  allowance: { id: "allowance", service_slug: "example", metric: "requests", quantity: 1000, recurrence: "monthly", is_active: true },
  period_start: "2026-10-01T00:00:00Z", period_end: "2026-11-01T00:00:00Z",
  consumed_quantity: 300, reserved_quantity: 10, remaining_quantity: 690,
};
export const organizationsFixture = { orgs: [
  { id: ORG_ADMIN, slug: "admin-org", display_name: "Admin organization", your_role: "admin" },
  { id: ORG_MEMBER, slug: "member-org", display_name: null, your_role: "member" },
  { id: ORG_VIEWER, slug: "viewer-org", your_role: "viewer" },
] };
