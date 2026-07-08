import { z } from "zod";

// Backend requires a positive integer when set (service_account_service.rs);
// anything unparseable would otherwise become NaN and silently drop the
// admin's override at submit time.
const rateLimitOverrideField = z
  .string()
  .refine(
    (v) => v.trim() === "" || /^[1-9]\d*$/.test(v.trim()),
    "Must be a whole number of at least 1 (or empty for no override)",
  )
  .optional()
  .or(z.literal(""));

export const createServiceAccountSchema = z.object({
  name: z
    .string()
    .min(1, "Name is required")
    .max(100, "Name must be 100 characters or less"),
  description: z
    .string()
    .max(500, "Description must be 500 characters or less")
    .optional()
    .or(z.literal("")),
  allowed_scopes: z.string().min(1, "At least one scope is required"),
  // role_ids and rate_limit_override are strings in the form because HTML
  // inputs produce string values; they are parsed in the submit handler.
  role_ids: z.string().optional().or(z.literal("")),
  rate_limit_override: rateLimitOverrideField,
});

export type CreateServiceAccountFormData = z.infer<
  typeof createServiceAccountSchema
>;

export const updateServiceAccountSchema = z.object({
  name: z
    .string()
    .min(1, "Name is required")
    .max(100, "Name must be 100 characters or less"),
  description: z
    .string()
    .max(500, "Description must be 500 characters or less")
    .optional()
    .or(z.literal("")),
  allowed_scopes: z.string().min(1, "At least one scope is required"),
  role_ids: z.string().optional().or(z.literal("")),
  rate_limit_override: rateLimitOverrideField,
  is_active: z.boolean().optional(),
});

export type UpdateServiceAccountFormData = z.infer<
  typeof updateServiceAccountSchema
>;
