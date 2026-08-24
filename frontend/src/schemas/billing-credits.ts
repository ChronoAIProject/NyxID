import { z } from "zod";
import { billingMetricSchema } from "@/schemas/billing";

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

export const billingServiceScopeSchema = z.object({
  all_services: z.boolean(),
  service_ids: z.array(z.string()),
  service_slugs: z.array(z.string()),
});

export const creditGrantSchema = z.object({
  id: z.string(),
  batch_id: z.string(),
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

export const issueGrantFormSchema = z
  .object({
    amount_credits: z.number().int().min(1).max(1_000_000),
    target_kind: billingTargetKindSchema,
    target_user_ids: z.array(z.string()).max(500),
    all_services: z.boolean(),
    service_refs: z.array(z.string()).max(100),
    expires_at: z.string(),
    reason: z.string().trim().max(2_000),
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
    if (!value.all_services && value.service_refs.length === 0) {
      ctx.addIssue({
        code: "custom",
        path: ["service_refs"],
        message: "Select at least one service",
      });
    }
    if (value.expires_at) {
      const expiry = new Date(value.expires_at).getTime();
      if (!Number.isFinite(expiry)) {
        ctx.addIssue({
          code: "custom",
          path: ["expires_at"],
          message: "Enter a valid expiry",
        });
      } else if (expiry <= Date.now()) {
        ctx.addIssue({
          code: "custom",
          path: ["expires_at"],
          message: "Expiry must be in the future",
        });
      }
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

export type CreditGrant = z.infer<typeof creditGrantSchema>;
export type AdminCreditGrant = z.infer<typeof adminCreditGrantSchema>;
export type CreditGrantList = z.infer<typeof creditGrantListSchema>;
export type IssueGrantResponse = z.infer<typeof issueGrantResponseSchema>;
export type IssueGrantForm = z.infer<typeof issueGrantFormSchema>;
export type UsageAllowance = z.infer<typeof usageAllowanceSchema>;
export type AllowanceForm = z.infer<typeof allowanceFormSchema>;
export type UserAllowanceBalance = z.infer<typeof userAllowanceBalanceSchema>;
