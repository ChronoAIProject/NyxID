import { describe, expect, it } from "vitest";
import {
  allowanceFormSchema,
  creditGrantListSchema,
  creditScheduleListSchema,
  issueGrantFormSchema,
  scheduleFormSchema,
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
            activation_state: "active",
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

  it("validates schedule expiry, targets, and service scope", () => {
    const valid = {
      amount_credits: 50,
      recurrence: "monthly" as const,
      expiry: { kind: "end_of_period" as const },
      target_kind: "selected_users" as const,
      target_user_ids: ["owner-1"],
      all_services: false,
      service_refs: ["service-1"],
      reason: "Monthly builder credits",
    };

    expect(scheduleFormSchema.safeParse(valid).success).toBe(true);
    expect(
      scheduleFormSchema.safeParse({
        ...valid,
        expiry: { kind: "after_days", days: 30 },
      }).success,
    ).toBe(true);
    expect(
      scheduleFormSchema.safeParse({
        ...valid,
        expiry: { kind: "after_days" },
      }).success,
    ).toBe(false);
    expect(
      scheduleFormSchema.safeParse({ ...valid, target_user_ids: [] }).success,
    ).toBe(false);
    expect(
      scheduleFormSchema.safeParse({
        ...valid,
        target_kind: "all_users",
        target_user_ids: ["owner-1"],
      }).success,
    ).toBe(false);
    expect(
      scheduleFormSchema.safeParse({ ...valid, service_refs: [] }).success,
    ).toBe(false);
  });

  it("parses schedule progress and grant schedule provenance", () => {
    expect(
      creditScheduleListSchema.safeParse({
        schedules: [
          {
            id: "schedule-1",
            amount_credits: 50,
            amount_micros: 50_000_000,
            recurrence: "monthly",
            expiry: { kind: "end_of_period" },
            target_kind: "selected_users",
            target_user_ids: ["owner-1"],
            scope: {
              all_services: true,
              service_ids: [],
              service_slugs: [],
            },
            reason: "Monthly builder credits",
            is_active: true,
            created_by: "admin-1",
            created_at: "2026-08-01T00:00:00Z",
            updated_at: "2026-08-01T00:00:00Z",
            skipped_periods: 0,
            current_period: {
              start: "2026-08-01T00:00:00Z",
              end: "2026-09-01T00:00:00Z",
              status: "disbursing",
              disbursed_count: 412,
              amount_micros: 50_000_000,
              expires_at: "2026-09-01T00:00:00Z",
            },
            recipients: [
              {
                recipient_user_id: "owner-1",
                recipient_billing_enabled: true,
              },
            ],
          },
        ],
      }).success,
    ).toBe(true);

    const grantList = creditGrantListSchema.parse({
      grants: [
        {
          id: "grant-1",
          batch_id: "schedule-1:1785542400000",
          schedule_id: "schedule-1",
          period_start: "2026-08-01T00:00:00Z",
          recipient_user_id: "owner-1",
          activation_state: "active",
          target_kind: "selected_users",
          amount_credits: 50,
          amount_micros: 50_000_000,
          remaining_micros: 50_000_000,
          reserved_micros: 0,
          scope: {
            all_services: true,
            service_ids: [],
            service_slugs: [],
          },
          granted_by: "schedule-1",
          status: "active",
          created_at: "2026-08-01T00:00:00Z",
          updated_at: "2026-08-01T00:00:00Z",
        },
      ],
      page: 1,
      per_page: 1,
      total: 1,
    });
    expect(grantList.grants[0]?.schedule_id).toBe("schedule-1");
  });
});
