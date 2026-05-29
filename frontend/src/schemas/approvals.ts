import { z } from "zod";

export const approvalModeSchema = z.enum(["per_request", "grant"]);
export const approvalEffectSchema = z.enum([
  "require_approval",
  "auto_allow",
  "deny",
]);
export const approvalVerbSchema = z.enum(["read", "write", "destructive"]);

export const approvalRuleSchema = z.object({
  methods: z
    .array(
      z.enum([
        "*",
        "GET",
        "POST",
        "PUT",
        "PATCH",
        "DELETE",
        "HEAD",
        "OPTIONS",
        "EXEC",
        "TUNNEL",
      ]),
    )
    .max(16, "A rule can include at most 16 methods")
    .default([]),
  resource_pattern: z
    .string()
    .trim()
    .min(1, "Resource pattern is required")
    .max(256, "Resource pattern must be 256 characters or less")
    .default("*"),
  verbs: z.array(approvalVerbSchema).default([]),
  effect: approvalEffectSchema.default("require_approval"),
  mode: approvalModeSchema.default("per_request"),
});

export const setServiceApprovalConfigSchema = z
  .object({
    approval_required: z.boolean().optional(),
    approval_mode: approvalModeSchema.optional(),
    rules: z.array(approvalRuleSchema).max(50).optional(),
    default_effect: approvalEffectSchema.optional(),
  })
  .refine(
    (value) =>
      value.approval_required !== undefined ||
      value.approval_mode !== undefined ||
      value.rules !== undefined ||
      value.default_effect !== undefined,
    "At least one approval config field is required",
  );

export const updateNotificationSettingsSchema = z.object({
  telegram_enabled: z.boolean(),
  push_enabled: z.boolean(),
  approval_required: z.boolean(),
  approval_timeout_secs: z
    .number()
    .int()
    .min(10, "Minimum timeout is 10 seconds")
    .max(300, "Maximum timeout is 300 seconds"),
  grant_expiry_days: z
    .number()
    .int()
    .min(1, "Minimum expiry is 1 day")
    .max(365, "Maximum expiry is 365 days"),
});

export type UpdateNotificationSettingsFormData = z.infer<
  typeof updateNotificationSettingsSchema
>;
export type SetServiceApprovalConfigFormData = z.infer<
  typeof setServiceApprovalConfigSchema
>;
