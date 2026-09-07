import { z } from "zod";
import { previewResponseSchema, userCodeSchema } from "./auth-device";
import {
  API_KEY_SCOPES,
  createApiKeySchema,
  type CreateApiKeyFormData,
} from "./api-keys";

export const resourceSummarySchema = z.object({
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
  allowed_services: z.array(resourceSummarySchema),
  allowed_nodes: z.array(resourceSummarySchema),
  expires_at: z.string().nullable(),
  rate_limit_per_second: z.number().nullable(),
  rate_limit_burst: z.number().nullable(),
  platform: z.string().nullable(),
  created_now: z.boolean(),
});
export const agentKeyOptionsSchema = z.object({
  keys: z.array(agentKeySummarySchema),
  services: z.array(resourceSummarySchema),
  nodes: z.array(resourceSummarySchema),
  orgs: z.array(resourceSummarySchema),
});
export const agentKeyPreviewSchema = previewResponseSchema.extend({
  requested_profile: z.string().max(64).nullable(),
  interval: z.number().int().positive(),
});
const newKeySelectionSchema = createApiKeySchema
  .pick({
    name: true,
    expires_at: true,
    rate_limit_per_second: true,
    rate_limit_burst: true,
    platform: true,
    target_org_id: true,
  })
  .extend({
    kind: z.literal("new"),
    scopes: z
      .string()
      .refine(
        (value) =>
          value.trim().length > 0 &&
          value
            .split(/\s+/)
            .every((scope) =>
              (API_KEY_SCOPES as readonly string[]).includes(scope),
            ),
        "Invalid scopes",
      ),
    allowed_service_ids: z.array(z.string()).default([]),
    allowed_node_ids: z.array(z.string()).default([]),
    allow_all_services: z.boolean().default(false),
    allow_all_nodes: z.boolean().default(false),
    scope_plan_digest: z.string().optional(),
  });
export const agentKeyApproveSchema = z
  .object({
    user_code: userCodeSchema,
    selection: z.discriminatedUnion("kind", [
      z.object({ kind: z.literal("existing"), api_key_id: z.string().min(1) }),
      newKeySelectionSchema,
    ]),
    credential_expires_at: createApiKeySchema.shape.expires_at,
  })
  .superRefine(({ selection }, ctx) => {
    if (selection.kind !== "new") return;
    for (const kind of ["services", "nodes"] as const) {
      const ids =
        kind === "services"
          ? selection.allowed_service_ids
          : selection.allowed_node_ids;
      if (selection[`allow_all_${kind}`] && ids.length)
        ctx.addIssue({
          code: "custom",
          path: ["selection", `allow_all_${kind}`],
          message: "Allow all cannot be combined with a restricted list",
        });
    }
  });
export const loginCredentialSchema = z.object({
  id: z.string(),
  label: z.string(),
  secret_prefix: z.string(),
  is_active: z.boolean(),
  revoked_reason: z
    .enum([
      "logout",
      "web_revoke",
      "parent_revoked",
      "parent_rotated",
      "undelivered_expired",
    ])
    .nullable(),
  revoked_at: z.string().nullable(),
  expires_at: z.string().nullable(),
  last_used_at: z.string().nullable(),
  created_at: z.string(),
});
export const loginCredentialsSchema = z.object({
  credentials: z.array(loginCredentialSchema),
});
export type AgentKeySummary = z.infer<typeof agentKeySummarySchema>;
export type AgentKeyOptions = z.infer<typeof agentKeyOptionsSchema>;
export type AgentKeyPreview = z.infer<typeof agentKeyPreviewSchema>;
export type AgentKeyApprove = z.infer<typeof agentKeyApproveSchema>;
export type LoginCredential = z.infer<typeof loginCredentialSchema>;

export function newKeySelection(form: CreateApiKeyFormData) {
  return newKeySelectionSchema.safeParse({
    kind: "new",
    name: form.name,
    scopes: form.scopes.join(" "),
    expires_at: form.expires_at || null,
    allowed_service_ids: form.allow_all_services
      ? []
      : (form.allowed_service_ids ?? []),
    allowed_node_ids: form.allow_all_nodes ? [] : (form.allowed_node_ids ?? []),
    allow_all_services: form.allow_all_services ?? false,
    allow_all_nodes: form.allow_all_nodes ?? false,
    target_org_id: form.target_org_id,
    rate_limit_per_second: form.rate_limit_per_second,
    rate_limit_burst: form.rate_limit_burst,
    platform: form.platform || null,
  });
}

export function agentKeyErrorMessage(error: unknown): string {
  const code = (error as { errorCode?: number } | null)?.errorCode;
  const messages: Record<number, string> = {
    11900: "This login request is no longer available.",
    11901: "This login request has expired.",
    11902: "Waiting for approval.",
    11903: "Please wait before trying again.",
    11904: "This login request was rejected.",
    11905: "This login request was already approved.",
    11906: "Too many attempts. Try again in a few minutes.",
    11907: "That code is not valid. Check the code in your terminal.",
    11908: "This key is no longer eligible. Choose another key.",
    11909: "This login credential is no longer available.",
  };
  return (
    (code === undefined ? undefined : messages[code]) ??
    (error instanceof Error
      ? error.message
      : "Could not reach NyxID. Try again.")
  );
}

export function effectivePermissions(scopes: string): string {
  const set = new Set(scopes.split(/\s+/));
  return [
    "read",
    set.has("write") || set.has("admin") ? "write" : null,
    set.has("proxy") || set.has("proxy:*") ? "proxy" : null,
  ]
    .filter(Boolean)
    .join(", ");
}
