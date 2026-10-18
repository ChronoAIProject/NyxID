import { z } from "zod";
import { billingMetricSchema } from "@/schemas/billing";

const microcreditStringSchema = z
  .string()
  .regex(/^\d+$/, "Invalid microcredit amount");

export const billingTargetKindSchema = z.enum(["all_users", "selected_users"]);
export const creditGrantStatusSchema = z.enum([
  "active",
  "consumed",
  "expired",
  "revoked",
]);
export const creditGrantActivationStateSchema = z.enum([
  "active",
  "pending_activation",
]);
export const allowanceRecurrenceSchema = z.enum([
  "one_time",
  "daily",
  "weekly",
  "monthly",
]);
export const scheduleRecurrenceSchema = z.enum(["daily", "weekly", "monthly"]);
export const creditExpiryPolicySchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("end_of_period") }),
  z.object({
    kind: z.literal("after_days"),
    days: z.number().int().min(1).max(3_650),
  }),
  z.object({ kind: z.literal("never") }),
]);

export const billingServiceScopeSchema = z.object({
  all_services: z.boolean(),
  service_ids: z.array(z.string()),
  service_slugs: z.array(z.string()),
});

export const serviceLimitSchema = z.object({
  key: z.string(), service_ids: z.array(z.string()), service_slugs: z.array(z.string()),
  label: z.string().nullable(), limit_micros: microcreditStringSchema,
  consumed_micros: microcreditStringSchema, reserved_micros: microcreditStringSchema,
});

const serviceLimitFormSchema = z.object({
  service_refs: z.array(z.string()).min(1, "Select at least one service").max(100),
  amount_credits: z.number().int().min(1).max(1_000_000),
  label: z.string().trim().max(120),
});

function validateServiceLimits(value: {
  amount_credits: number; all_services: boolean; service_refs: string[];
  service_limits?: z.infer<typeof serviceLimitFormSchema>[];
}, ctx: z.RefinementCtx) {
  const used = new Set<string>();
  for (const [index, rule] of (value.service_limits ?? []).entries()) {
    if (rule.amount_credits > value.amount_credits) ctx.addIssue({ code: "custom", path: ["service_limits", index, "amount_credits"], message: "Limit must not exceed the shared total" });
    for (const id of rule.service_refs) {
      if (used.has(id)) ctx.addIssue({ code: "custom", path: ["service_limits", index, "service_refs"], message: "A service can belong to only one limit" });
      if (!value.all_services && !value.service_refs.includes(id)) ctx.addIssue({ code: "custom", path: ["service_limits", index, "service_refs"], message: "Choose services included in the eligibility scope" });
      used.add(id);
    }
  }
}

export const creditGrantSchema = z.object({
  id: z.string(),
  batch_id: z.string(),
  schedule_id: z.string().nullable().optional(),
  period_start: z.string().nullable().optional(),
  recipient_user_id: z.string(),
  recipient_email: z.string().nullable().optional(),
  recipient_display_name: z.string().nullable().optional(),
  recipient_billing_enabled: z.boolean().optional(),
  activation_state: creditGrantActivationStateSchema,
  target_kind: billingTargetKindSchema,
  amount_credits: z.number().int().positive(),
  amount_micros: z.number().int().nonnegative(),
  remaining_micros: z.number().int().nonnegative(),
  reserved_micros: z.number().int().nonnegative(),
  allocated_micros_exact: microcreditStringSchema.optional(),
  remaining_micros_exact: microcreditStringSchema.optional(),
  reserved_micros_exact: microcreditStringSchema.optional(),
  used_micros: microcreditStringSchema.optional(),
  available_micros: microcreditStringSchema.optional(),
  reclaimed_micros: microcreditStringSchema.optional(),
  revoke_requested_at: z.string().nullable().optional(),
  service_limits: z.array(serviceLimitSchema).optional(),
  scope: billingServiceScopeSchema,
  expires_at: z.string().nullable().optional(),
  reason: z.string().nullable().optional(),
  granted_by: z.string(),
  status: creditGrantStatusSchema,
  created_at: z.string(),
  updated_at: z.string(),
  consumed_at: z.string().nullable().optional(),
  expired_at: z.string().nullable().optional(),
  revoked_at: z.string().nullable().optional(),
});

export const creditGrantListSchema = z.object({
  grants: z.array(creditGrantSchema),
  page: z.number().int().positive(),
  per_page: z.number().int().nonnegative(),
  total: z.number().int().nonnegative(),
});

export const adminCreditGrantSchema = creditGrantSchema.extend({
  recipient_billing_enabled: z.boolean(),
});

export const adminCreditGrantListSchema = creditGrantListSchema.extend({
  grants: z.array(adminCreditGrantSchema),
});

export const creditSchedulePeriodSchema = z.object({
  start: z.string(),
  end: z.string(),
  status: z.enum(["disbursing", "complete"]),
  disbursed_count: z.number().int().nonnegative(),
  amount_micros: z.number().int().nonnegative(),
  expires_at: z.string().nullable().optional(),
  completed_at: z.string().nullable().optional(),
});

export const creditScheduleRecipientSchema = z.object({
  recipient_user_id: z.string(),
  recipient_email: z.string().nullable().optional(),
  recipient_display_name: z.string().nullable().optional(),
  recipient_billing_enabled: z.boolean(),
});

export const creditScheduleSchema = z.object({
  id: z.string(),
  amount_credits: z.number().int().positive(),
  amount_micros: z.number().int().nonnegative(),
  recurrence: scheduleRecurrenceSchema,
  expiry: creditExpiryPolicySchema,
  target_kind: billingTargetKindSchema,
  target_user_ids: z.array(z.string()),
  scope: billingServiceScopeSchema,
  service_limits: z.array(serviceLimitSchema).optional(),
  reason: z.string().nullable().optional(),
  is_active: z.boolean(),
  created_by: z.string(),
  created_at: z.string(),
  updated_at: z.string(),
  last_period_start: z.string().nullable().optional(),
  last_disbursed_at: z.string().nullable().optional(),
  skipped_periods: z.number().int().nonnegative(),
  current_period: creditSchedulePeriodSchema.nullable().optional(),
  recipients: z.array(creditScheduleRecipientSchema).optional(),
});

export const creditScheduleListSchema = z.object({
  schedules: z.array(creditScheduleSchema),
});

const scheduleFormExpiryPolicySchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("end_of_period") }),
  z.object({
    kind: z.literal("after_days"),
    days: z.union([z.number(), z.nan()]),
  }),
  z.object({ kind: z.literal("never") }),
]);

export const scheduleFormSchema = z
  .object({
    amount_credits: z.number().int().min(1).max(1_000_000),
    recurrence: scheduleRecurrenceSchema,
    expiry: scheduleFormExpiryPolicySchema,
    target_kind: billingTargetKindSchema,
    target_user_ids: z.array(z.string()).max(500),
    all_services: z.boolean(),
    service_refs: z.array(z.string()).max(100),
    service_limits: z.array(serviceLimitFormSchema).max(20).optional(),
    reason: z.string().trim().max(2_000),
  })
  .superRefine((value, ctx) => {
    validateServiceLimits(value, ctx);
    if (
      value.expiry.kind === "after_days" &&
      (!Number.isInteger(value.expiry.days) ||
        value.expiry.days < 1 ||
        value.expiry.days > 3_650)
    ) {
      ctx.addIssue({
        code: "custom",
        path: ["expiry", "days"],
        message: "Enter 1 to 3,650 days",
      });
    }
    if (
      value.target_kind === "selected_users" &&
      value.target_user_ids.length === 0
    ) {
      ctx.addIssue({
        code: "custom",
        path: ["target_user_ids"],
        message: "Select at least one owner",
      });
    }
    if (value.target_kind === "all_users" && value.target_user_ids.length > 0) {
      ctx.addIssue({
        code: "custom",
        path: ["target_user_ids"],
        message: "All-owner schedules cannot include selected owners",
      });
    }
    if (!value.all_services && value.service_refs.length === 0) {
      ctx.addIssue({
        code: "custom",
        path: ["service_refs"],
        message: "Select at least one service",
      });
    }
    if (value.all_services && value.service_refs.length > 0) {
      ctx.addIssue({
        code: "custom",
        path: ["service_refs"],
        message: "All-service schedules cannot include selected services",
      });
    }
  });

export const issueGrantResponseSchema = z.object({
  batch_id: z.string(),
  created_count: z.number().int().positive(),
  activated_count: z.number().int().nonnegative(),
  pending_activation_count: z.number().int().nonnegative(),
  recipients: z.array(
    z.object({
      recipient_user_id: z.string(),
      recipient_billing_enabled: z.boolean(),
      activation_state: creditGrantActivationStateSchema,
    }),
  ),
});

export const addCreditsRecurrenceSchema = z.enum([
  "one_time",
  "daily",
  "weekly",
  "monthly",
]);

export const addCreditsFormSchema = z
  .object({
    amount_credits: z.number().int().min(1).max(1_000_000),
    recurrence: addCreditsRecurrenceSchema,
    target_kind: billingTargetKindSchema,
    target_user_ids: z.array(z.string()).max(500),
    all_services: z.boolean(),
    service_refs: z.array(z.string()).max(100),
    service_limits: z.array(serviceLimitFormSchema).max(20).optional(),
    // This input is unmounted for recurring issuance; default it so a mode
    // switch cannot leave a hidden validation error behind.
    expires_at: z.string().optional(),
    expiry: scheduleFormExpiryPolicySchema,
    reason: z.string().trim().max(2_000),
  })
  .superRefine((value, ctx) => {
    validateServiceLimits(value, ctx);
    if (
      value.recurrence !== "one_time" &&
      value.expiry.kind === "after_days" &&
      (!Number.isInteger(value.expiry.days) ||
        value.expiry.days < 1 ||
        value.expiry.days > 3_650)
    ) {
      ctx.addIssue({
        code: "custom",
        path: ["expiry", "days"],
        message: "Enter 1 to 3,650 days",
      });
    }
    if (
      value.target_kind === "selected_users" &&
      value.target_user_ids.length === 0
    ) {
      ctx.addIssue({
        code: "custom",
        path: ["target_user_ids"],
        message: "Select at least one owner",
      });
    }
    if (value.target_kind === "all_users" && value.target_user_ids.length > 0) {
      ctx.addIssue({
        code: "custom",
        path: ["target_user_ids"],
        message: "All-owner issuance cannot include selected owners",
      });
    }
    if (!value.all_services && value.service_refs.length === 0) {
      ctx.addIssue({
        code: "custom",
        path: ["service_refs"],
        message: "Select at least one service",
      });
    }
    if (value.all_services && value.service_refs.length > 0) {
      ctx.addIssue({
        code: "custom",
        path: ["service_refs"],
        message: "All-service issuance cannot include selected services",
      });
    }
    if (value.recurrence === "one_time" && value.expires_at) {
      const expiry = new Date(value.expires_at).getTime();
      if (!Number.isFinite(expiry) || expiry <= Date.now()) {
        ctx.addIssue({
          code: "custom",
          path: ["expires_at"],
          message: "Expiry must be a future date",
        });
      }
    }
  });

export const adminCreditSummarySchema = z.object({
  outstanding_micros: microcreditStringSchema,
  available_micros: microcreditStringSchema,
  unavailable_micros: microcreditStringSchema.optional(),
  reserved_micros: microcreditStringSchema,
  pending_activation_micros: microcreditStringSchema,
  expiring_within_30_days_micros: microcreditStringSchema,
  restricted_micros: microcreditStringSchema,
  expired_awaiting_cleanup_micros: microcreditStringSchema,
  active_grant_count: z.number().int().nonnegative(),
  pending_activation_count: z.number().int().nonnegative(),
  recipient_count: z.number().int().nonnegative(),
  generated_at: z.string(),
});

export const userCreditSummarySchema = z.object({
  owner_id: z.string().min(1),
  available_micros: microcreditStringSchema,
  wallet_available_micros: z
    .string()
    .regex(/^-?\d+$/, "Invalid signed microcredit amount"),
  issued_available_micros: microcreditStringSchema,
  issued_reserved_micros: microcreditStringSchema,
  pending_activation_micros: microcreditStringSchema,
  restricted_micros: microcreditStringSchema,
  restricted_grant_count: z.number().int().nonnegative(),
  wallet: z
    .object({
      plan_kind: z.enum(["prepaid", "subscription", "hybrid"]),
      collection_state: z.enum(["good", "past_due", "suspended"]),
      balance_credits: z.string().regex(/^-?\d+$/),
      reserved_credits: z.string().regex(/^-?\d+$/),
      pending_lago_debits: z.string().regex(/^-?\d+$/),
      pending_topup_expiry_credits: z.string().regex(/^\d+$/),
      available_credits: z.string().regex(/^-?\d+$/),
      spendable_credits: z.string().regex(/^\d+$/),
      overdraft_cap_credits: z.string().regex(/^\d+$/),
      has_payment_instrument: z.boolean(),
      suspended: z.boolean(),
      balance_synced_at: z.string(),
    })
    .nullable(),
  grants: z.array(
    z.object({
      id: z.string(),
      allocated_micros: microcreditStringSchema.optional(), used_micros: microcreditStringSchema.optional(),
      reclaimed_micros: microcreditStringSchema.optional(), status: creditGrantStatusSchema.optional(),
      revoke_requested_at: z.string().nullable().optional(), created_at: z.string().optional(),
      schedule_id: z.string().nullable().optional(), period_start: z.string().nullable().optional(),
      service_limits: z.array(serviceLimitSchema).optional(),
      remaining_micros: microcreditStringSchema,
      reserved_micros: microcreditStringSchema,
      available_micros: microcreditStringSchema,
      scope: billingServiceScopeSchema,
      expires_at: z.string().nullable().optional(),
    }),
  ),
  allowances: z.array(
    z.object({
      id: z.string(),
      service_id: z.string(),
      service_slug: z.string(),
      metric: billingMetricSchema,
      quantity: z.number().int().positive(),
      recurrence: allowanceRecurrenceSchema,
      period_start: z.string(),
      period_end: z.string().nullable().optional(),
      consumed_quantity: z.number().int().nonnegative(),
      reserved_quantity: z.number().int().nonnegative(),
      remaining_quantity: z.number().int().nonnegative(),
    }),
  ),
  generated_at: z.string(),
});

export const usageAllowanceSchema = z.object({
  id: z.string(),
  service_id: z.string(),
  service_slug: z.string(),
  metric: billingMetricSchema,
  quantity: z.number().int().positive(),
  recurrence: allowanceRecurrenceSchema,
  target_kind: billingTargetKindSchema,
  target_user_ids: z.array(z.string()),
  is_active: z.boolean(),
  created_by: z.string(),
  created_at: z.string(),
  updated_at: z.string(),
});

export const usageAllowanceListSchema = z.object({
  allowances: z.array(usageAllowanceSchema),
});

export const allowanceFormSchema = z
  .object({
    service_ref: z.string().min(1, "Select a service"),
    quantity: z.number().int().min(1).max(1_000_000_000_000),
    recurrence: allowanceRecurrenceSchema,
    target_kind: billingTargetKindSchema,
    target_user_ids: z.array(z.string()).max(500),
  })
  .superRefine((value, ctx) => {
    if (
      value.target_kind === "selected_users" &&
      value.target_user_ids.length === 0
    ) {
      ctx.addIssue({
        code: "custom",
        path: ["target_user_ids"],
        message: "Select at least one user",
      });
    }
  });

export type CreditGrant = z.infer<typeof creditGrantSchema>;
export type AdminCreditGrant = z.infer<typeof adminCreditGrantSchema>;
export type CreditGrantList = z.infer<typeof creditGrantListSchema>;
export type IssueGrantResponse = z.infer<typeof issueGrantResponseSchema>;
export type AddCreditsForm = z.infer<typeof addCreditsFormSchema>;
export type AdminCreditSummary = z.infer<typeof adminCreditSummarySchema>;
export type UserCreditSummary = z.infer<typeof userCreditSummarySchema>;
export type CreditExpiryPolicy = z.infer<typeof creditExpiryPolicySchema>;
export type CreditSchedule = z.infer<typeof creditScheduleSchema>;
export type ScheduleForm = z.infer<typeof scheduleFormSchema>;
export type UsageAllowance = z.infer<typeof usageAllowanceSchema>;
export type AllowanceForm = z.infer<typeof allowanceFormSchema>;

export type ServiceLimit = z.infer<typeof serviceLimitSchema>;

export const userAllowanceBalanceSchema = z.object({
  allowance: usageAllowanceSchema,
  period_start: z.string(),
  period_end: z.string().nullable().optional(),
  consumed_quantity: z.number().int().nonnegative(),
  reserved_quantity: z.number().int().nonnegative(),
  remaining_quantity: z.number().int().nonnegative(),
});

export const userAllowanceListSchema = z.object({
  allowances: z.array(userAllowanceBalanceSchema),
});


export type UserAllowanceBalance = z.infer<typeof userAllowanceBalanceSchema>;
