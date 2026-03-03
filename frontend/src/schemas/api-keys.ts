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
  expires_at: z.string().nullable().optional(),
});

export type CreateApiKeyFormData = z.infer<typeof createApiKeySchema>;
