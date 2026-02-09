import { z } from "zod";

export const API_KEY_SCOPES = [
  "read:profile",
  "write:profile",
  "read:services",
  "write:services",
  "read:connections",
  "write:connections",
  "admin",
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
