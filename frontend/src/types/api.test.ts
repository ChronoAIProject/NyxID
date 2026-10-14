import { describe, expect, it } from "vitest";
import type { User, DownstreamService } from "./api";
import {
  canManageTools,
  canWriteTool,
  canManageCreditGrants,
  canReadAdminUsage,
  isBillingAvailable,
} from "./api";

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
    expect(canManageCreditGrants({ is_admin: false, role: "operator" })).toBe(
      false,
    );
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

describe("Tools Manager capability and ownership", () => {
  it("fails closed without a backend capability and preserves Admin fallback", () => {
    expect(canManageTools(null)).toBe(false);
    expect(
      canManageTools({ is_admin: false, is_operator: true, role: "operator" }),
    ).toBe(false);
    expect(
      canManageTools({
        is_admin: false,
        capabilities: { manage_catalog: true },
      }),
    ).toBe(false);
    expect(canManageTools({ is_admin: true })).toBe(true);
    expect(
      canManageTools({ is_admin: false, capabilities: { manage_tools: true } }),
    ).toBe(true);
  });
  it("requires own marked Tool rows and loses authority on revocation", () => {
    const user = {
      id: "manager",
      is_admin: false,
      capabilities: { manage_tools: true },
    } as User;
    const tool = {
      offering_kind: "tool",
      service_type: "http",
      service_category: "internal",
      requires_user_credential: false,
      created_by: "manager",
      tools_manager_created: true,
      tools_manager_eligible: true,
    } as DownstreamService;
    expect(canWriteTool(user, tool)).toBe(true);
    expect(canWriteTool(user, { ...tool, created_by: "foreign" })).toBe(false);
    expect(canWriteTool(user, { ...tool, owner_user_id: "foreign" })).toBe(
      false,
    );
    expect(canWriteTool(user, { ...tool, tools_manager_created: false })).toBe(
      false,
    );
    // Server evaluates every shape field, including secret-bearing token exchange.
    expect(canWriteTool(user, { ...tool, tools_manager_eligible: false })).toBe(false);
    expect(canWriteTool(user, { ...tool, tools_manager_eligible: undefined })).toBe(false);
    expect(
      canWriteTool({ ...user, capabilities: { manage_tools: false } }, tool),
    ).toBe(false);
  });
});
