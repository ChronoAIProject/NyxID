import { describe, it, expect } from "vitest";
import {
  createInviteRequestSchema,
  createOrgRequestSchema,
  credentialSourceSchema,
  inviteResponseSchema,
  memberResponseSchema,
  orgListItemSchema,
  orgResponseSchema,
  orgRoleSchema,
  orgSlugSchema,
  ORG_ROLES,
  scopeSourceSchema,
  updateMemberRequestSchema,
  updateOrgRequestSchema,
} from "./orgs";

describe("orgRoleSchema", () => {
  it("accepts the three canonical roles", () => {
    for (const role of ORG_ROLES) {
      expect(orgRoleSchema.safeParse(role).success).toBe(true);
    }
  });

  it("rejects unknown roles", () => {
    expect(orgRoleSchema.safeParse("superadmin").success).toBe(false);
    expect(orgRoleSchema.safeParse("Admin").success).toBe(false);
  });
});

describe("scopeSourceSchema", () => {
  it("accepts inherit and override", () => {
    expect(scopeSourceSchema.safeParse("inherit").success).toBe(true);
    expect(scopeSourceSchema.safeParse("override").success).toBe(true);
    expect(scopeSourceSchema.safeParse("custom").success).toBe(false);
  });
});

describe("orgListItemSchema", () => {
  it("accepts a wire-format list item", () => {
    const result = orgListItemSchema.safeParse({
      id: "org-1",
      slug: "chrono-ai",
      display_name: "Chrono AI",
      avatar_url: null,
      contact_email: "contact@chrono.ai",
      your_role: "admin",
      created_at: "2026-01-01T00:00:00Z",
    });
    expect(result.success).toBe(true);
  });

  it("allows null display_name, avatar_url, and contact_email", () => {
    const result = orgListItemSchema.safeParse({
      id: "org-1",
      slug: "chrono-ai",
      display_name: null,
      avatar_url: null,
      contact_email: null,
      your_role: "viewer",
      created_at: "2026-01-01T00:00:00Z",
    });
    expect(result.success).toBe(true);
  });
});

describe("orgResponseSchema", () => {
  it("requires a non-negative member_count", () => {
    const base = {
      id: "org-1",
      slug: "acme",
      display_name: "Acme",
      avatar_url: null,
      contact_email: null,
      created_at: "2026-01-01T00:00:00Z",
      your_role: "member" as const,
    };
    expect(
      orgResponseSchema.safeParse({ ...base, member_count: 3 }).success,
    ).toBe(true);
    expect(
      orgResponseSchema.safeParse({ ...base, member_count: -1 }).success,
    ).toBe(false);
  });

  it("accepts a contact_email string", () => {
    const result = orgResponseSchema.safeParse({
      id: "org-1",
      slug: "acme",
      display_name: "Acme",
      avatar_url: null,
      contact_email: "contact@acme.test",
      created_at: "2026-01-01T00:00:00Z",
      your_role: "admin",
      member_count: 1,
    });
    expect(result.success).toBe(true);
  });
});

describe("memberResponseSchema", () => {
  it("accepts a full member response", () => {
    const result = memberResponseSchema.safeParse({
      membership_id: "m-1",
      user_id: "u-1",
      display_name: "Alice",
      email: "alice@example.com",
      role: "admin",
      scope_source: "override",
      allowed_service_ids: ["svc-1"],
      effective_allowed_service_ids: ["svc-1"],
      created_at: "2026-01-01T00:00:00Z",
      revoked_at: null,
    });
    expect(result.success).toBe(true);
  });

  it("allows null allowed_service_ids for unrestricted access", () => {
    const result = memberResponseSchema.safeParse({
      membership_id: "m-1",
      user_id: "u-1",
      display_name: null,
      email: null,
      role: "member",
      scope_source: "inherit",
      allowed_service_ids: null,
      effective_allowed_service_ids: null,
      created_at: "2026-01-01T00:00:00Z",
      revoked_at: null,
    });
    expect(result.success).toBe(true);
  });
});

describe("inviteResponseSchema", () => {
  it("accepts a pending invite", () => {
    const result = inviteResponseSchema.safeParse({
      id: "inv-1",
      nonce: "abcd1234",
      role: "member",
      scope_source: "inherit",
      allowed_service_ids: null,
      created_by: "u-1",
      expires_at: "2026-01-02T00:00:00Z",
      redeemed_by: null,
      redeemed_at: null,
      created_at: "2026-01-01T00:00:00Z",
    });
    expect(result.success).toBe(true);
  });

  it("accepts a redeemed invite with redeemer identity fields", () => {
    const result = inviteResponseSchema.safeParse({
      id: "inv-2",
      nonce: "redeemed1234",
      role: "viewer",
      scope_source: "override",
      allowed_service_ids: null,
      created_by: "u-1",
      expires_at: "2026-02-02T00:00:00Z",
      redeemed_by: "u-2",
      redeemed_by_email: "alice@example.com",
      redeemed_by_display_name: "Alice",
      redeemed_at: "2026-01-15T00:00:00Z",
      created_at: "2026-01-01T00:00:00Z",
    });
    expect(result.success).toBe(true);
  });
});

describe("createOrgRequestSchema", () => {
  it("requires a non-empty display_name", () => {
    expect(
      createOrgRequestSchema.safeParse({ display_name: "" }).success,
    ).toBe(false);
    expect(
      createOrgRequestSchema.safeParse({ display_name: "Acme" }).success,
    ).toBe(true);
  });

  it("rejects invalid contact_email when provided", () => {
    expect(
      createOrgRequestSchema.safeParse({
        display_name: "Acme",
        contact_email: "not-an-email",
      }).success,
    ).toBe(false);
  });

  it("allows empty contact_email as a sentinel for absent", () => {
    expect(
      createOrgRequestSchema.safeParse({
        display_name: "Acme",
        contact_email: "",
      }).success,
    ).toBe(true);
  });

  it("rejects display_name longer than 128 chars", () => {
    expect(
      createOrgRequestSchema.safeParse({
        display_name: "a".repeat(129),
      }).success,
    ).toBe(false);
  });
});

describe("orgSlugSchema", () => {
  it("accepts lowercase slugs with digits and hyphens", () => {
    expect(orgSlugSchema.safeParse("chrono-ai-2").success).toBe(true);
  });

  it("rejects uppercase, edge hyphens, and uuid-shaped values", () => {
    expect(orgSlugSchema.safeParse("Chrono-AI").success).toBe(false);
    expect(orgSlugSchema.safeParse("-chrono").success).toBe(false);
    expect(orgSlugSchema.safeParse("chrono-").success).toBe(false);
    expect(
      orgSlugSchema.safeParse("550e8400-e29b-41d4-a716-446655440000")
        .success,
    ).toBe(false);
  });
});

describe("updateOrgRequestSchema", () => {
  it("accepts an empty object (no changes)", () => {
    expect(updateOrgRequestSchema.safeParse({}).success).toBe(true);
  });

  it("accepts a valid contact_email", () => {
    expect(
      updateOrgRequestSchema.safeParse({
        contact_email: "contact@example.com",
      }).success,
    ).toBe(true);
  });

  it("treats an empty contact_email as a sentinel for clear", () => {
    expect(
      updateOrgRequestSchema.safeParse({ contact_email: "" }).success,
    ).toBe(true);
  });

  it("rejects an invalid contact_email", () => {
    expect(
      updateOrgRequestSchema.safeParse({
        contact_email: "not-an-email",
      }).success,
    ).toBe(false);
  });
});

describe("createInviteRequestSchema", () => {
  it("accepts a minimal request", () => {
    const result = createInviteRequestSchema.safeParse({ role: "member" });
    expect(result.success).toBe(true);
  });

  it("rejects non-positive ttl_hours", () => {
    expect(
      createInviteRequestSchema.safeParse({ role: "member", ttl_hours: 0 })
        .success,
    ).toBe(false);
    expect(
      createInviteRequestSchema.safeParse({ role: "member", ttl_hours: -1 })
        .success,
    ).toBe(false);
  });

  it("rejects ttl_hours beyond 30 days", () => {
    expect(
      createInviteRequestSchema.safeParse({
        role: "member",
        ttl_hours: 24 * 31,
      }).success,
    ).toBe(false);
  });
});

describe("updateMemberRequestSchema", () => {
  it("accepts explicit null allowed_service_ids to clear scope", () => {
    expect(
      updateMemberRequestSchema.safeParse({
        allowed_service_ids: null,
      }).success,
    ).toBe(true);
  });

  it("accepts an array allowed_service_ids to restrict scope", () => {
    expect(
      updateMemberRequestSchema.safeParse({
        allowed_service_ids: ["svc-1", "svc-2"],
      }).success,
    ).toBe(true);
  });

  it("accepts scope_source changes", () => {
    expect(
      updateMemberRequestSchema.safeParse({
        scope_source: "inherit",
      }).success,
    ).toBe(true);
    expect(
      updateMemberRequestSchema.safeParse({
        scope_source: "override",
        allowed_service_ids: [],
      }).success,
    ).toBe(true);
  });
});

describe("credentialSourceSchema", () => {
  it("accepts personal source", () => {
    expect(
      credentialSourceSchema.safeParse({ type: "personal" }).success,
    ).toBe(true);
  });

  it("accepts org source with all fields", () => {
    const result = credentialSourceSchema.safeParse({
      type: "org",
      org_id: "org-1",
      org_name: "Chrono AI",
      avatar_url: null,
      role: "member",
      allowed: true,
    });
    expect(result.success).toBe(true);
  });

  it("accepts org source with an avatar url so /keys can render it", () => {
    const result = credentialSourceSchema.safeParse({
      type: "org",
      org_id: "org-1",
      org_name: "Chrono AI",
      avatar_url: "https://example.com/orgs/chrono.png",
      role: "admin",
      allowed: true,
    });
    expect(result.success).toBe(true);
    if (result.success && result.data.type === "org") {
      expect(result.data.avatar_url).toBe(
        "https://example.com/orgs/chrono.png",
      );
    }
  });

  it("treats omitted avatar_url as undefined for backwards compat", () => {
    const result = credentialSourceSchema.safeParse({
      type: "org",
      org_id: "org-1",
      org_name: "Chrono AI",
      role: "member",
      allowed: true,
    });
    expect(result.success).toBe(true);
    if (result.success && result.data.type === "org") {
      expect(result.data.avatar_url ?? null).toBeNull();
    }
  });

  it("rejects org source missing required fields", () => {
    const result = credentialSourceSchema.safeParse({
      type: "org",
      org_id: "org-1",
    });
    expect(result.success).toBe(false);
  });

  it("discriminates between personal and org via the type tag", () => {
    const result = credentialSourceSchema.safeParse({
      type: "personal",
      org_id: "org-1",
    });
    // type=personal ignores extra keys
    expect(result.success).toBe(true);
  });

  it("rejects unknown discriminator values", () => {
    const result = credentialSourceSchema.safeParse({
      type: "team",
    });
    expect(result.success).toBe(false);
  });
});
