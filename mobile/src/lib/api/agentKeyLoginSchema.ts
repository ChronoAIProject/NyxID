import { z } from "zod";
import { authDevicePreviewSchema } from "./authDeviceSchema";
import { normalizeAuthDeviceUserCode } from "../../features/auth/deviceUserCode";
import { normalizeAgentKeyExpiry } from "../../features/auth/agentKeyExpiry";

export const agentKeyScopes = [
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
const resourceSchema = z.object({
  id: z.string(),
  name: z.string(),
  owner_id: z.string(),
});
export const agentKeySummarySchema = z.object({
  id: z.string(),
  name: z.string(),
  key_prefix: z.string(),
  owner_type: z.enum(["personal", "org"]),
  owner_id: z.string(),
  owner_name: z.string(),
  scopes: z.string(),
  allow_all_services: z.boolean(),
  allow_all_nodes: z.boolean(),
  allowed_service_ids: z.array(z.string()),
  allowed_node_ids: z.array(z.string()),
  allowed_services: z.array(resourceSchema),
  allowed_nodes: z.array(resourceSchema),
  expires_at: z.string().nullable(),
  rate_limit_per_second: z.number().nullable(),
  rate_limit_burst: z.number().nullable(),
  platform: z.string().nullable(),
  created_now: z.boolean(),
});
export const agentKeyOptionsSchema = z.object({
  keys: z.array(agentKeySummarySchema),
  services: z.array(resourceSchema),
  nodes: z.array(resourceSchema),
  orgs: z.array(resourceSchema),
});
export const agentKeyPreviewSchema = authDevicePreviewSchema.extend({
  requested_profile: z.string().max(64).nullable(),
  interval: z.number().int().positive(),
});
export const futureExpirySchema = z
  .string()
  .nullable()
  .optional()
  .transform((value, ctx) => {
    if (value == null) return value;
    try {
      return normalizeAgentKeyExpiry(value);
    } catch (error) {
      ctx.addIssue({ code: "custom", message: (error as Error).message });
      return z.NEVER;
    }
  })
  .refine(
    (value) =>
      !value ||
      (Number.isFinite(Date.parse(value)) && Date.parse(value) > Date.now()),
    "Expiry must be a future date",
  );
export const newAgentKeySchema = z.object({
  kind: z.literal("new"),
  name: z.string().trim().min(1).max(64),
  scopes: z
    .string()
    .refine(
      (value) =>
        value.trim().length > 0 &&
        value
          .split(/\s+/)
          .every((scope) =>
            (agentKeyScopes as readonly string[]).includes(scope),
          ),
      "Select valid scopes",
    ),
  allowed_service_ids: z.array(z.string()).default([]),
  allowed_node_ids: z.array(z.string()).default([]),
  allow_all_services: z.boolean().default(false),
  allow_all_nodes: z.boolean().default(false),
  expires_at: futureExpirySchema,
  target_org_id: z.string().optional(),
  platform: z.string().max(64).nullable().optional(),
  rate_limit_per_second: z.number().int().positive().max(4294967295).optional(),
  rate_limit_burst: z.number().int().positive().max(4294967295).optional(),
  scope_plan_digest: z.string().optional(),
});
export const agentKeyApproveSchema = z
  .object({
    user_code: z.string().transform((value, ctx) => {
      const code = normalizeAuthDeviceUserCode(value);
      if (!code) {
        ctx.addIssue({
          code: "custom",
          message: "Enter a valid eight-character code",
        });
        return z.NEVER;
      }
      return code;
    }),
    selection: z.discriminatedUnion("kind", [
      z.object({ kind: z.literal("existing"), api_key_id: z.string().min(1) }),
      newAgentKeySchema,
    ]),
    credential_expires_at: futureExpirySchema,
  })
  .superRefine(({ selection }, ctx) => {
    if (selection.kind !== "new") return;
    if (
      (selection.allow_all_services && selection.allowed_service_ids.length) ||
      (selection.allow_all_nodes && selection.allowed_node_ids.length)
    )
      ctx.addIssue({
        code: "custom",
        message: "Allow all cannot be combined with a restricted list",
      });
  });
export type AgentKeySummary = z.infer<typeof agentKeySummarySchema>;
export type AgentKeyOptions = z.infer<typeof agentKeyOptionsSchema>;
export type AgentKeyPreview = z.infer<typeof agentKeyPreviewSchema>;
export type AgentKeyApprove = z.infer<typeof agentKeyApproveSchema>;
export type NewAgentKey = z.infer<typeof newAgentKeySchema>;
