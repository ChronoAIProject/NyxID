import assert from "node:assert/strict";
import test from "node:test";
import { balanceRows, spendableGrant } from "./balanceSummary";
import { allowancesSchema, grantsSchema, walletSchema } from "./billingContracts";
import { allowanceFixture, grantFixture, walletFixture } from "./billingFixtures";
import { creditUnits } from "./credits";

const now = Date.parse("2026-10-28T00:00:00Z");
const wallet = walletSchema.parse({ ...walletFixture, overdraft_cap: "0", reserved: "0", pending_debits: "0", pending_expiry: "0" });
const grant = (overrides: Record<string, unknown> = {}) => grantsSchema.parse({ grants: [{ ...grantFixture, remaining: "10.123456789012", held: "0.000000000001", ...overrides }] }).grants[0]!;
const allowance = (overrides: Record<string, unknown> = {}) => allowancesSchema.parse({ allowances: [{ ...allowanceFixture, ...overrides }] }).allowances[0]!;

test("zero and exhausted balances add no extra rows", () => {
  assert.deepEqual(balanceRows(wallet, [], [], now), []);
  assert.deepEqual(balanceRows(wallet, [grant({ remaining: "0" })], [allowance({ remaining_quantity: 0 })], now), []);
});

test("held credits and disjoint service ceilings bound a grant exactly, without floats", () => {
  const capped = grant();
  // Scoped ceiling: 50.000000000001 - 49 - 0.000000000001 = 1.
  assert.equal(spendableGrant(capped, now), creditUnits("1"));
  assert.equal(spendableGrant(grant({ service_limits: [] }), now), creditUnits("10.123456789011"));
  assert.equal(spendableGrant(grant({ service_limits: [], held: "11" }), now), 0n);
  assert.equal(spendableGrant(grant({ service_limits: [], held: undefined, reserved: "1.000000000001" }), now), creditUnits("9.123456789011"));
  assert.equal(spendableGrant(grant({ remaining: null }), now), 0n);
  assert.equal(spendableGrant(grant({ held: undefined, reserved: null }), now), 0n);
  const rules = [
    { ...capped.service_limits[0]!, limit: "50", consumed: "49", held: "0.1" },
    { ...capped.service_limits[0]!, key: "second", service_ids: ["second"], service_slugs: ["other"], limit: "2", consumed: "0", held: "0.000000000001" },
  ];
  assert.equal(spendableGrant(grant({ scope: { all_services: false, service_ids: ["service", "second"], service_slugs: ["example", "other"] }, service_limits: rules }), now), creditUnits("2.899999999999"));
  assert.equal(spendableGrant(grant({ scope: { all_services: false, service_ids: [], service_slugs: [] } }), now), 0n);
});

test("expired, pending, revoked, reclaim-pending and consumed grants are excluded", () => {
  for (const overrides of [
    { expires_at: "2026-10-28T00:00:00Z" }, { status: "expired" }, { status: "consumed" },
    { status: "revoked" }, { activation_state: "pending_activation" }, { revoke_requested_at: "2026-10-27T00:00:00Z" },
  ]) assert.equal(spendableGrant(grant(overrides), now), 0n);
});

test("expiring-soon row contains only the earliest dated spendable credits, grouped exactly", () => {
  const rows = balanceRows(wallet, [
    grant({ id: "one", service_limits: [], expires_at: "2026-10-30T00:00:00Z" }),
    grant({ id: "two", service_limits: [], expires_at: "2026-10-30T00:00:00Z" }),
    grant({ id: "later", expires_at: "2026-10-31T00:00:00Z" }),
    grant({ id: "far", expires_at: "2027-10-30T00:00:00Z" }),
    grant({ id: "expired", expires_at: "2026-10-27T00:00:00Z" }),
  ], [], now);
  const expiring = rows.find((row) => row.key === "expiring")!;
  assert.equal(expiring.value, "20.246914");
  assert.equal(expiring.exactValue, "20.246913578022");
  assert.match(expiring.label, /Oct 30, 2026.*UTC/);
  assert.equal(rows.filter((row) => row.key === "expiring").length, 1);
  assert.equal(balanceRows(wallet, [grant({ expires_at: null })], [], now).some((row) => row.key === "expiring"), false);
});

test("allowances show only current active spendable remainders and their period limits", () => {
  const rows = balanceRows(wallet, [], [
    allowance(), allowance({ remaining_quantity: 0 }),
    allowance({ allowance: { ...allowanceFixture.allowance, is_active: false } }),
    allowance({ period_start: "2026-10-29T00:00:00Z" }),
    allowance({ period_end: "2026-10-28T00:00:00Z" }),
  ], now);
  assert.equal(rows.length, 1);
  assert.equal(rows[0]!.label, "Included allowance · example");
  assert.equal(rows[0]!.value, "690 / 1,000 requests left this month");
});
