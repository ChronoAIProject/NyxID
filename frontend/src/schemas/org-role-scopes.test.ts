import { describe, expect, it } from "vitest";
import {
  orgRoleScopeSchema,
  orgRoleScopesResponseSchema,
  updateRoleScopeRequestSchema,
} from "./org-role-scopes";

describe("orgRoleScopeSchema", () => {
  it("accepts a persisted role scope", () => {
    expect(
      orgRoleScopeSchema.safeParse({
        role: "member",
        allowed_service_ids: ["svc-1"],
        is_default: false,
        updated_at: "2026-01-01T00:00:00Z",
        updated_by: "user-1",
      }).success,
    ).toBe(true);
  });

  it("accepts a synthesized default role scope", () => {
    expect(
      orgRoleScopeSchema.safeParse({
        role: "viewer",
        allowed_service_ids: null,
        is_default: true,
        updated_at: null,
        updated_by: null,
      }).success,
    ).toBe(true);
  });
});

describe("orgRoleScopesResponseSchema", () => {
  it("accepts the role-scopes list response", () => {
    expect(
      orgRoleScopesResponseSchema.safeParse({
        role_scopes: [
          {
            role: "admin",
            allowed_service_ids: null,
            is_default: true,
            updated_at: null,
            updated_by: null,
          },
        ],
      }).success,
    ).toBe(true);
  });
});

describe("updateRoleScopeRequestSchema", () => {
  it("accepts null for full access and an array for restricted access", () => {
    expect(
      updateRoleScopeRequestSchema.safeParse({
        allowed_service_ids: null,
      }).success,
    ).toBe(true);
    expect(
      updateRoleScopeRequestSchema.safeParse({
        allowed_service_ids: ["svc-1"],
      }).success,
    ).toBe(true);
  });
});
