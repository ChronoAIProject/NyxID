import { describe, expect, it } from "vitest";
import {
  allowanceFormSchema,
  addCreditsFormSchema,
  adminCreditGrantListSchema,
  creditScheduleListSchema,
  scheduleFormSchema,
  userCreditSummarySchema,
} from "./billing-credits";

describe("billing credit schemas", () => {
  it("accepts signed wallet diagnostics while keeping spendable totals nonnegative", () => {
    const response = {
      owner_id: "owner-1",
      available_micros: "10000000",
      wallet_available_micros: "-4000000",
      issued_available_micros: "10000000",
      issued_reserved_micros: "0",
      pending_activation_micros: "0",
      restricted_micros: "0",
      restricted_grant_count: 0,
      generated_at: "2026-10-17T00:00:00Z",
      wallet: null,
      grants: [],
      allowances: [],
    };
    expect(
      userCreditSummarySchema.parse(response).wallet_available_micros,
    ).toBe("-4000000");
    for (const field of [
      "available_micros",
      "issued_available_micros",
      "issued_reserved_micros",
      "pending_activation_micros",
      "restricted_micros",
    ]) {
      expect(
        userCreditSummarySchema.safeParse({ ...response, [field]: "-1" })
          .success,
      ).toBe(false);
    }
    for (const invalid of ["-1.5", "+4", "1e6", "NaN"]) {
      expect(
        userCreditSummarySchema.safeParse({
          ...response,
          wallet_available_micros: invalid,
        }).success,
      ).toBe(false);
    }
  });

  it("validates one-time Add credits targets, service scope, bounds, and expiry", () => {
    const valid = {
      amount_credits: 100,
      recurrence: "one_time" as const,
      expiry: { kind: "end_of_period" as const },
      target_kind: "selected_users" as const,
      target_user_ids: ["user-1"],
      all_services: false,
      service_refs: ["service-1"],
      expires_at: "2099-01-01T00:00",
      reason: "Launch credits",
    };

    expect(addCreditsFormSchema.safeParse(valid).success).toBe(true);
    expect(
      addCreditsFormSchema.safeParse({ ...valid, target_user_ids: [] }).success,
    ).toBe(false);
    expect(
      addCreditsFormSchema.safeParse({ ...valid, service_refs: [] }).success,
    ).toBe(false);
    for (const amount_credits of [0, 1.5, 1_000_001]) {
      expect(
        addCreditsFormSchema.safeParse({ ...valid, amount_credits }).success,
      ).toBe(false);
    }
    expect(
      addCreditsFormSchema.safeParse({ ...valid, expires_at: "not-a-date" })
        .success,
    ).toBe(false);
    expect(
      addCreditsFormSchema.safeParse({
        ...valid,
        expires_at: "2000-01-01T00:00",
      }).success,
    ).toBe(false);
    expect(
      addCreditsFormSchema.safeParse({ ...valid, expires_at: "" }).success,
    ).toBe(true);
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

  it("parses administrator history and included usage in the combined summary", () => {
    expect(
      adminCreditGrantListSchema.safeParse({
        grants: [
          {
            id: "grant-1",
            batch_id: "batch-1",
            recipient_user_id: "user-1",
            recipient_billing_enabled: true,
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

    const summary = userCreditSummarySchema.parse({
      owner_id: "user-1",
      available_micros: "86500000",
      wallet_available_micros: "85000000",
      issued_available_micros: "1500000",
      issued_reserved_micros: "0",
      pending_activation_micros: "0",
      restricted_micros: "0",
      restricted_grant_count: 0,
      generated_at: "2026-10-17T00:00:00Z",
      wallet: {
        plan_kind: "prepaid",
        collection_state: "good",
        balance_credits: "100",
        reserved_credits: "10",
        pending_lago_debits: "5",
        pending_topup_expiry_credits: "0",
        available_credits: "85",
        spendable_credits: "85",
        overdraft_cap_credits: "0",
        has_payment_instrument: false,
        suspended: false,
        balance_synced_at: "2026-10-17T00:00:00Z",
      },
      grants: [
        {
          id: "grant-1",
          remaining_micros: "1500000",
          reserved_micros: "0",
          available_micros: "1500000",
          scope: { all_services: true, service_ids: [], service_slugs: [] },
          expires_at: null,
        },
      ],
      allowances: [
        {
          id: "allowance-1",
          service_id: "service-1",
          service_slug: "llm-one",
          metric: "tokens",
          quantity: 1000,
          recurrence: "daily",
          period_start: "2026-10-17T00:00:00Z",
          period_end: "2026-10-18T00:00:00Z",
          consumed_quantity: 250,
          reserved_quantity: 50,
          remaining_quantity: 700,
        },
      ],
    });
    expect(summary.available_micros).toBe("86500000");
    expect(summary.wallet?.available_credits).toBe("85");
    expect(summary.grants[0]?.available_micros).toBe("1500000");
    expect(summary.allowances[0]?.remaining_quantity).toBe(700);
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

  it("supports one-time and recurring mode transitions without hidden expiry errors", () => {
    const base = {
      amount_credits: 25,
      recurrence: "one_time" as const,
      target_kind: "all_users" as const,
      target_user_ids: [],
      all_services: true,
      service_refs: [],
      reason: "",
      expiry: { kind: "end_of_period" as const },
    };
    expect(addCreditsFormSchema.safeParse(base).success).toBe(true);
    expect(
      addCreditsFormSchema.safeParse({
        ...base,
        recurrence: "monthly",
        expiry: { kind: "after_days", days: 45 },
      }).success,
    ).toBe(true);
    expect(
      addCreditsFormSchema.safeParse({
        ...base,
        recurrence: "monthly",
        expiry: { kind: "after_days", days: 0 },
      }).success,
    ).toBe(false);
    expect(
      addCreditsFormSchema.safeParse({
        ...base,
        recurrence: "monthly",
        expiry: { kind: "after_days", days: 45.5 },
      }).success,
    ).toBe(false);
    expect(
      addCreditsFormSchema.safeParse({
        ...base,
        all_services: true,
        service_refs: ["service-1"],
      }).success,
    ).toBe(false);
    for (const days of [Number.NaN, 0, 1.5, 3_651]) {
      const result = addCreditsFormSchema.safeParse({
        ...base,
        recurrence: "monthly",
        expiry: { kind: "after_days", days },
      });
      expect(result.success).toBe(false);
      if (!result.success) {
        expect(
          result.error.issues.some(
            (issue) => issue.path.join(".") === "expiry.days",
          ),
        ).toBe(true);
      }
    }
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

    const grantList = adminCreditGrantListSchema.parse({
      grants: [
        {
          id: "grant-1",
          batch_id: "schedule-1:1785542400000",
          schedule_id: "schedule-1",
          period_start: "2026-08-01T00:00:00Z",
          recipient_user_id: "owner-1",
          recipient_billing_enabled: true,
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
