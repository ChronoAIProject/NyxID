import { describe, expect, it } from "vitest";
import { canManageCreditGrants, canReadAdminUsage, isBillingAvailable } from "./api";

describe("isBillingAvailable", () => {
  it("fails closed when user capabilities are absent", () => {
    expect(isBillingAvailable(null)).toBe(false);
    expect(isBillingAvailable({})).toBe(false);
  });

  it("returns false when billing is explicitly unavailable", () => {
    expect(
      isBillingAvailable({
        capabilities: { billing_available: false },
      }),
    ).toBe(false);
  });

  it("returns true only when the backend marks billing available", () => {
    expect(
      isBillingAvailable({
        capabilities: { billing_available: true },
      }),
    ).toBe(true);
  });
});

describe("canManageCreditGrants", () => {
  it("allows admins and operators the backend marks as credits managers", () => {
    expect(canManageCreditGrants({ is_admin: true, role: "admin" })).toBe(true);
    expect(
      canManageCreditGrants({
        is_admin: false,
        role: "operator",
        capabilities: { manage_credit_grants: true },
      }),
    ).toBe(true);
  });

  it("denies plain operators, users and older backends", () => {
    expect(canManageCreditGrants({ is_admin: false, role: "operator" })).toBe(false);
    expect(
      canManageCreditGrants({
        is_admin: false,
        role: "user",
        capabilities: { manage_credit_grants: false },
      }),
    ).toBe(false);
    expect(canManageCreditGrants(null)).toBe(false);
  });
});

describe("canReadAdminUsage", () => {
  it("allows admin readers and users the backend marks as usage readers", () => {
    expect(canReadAdminUsage({ is_admin: true, role: "admin" })).toBe(true);
    expect(canReadAdminUsage({ is_admin: false, role: "operator" })).toBe(true);
    expect(
      canReadAdminUsage({
        is_admin: false,
        role: "user",
        capabilities: { read_admin_usage: true },
      }),
    ).toBe(true);
  });

  it("denies plain users and older backends", () => {
    expect(
      canReadAdminUsage({
        is_admin: false,
        role: "user",
        capabilities: { read_admin_usage: false },
      }),
    ).toBe(false);
    expect(canReadAdminUsage({ is_admin: false, role: "user" })).toBe(false);
    expect(canReadAdminUsage(null)).toBe(false);
  });
});
