import { z } from "zod";

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
  allowed_scopes: z
    .string()
    .min(1, "At least one scope is required"),
  // role_ids and rate_limit_override are strings in the form because HTML
  // inputs produce string values; they are parsed in the submit handler.
  role_ids: z
    .string()
    .optional()
    .or(z.literal("")),
  rate_limit_override: z
    .string()
    .optional()
    .or(z.literal("")),
});

export type CreateServiceAccountFormData = z.infer<typeof createServiceAccountSchema>;

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
  allowed_scopes: z
    .string()
    .min(1, "At least one scope is required"),
  role_ids: z
    .string()
    .optional()
    .or(z.literal("")),
  rate_limit_override: z
    .string()
    .optional()
    .or(z.literal("")),
  is_active: z.boolean().optional(),
});

export type UpdateServiceAccountFormData = z.infer<typeof updateServiceAccountSchema>;
