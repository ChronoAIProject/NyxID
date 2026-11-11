import { z } from "zod";
import { isValidHttpUrl } from "./http-url";

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
    .max(64, "Name must be at most 64 characters")
    .refine((v) => v.trim().length > 0, "Name must not be blank"),
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
  never_expires: z.boolean().optional(),
  description: z.string().nullable().optional(),
  allow_all_services: z.boolean().optional(),
  allow_auto_connected_services: z.boolean().optional(),
  allow_all_nodes: z.boolean().optional(),
  allowed_service_ids: z.array(z.string()).optional(),
  allowed_node_ids: z.array(z.string()).optional(),
  callback_url: z
    .string()
    .refine(isValidHttpUrl, "Must be a valid URL")
    .nullable()
    .optional(),
  platform: z.string().nullable().optional(),
  rate_limit_per_second: z.number().int().positive().max(4294967295).optional(),
  rate_limit_burst: z.number().int().positive().max(4294967295).optional(),
  /**
   * When set, the key is created under the given org and managed by every
   * admin of that org. Omit for a personal key. The backend enforces that
   * the caller is an admin of the target org.
   */
  target_org_id: z.string().optional(),
}).refine((value) => !(value.never_expires && value.expires_at), {
  path: ["expires_at"],
  message: "Choose either an expiry date or no expiry",
});

export type CreateApiKeyFormData = z.infer<typeof createApiKeySchema>;

/** A new form intent; persisted keys and rotation never use these defaults. */
export function newApiKeyDefaults(now = Date.now()): CreateApiKeyFormData {
  return {
    name: "", scopes: [],
    expires_at: new Date(now + 90 * 86400000).toISOString().slice(0, 10),
    never_expires: false, description: null,
    allow_all_services: false, allow_auto_connected_services: false, allow_all_nodes: false,
    allowed_service_ids: [], allowed_node_ids: [], callback_url: null,
    target_org_id: undefined,
  };
}
