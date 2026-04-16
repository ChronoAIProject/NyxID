import { z } from "zod";

/**
 * Valid API key scopes -- must match backend VALID_API_KEY_SCOPES
 * in services/key_service.rs.
 */
export const API_KEY_SCOPES = [
  "read",
  "write",
  "admin",
  "openid",
  "profile",
  "email",
  "services:read",
  "services:write",
  "proxy",
] as const;

export type ApiKeyScope = (typeof API_KEY_SCOPES)[number];

export const createApiKeySchema = z.object({
  name: z
    .string()
    .min(1, "Name is required")
    .max(64, "Name must be at most 64 characters"),
  scopes: z
    .array(z.enum(API_KEY_SCOPES))
    .min(1, "At least one scope is required"),
  expires_at: z
    .string()
    .nullable()
    .optional()
    .refine(
      (value) => {
        if (value === null || value === undefined || value === "") return true;
        // Backend treats date-only (YYYY-MM-DD) as 23:59:59 UTC.
        const dateOnlyMatch = /^\d{4}-\d{2}-\d{2}$/.test(value);
        const parsed = dateOnlyMatch
          ? new Date(`${value}T23:59:59Z`)
          : new Date(value);
        if (Number.isNaN(parsed.getTime())) return false;
        return parsed.getTime() > Date.now();
      },
      { message: "Expiry date must be in the future" },
    ),
  description: z.string().nullable().optional(),
  allow_all_services: z.boolean().optional(),
  allow_all_nodes: z.boolean().optional(),
  allowed_service_ids: z.array(z.string()).optional(),
  allowed_node_ids: z.array(z.string()).optional(),
  callback_url: z
    .string()
    .url("Must be a valid URL")
    .nullable()
    .optional(),
});

export type CreateApiKeyFormData = z.infer<typeof createApiKeySchema>;
