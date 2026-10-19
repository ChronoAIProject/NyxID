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

const uuid = z.uuid();
export const curationGrantSchema = z.object({
  service_ids: z.string().refine((value) => {
    const ids = value.split(/[,\s]+/).filter(Boolean);
    return ids.length >= 1 && ids.length <= 100 && new Set(ids).size === ids.length && ids.every((id) => uuid.safeParse(id).success);
  }, "Enter 1–100 distinct catalog service UUIDs"),
  ornn_proxy_service_id: z.string().refine((value) => value === "" || uuid.safeParse(value).success, "Enter a catalog service UUID"),
  expires_at: z.string().refine((value) => value === "" || (!Number.isNaN(Date.parse(value)) && Date.parse(value) > Date.now()), "Choose a future expiry"),
  max_writes: z.string().regex(/^[1-9]\d*$/, "Enter a whole number").refine((value) => Number(value) <= 10_000, "At most 10,000 writes"),
  window_seconds: z.string().regex(/^\d+$/, "Enter a whole number").refine((value) => Number(value) >= 60 && Number(value) <= 86_400, "Use 60–86,400 seconds"),
});
export type CurationGrantFormData = z.infer<typeof curationGrantSchema>;
