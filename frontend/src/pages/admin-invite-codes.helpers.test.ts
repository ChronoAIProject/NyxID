import { describe, expect, it } from "vitest";
import { flattenRedemptions } from "./admin-invite-codes.helpers";
import type { InviteCode } from "@/types/admin";

describe("flattenRedemptions", () => {
  it("flattens multiple codes x multiple usages into correct total row count, each carrying correct code/note/user fields", () => {
    const inviteCodes: InviteCode[] = [
      {
        id: "code-1",
        code: "INVITE-100",
        max_uses: 10,
        used_count: 2,
        created_by: "admin-uuid",
        creator: { email: "admin@corp", display_name: "Admin" },
        note: "First invite code",
        is_active: true,
        created_at: "2026-01-01T00:00:00Z",
        updated_at: "2026-01-01T00:00:00Z",
        usages: [
          {
            user_id: "user-1",
            used_at: "2026-01-02T10:00:00Z",
            user_email: "user1@example.com",
            user_display_name: "User One",
          },
          {
            user_id: "user-2",
            used_at: "2026-01-03T11:00:00Z",
            user_email: "user2@example.com",
            user_display_name: "User Two",
          },
        ],
      },
      {
        id: "code-2",
        code: "INVITE-200",
        max_uses: 5,
        used_count: 1,
        created_by: "admin-uuid",
        creator: { email: "admin@corp", display_name: "Admin" },
        note: "Second invite code",
        is_active: true,
        created_at: "2026-01-01T00:00:00Z",
        updated_at: "2026-01-01T00:00:00Z",
        usages: [
          {
            user_id: "user-3",
            used_at: "2026-01-04T12:00:00Z",
            user_email: "user3@example.com",
            user_display_name: "User Three",
          },
        ],
      },
    ];

    const result = flattenRedemptions(inviteCodes);

    expect(result).toHaveLength(3);

    // Verify fields of the first (latest) row, which should be user-3 usedAt "2026-01-04T12:00:00Z"
    expect(result[0]).toEqual({
      id: "code-2-user-3-2026-01-04T12:00:00Z",
      code: "INVITE-200",
      codeId: "code-2",
      note: "Second invite code",
      userId: "user-3",
      userEmail: "user3@example.com",
      userDisplayName: "User Three",
      usedAt: "2026-01-04T12:00:00Z",
    });

    // Verify other rows are present with correct properties
    expect(result.some(r => r.userId === "user-1" && r.code === "INVITE-100" && r.note === "First invite code")).toBe(true);
    expect(result.some(r => r.userId === "user-2" && r.code === "INVITE-100" && r.note === "First invite code")).toBe(true);
  });

  it("orders rows by usedAt descending", () => {
    const inviteCodes: InviteCode[] = [
      {
        id: "code-1",
        code: "INVITE-100",
        max_uses: 10,
        used_count: 3,
        created_by: "admin-uuid",
        creator: null,
        note: null,
        is_active: true,
        created_at: "2026-01-01T00:00:00Z",
        updated_at: "2026-01-01T00:00:00Z",
        usages: [
          {
            user_id: "user-1",
            used_at: "2026-01-02T10:00:00Z",
            user_email: "user1@example.com",
            user_display_name: "User One",
          },
          {
            user_id: "user-2",
            used_at: "2026-01-05T10:00:00Z",
            user_email: "user2@example.com",
            user_display_name: "User Two",
          },
          {
            user_id: "user-3",
            used_at: "2026-01-03T10:00:00Z",
            user_email: "user3@example.com",
            user_display_name: "User Three",
          },
        ],
      },
    ];

    const result = flattenRedemptions(inviteCodes);

    expect(result).toHaveLength(3);
    expect(result[0]!.userId).toBe("user-2"); // latest: Jan 5
    expect(result[1]!.userId).toBe("user-3"); // middle: Jan 3
    expect(result[2]!.userId).toBe("user-1"); // oldest: Jan 2
  });

  it("code with usages: [] contributes zero rows", () => {
    const inviteCodes: InviteCode[] = [
      {
        id: "code-1",
        code: "INVITE-100",
        max_uses: 10,
        used_count: 0,
        created_by: "admin-uuid",
        creator: null,
        note: null,
        is_active: true,
        created_at: "2026-01-01T00:00:00Z",
        updated_at: "2026-01-01T00:00:00Z",
        usages: [],
      },
    ];

    const result = flattenRedemptions(inviteCodes);
    expect(result).toHaveLength(0);
  });

  it("passes through null names/emails (deleted user)", () => {
    const inviteCodes: InviteCode[] = [
      {
        id: "code-1",
        code: "INVITE-100",
        max_uses: 10,
        used_count: 1,
        created_by: "admin-uuid",
        creator: null,
        note: null,
        is_active: true,
        created_at: "2026-01-01T00:00:00Z",
        updated_at: "2026-01-01T00:00:00Z",
        usages: [
          {
            user_id: "deleted-user-id",
            used_at: "2026-01-02T10:00:00Z",
            user_email: null,
            user_display_name: null,
          },
        ],
      },
    ];

    const result = flattenRedemptions(inviteCodes);
    expect(result).toHaveLength(1);
    expect(result[0]!.userEmail).toBeNull();
    expect(result[0]!.userDisplayName).toBeNull();
  });

  it("passes through note values (null and string)", () => {
    const inviteCodes: InviteCode[] = [
      {
        id: "code-1",
        code: "INVITE-100",
        max_uses: 10,
        used_count: 1,
        created_by: "admin-uuid",
        creator: null,
        note: "A nice string note",
        is_active: true,
        created_at: "2026-01-01T00:00:00Z",
        updated_at: "2026-01-01T00:00:00Z",
        usages: [
          {
            user_id: "user-1",
            used_at: "2026-01-02T10:00:00Z",
            user_email: "user1@example.com",
            user_display_name: "User One",
          },
        ],
      },
      {
        id: "code-2",
        code: "INVITE-200",
        max_uses: 10,
        used_count: 1,
        created_by: "admin-uuid",
        creator: null,
        note: null,
        is_active: true,
        created_at: "2026-01-01T00:00:00Z",
        updated_at: "2026-01-01T00:00:00Z",
        usages: [
          {
            user_id: "user-2",
            used_at: "2026-01-02T11:00:00Z",
            user_email: "user2@example.com",
            user_display_name: "User Two",
          },
        ],
      },
    ];

    const result = flattenRedemptions(inviteCodes);
    expect(result).toHaveLength(2);
    // latest is user-2 (code-2) which has null note
    expect(result[0]!.note).toBeNull();
    // oldest is user-1 (code-1) which has string note
    expect(result[1]!.note).toBe("A nice string note");
  });
});
