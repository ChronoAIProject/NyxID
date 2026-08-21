import { describe, expect, it } from "vitest";
import {
  allowanceFormSchema,
  creditGrantListSchema,
  issueGrantFormSchema,
  userAllowanceListSchema,
} from "./billing-credits";

describe("billing credit schemas", () => {
  it("validates grant target, service scope, bounds, and expiry", () => {
    const valid = {
      amount_credits: 100,
      target_kind: "selected_users" as const,
      target_user_ids: ["user-1"],
      all_services: false,
      service_refs: ["service-1"],
      expires_at: "2099-01-01T00:00",
      reason: "Launch credits",
    };

    expect(issueGrantFormSchema.safeParse(valid).success).toBe(true);
    expect(
      issueGrantFormSchema.safeParse({ ...valid, target_user_ids: [] }).success,
    ).toBe(false);
    expect(
      issueGrantFormSchema.safeParse({ ...valid, service_refs: [] }).success,
    ).toBe(false);
    expect(
      issueGrantFormSchema.safeParse({ ...valid, amount_credits: 1_000_001 })
        .success,
    ).toBe(false);
    expect(
      issueGrantFormSchema.safeParse({ ...valid, expires_at: "not-a-date" })
        .success,
    ).toBe(false);
  });

  it("requires selected allowance owners and bounded whole units", () => {
    const valid = {
      service_ref: "service-1",
      quantity: 1_000,
      recurrence: "monthly" as const,
      target_kind: "selected_users" as const,
      target_user_ids: ["org-1"],
    };

    expect(allowanceFormSchema.safeParse(valid).success).toBe(true);
    expect(
      allowanceFormSchema.safeParse({ ...valid, target_user_ids: [] }).success,
    ).toBe(false);
    expect(
      allowanceFormSchema.safeParse({ ...valid, quantity: 1.5 }).success,
    ).toBe(false);
  });

  it("parses active user grant and allowance response contracts", () => {
    expect(
      creditGrantListSchema.safeParse({
        grants: [
          {
            id: "grant-1",
            batch_id: "batch-1",
            recipient_user_id: "user-1",
            target_kind: "selected_users",
            amount_credits: 2,
            amount_micros: 2_000_000,
            remaining_micros: 1_500_000,
            reserved_micros: 0,
            scope: {
              all_services: true,
              service_ids: [],
              service_slugs: [],
            },
            granted_by: "admin-1",
            status: "active",
            created_at: "2026-08-21T00:00:00Z",
            updated_at: "2026-08-21T00:00:00Z",
          },
        ],
        page: 1,
        per_page: 1,
        total: 1,
      }).success,
    ).toBe(true);

    expect(
      userAllowanceListSchema.safeParse({
        allowances: [
          {
            allowance: {
              id: "allowance-1",
              service_id: "service-1",
              service_slug: "llm-one",
              metric: "tokens",
              quantity: 1000,
              recurrence: "daily",
              target_kind: "all_users",
              target_user_ids: [],
              is_active: true,
              created_by: "admin-1",
              created_at: "2026-08-21T00:00:00Z",
              updated_at: "2026-08-21T00:00:00Z",
            },
            period_start: "2026-08-21T00:00:00Z",
            period_end: "2026-08-22T00:00:00Z",
            consumed_quantity: 250,
            reserved_quantity: 50,
            remaining_quantity: 700,
          },
        ],
      }).success,
    ).toBe(true);
  });
});
