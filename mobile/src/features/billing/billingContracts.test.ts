import assert from "node:assert/strict";
import test from "node:test";
import { allowancesSchema, billingOrganizationsSchema, grantsSchema, serviceUsage, usageSchema, walletSchema } from "./billingContracts";
import { billingDate, creditUnits, decimalCredits, expiryLabel, formatCredits, legacyCredits, sumCredits } from "./credits";
import { allowanceFixture, grantFixture, organizationsFixture, ORG_ADMIN, ORG_MEMBER, usageFixture, walletFixture } from "./billingFixtures";

test("decimal credits preserve all twelve places and integers beyond floating-point precision", () => {
  for (const [raw, formatted] of [
    ["9007199254740993.000000000001", "9,007,199,254,740,993.000000000001"],
    ["-0.000000000001", "-0.000000000001"], ["-123456.120000000000", "-123,456.12"],
    ["-0.000", "0"], ["0001.2300", "1.23"],
    ["9999999999999999999999.999999999999", "9,999,999,999,999,999,999,999.999999999999"],
  ]) assert.equal(formatCredits(raw), formatted);
  assert.equal(sumCredits(["9007199254740993.000000000001", "0.000000000002"]), "9007199254740993.000000000003");
  assert.equal(decimalCredits(creditUnits("-1.23")), "-1.23");
  assert.equal(formatCredits(null), "Unavailable");
  assert.equal(sumCredits(["1", null]), null, "unknown costs are never turned into zero");
});

test("invalid, nonfinite, oversized and sub-pico credit amounts fail closed", () => {
  for (const value of ["NaN", "Infinity", "1e3", " 1", "+1", "1.", ".1", "0.0000000000001", "10000000000000000000000", "9".repeat(100)]) {
    assert.throws(() => creditUnits(value));
    assert.throws(() => walletSchema.parse({ ...walletFixture, balance: value }));
  }
  assert.throws(() => walletSchema.parse({ ...walletFixture, balance: 0.1 }));
});

test("wallet view needs only exact balances and strips legacy and provider metadata", () => {
  const wallet = walletSchema.parse({ ...walletFixture, balance_credits: 9007199254740992 });
  assert.equal(wallet.balance, "-0.000000000001");
  assert.equal(wallet.available, "-1.334567890126");
  assert.deepEqual(Object.keys(wallet).sort(), ["available", "balance", "overdraftCap", "pendingDebits", "pendingExpiry", "reserved", "unit"]);
  assert.deepEqual(walletSchema.parse({ ...walletFixture, lago_customer_id: "unused", owner_id: "unused", created: true }), wallet);
  assert.equal(walletSchema.parse({ ...walletFixture, unit: undefined }).unit, "credits");
  assert.throws(() => walletSchema.parse({ ...walletFixture, unit: "USD" }));
  assert.throws(() => walletSchema.parse({ ...walletFixture, balance: null }));
  assert.throws(() => walletSchema.parse({ ...walletFixture, balance: undefined, balance_credits: -123 }));
});

test("usage exact money wins over legacy mirrors, including authoritative nulls", () => {
  assert.equal(legacyCredits(-1234567, "micros"), "-1.234567");
  const row = { ...usageFixture.rows[0], estimated_credits: null, estimated_credits_micros: 1000000 };
  assert.equal(usageSchema.parse({ ...usageFixture, rows: [row] }).rows[0]!.cost, null);
});

test("service usage sums costs exactly, retains unknown prices and distinguishes metering-only usage", () => {
  const parsed = usageSchema.parse(usageFixture);
  const services = serviceUsage(parsed.rows);
  assert.equal(services.length, 1);
  assert.equal(services[0]!.cost, "9007199254740993.000000000003");
  assert.equal(services[0]!.wallet, "0.000000000003");
  assert.equal(services[0]!.rows.length, 2);
  assert.equal(serviceUsage([{ ...parsed.rows[0]!, cost: null }])[0]!.cost, null);
  assert.equal(serviceUsage([{ ...parsed.rows[0]!, cost: null, billable: false }])[0]!.cost, "0");
  assert.equal(serviceUsage([{ ...parsed.rows[0]!, billable: false }])[0]!.meteringOnly, true);
});

test("grant expiry, service limits and allowance periods retain their exact values", () => {
  const grant = grantsSchema.parse({ grants: [grantFixture] }).grants[0]!;
  assert.equal(grant.remaining, "0.000000000001");
  assert.equal(grant.service_limits[0]!.limit, "50.000000000001");
  assert.match(expiryLabel(grant.expires_at, Date.parse("2026-10-28T00:00:00Z")), /^Expires .*2026.*\(UTC\)$/);
  assert.match(expiryLabel(grant.expires_at, Date.parse("2026-11-01T00:00:00Z")), /^Expired /);
  assert.equal(expiryLabel(null), "No expiry");
  assert.match(billingDate(allowanceFixture.period_start), /2026/);
  assert.equal(allowancesSchema.parse({ allowances: [allowanceFixture] }).allowances[0]!.remaining_quantity, 690);
  assert.throws(() => grantsSchema.parse({ grants: [{ ...grantFixture, expires_at: "not-a-date" }] }));
  assert.throws(() => allowancesSchema.parse({ allowances: [{ ...allowanceFixture, period_end: "invalid" }] }));
  assert.throws(() => allowancesSchema.parse({ allowances: [{ ...allowanceFixture, remaining_quantity: -1 }] }));
});

test("benefit account options allow org Admins and Members, excluding Viewers and unknown roles", () => {
  const rows = billingOrganizationsSchema.parse({ orgs: [...organizationsFixture.orgs, { ...organizationsFixture.orgs[0], your_role: "future-role" }] });
  assert.deepEqual(rows.map((row) => row.id), [ORG_ADMIN, ORG_MEMBER]);
});
