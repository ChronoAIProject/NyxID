import { describe, it, expect } from "vitest";
import {
  createRoleSchema,
  updateRoleSchema,
  createGroupSchema,
  updateGroupSchema,
} from "./rbac";

describe("createRoleSchema", () => {
  const base = { name: "Admin", slug: "admin", is_default: false };

  it("accepts a minimal valid role", () => {
    expect(createRoleSchema.safeParse(base).success).toBe(true);
  });

  it("enforces the slug character set", () => {
    expect(createRoleSchema.safeParse({ ...base, slug: "team_lead-1" }).success).toBe(true);
    expect(createRoleSchema.safeParse({ ...base, slug: "Team Lead" }).success).toBe(false);
    expect(createRoleSchema.safeParse({ ...base, slug: "" }).success).toBe(false);
  });

  it("rejects names and descriptions over their limits", () => {
    expect(createRoleSchema.safeParse({ ...base, name: "a".repeat(101) }).success).toBe(false);
    expect(
      createRoleSchema.safeParse({ ...base, description: "d".repeat(501) }).success,
    ).toBe(false);
  });

  it("allows optional text fields to be empty strings", () => {
    expect(
      createRoleSchema.safeParse({
        ...base,
        description: "",
        permissions: "",
        client_id: "",
      }).success,
    ).toBe(true);
  });

  it("requires the is_default boolean", () => {
    const withoutFlag: Partial<typeof base> = { ...base };
    delete withoutFlag.is_default;
    expect(createRoleSchema.safeParse(withoutFlag).success).toBe(false);
  });
});

describe("updateRoleSchema", () => {
  it("validates the same name/slug rules but has no client_id field", () => {
    const result = updateRoleSchema.safeParse({
      name: "Admin",
      slug: "admin",
      is_default: true,
      client_id: "should-be-ignored",
    });
    expect(result.success).toBe(true);
    if (result.success) {
      expect("client_id" in result.data).toBe(false);
    }
  });
});

describe("group schemas", () => {
  const groupBase = { name: "Engineers", slug: "engineers" };

  it("createGroupSchema accepts role_ids and parent_group_id as optional empties", () => {
    expect(
      createGroupSchema.safeParse({ ...groupBase, role_ids: "", parent_group_id: "" }).success,
    ).toBe(true);
  });

  it("updateGroupSchema enforces the slug regex", () => {
    expect(updateGroupSchema.safeParse({ ...groupBase, slug: "Bad Slug" }).success).toBe(false);
    expect(updateGroupSchema.safeParse(groupBase).success).toBe(true);
  });
});
