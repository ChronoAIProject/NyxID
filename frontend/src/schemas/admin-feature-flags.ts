import { z } from "zod";

export const adminFeatureFlagTargetKindSchema = z.enum([
  "global",
  "org",
  "user",
]);
export type AdminFeatureFlagTargetKind = z.infer<
  typeof adminFeatureFlagTargetKindSchema
>;

export const adminUserFeatureFlagOverrideSchema = z.object({
  user_id: z.string(),
  user_email: z.string().nullable(),
  user_display_name: z.string().nullable(),
  enabled: z.boolean(),
  updated_at: z.string(),
  updated_by: z.string(),
});

export const adminOrgFeatureFlagOverrideSchema = z.object({
  org_user_id: z.string(),
  org_display_name: z.string().nullable(),
  org_slug: z.string().nullable(),
  enabled: z.boolean(),
  updated_at: z.string(),
  updated_by: z.string(),
});
export type AdminOrgFeatureFlagOverride = z.infer<
  typeof adminOrgFeatureFlagOverrideSchema
>;

export const adminFeatureFlagSchema = z.object({
  key: z.string(),
  description: z.string(),
  default_enabled: z.boolean(),
  org_manageable: z.boolean(),
  global_override: z.boolean().nullable(),
  org_overrides: z.array(adminOrgFeatureFlagOverrideSchema),
  user_overrides: z.array(adminUserFeatureFlagOverrideSchema),
});
export type AdminFeatureFlag = z.infer<typeof adminFeatureFlagSchema>;

export const adminFeatureFlagListResponseSchema = z.object({
  flags: z.array(adminFeatureFlagSchema),
});
export type AdminFeatureFlagListResponse = z.infer<
  typeof adminFeatureFlagListResponseSchema
>;

export const setAdminFeatureFlagRequestSchema = z.object({
  target_kind: adminFeatureFlagTargetKindSchema,
  target_key: z.string().nullable(),
  enabled: z.boolean(),
});
export type SetAdminFeatureFlagRequest = z.infer<
  typeof setAdminFeatureFlagRequestSchema
>;
