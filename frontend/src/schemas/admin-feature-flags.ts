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
  org_id: z.string(),
  org_display_name: z.string().nullable(),
  org_slug: z.string().nullable(),
  enabled: z.boolean(),
  updated_at: z.string(),
  updated_by: z.string(),
});
export type AdminOrgFeatureFlagOverride = z.infer<
  typeof adminOrgFeatureFlagOverrideSchema
>;

/** Mirrors `feature_flag_service::MAX_FLAG_DESCRIPTION_LEN`. */
export const MAX_FEATURE_FLAG_DESCRIPTION_LENGTH = 512;
/** Mirrors `feature_flag_service::MAX_FLAG_OWNER_LEN`. */
export const MAX_FEATURE_FLAG_OWNER_LENGTH = 128;

export const adminFeatureFlagSchema = z.object({
  key: z.string(),
  /** Effective description: the admin-authored one when set, else the code one. */
  description: z.string(),
  /** The code-declared description — the reset target for the editor. */
  code_description: z.string(),
  /** The admin-authored description, when one exists. */
  custom_description: z.string().nullable(),
  owner: z.string().nullable(),
  metadata_updated_at: z.string().nullable(),
  metadata_updated_by: z.string().nullable(),
  default_enabled: z.boolean(),
  global_override: z.boolean().nullable(),
  org_overrides: z.array(adminOrgFeatureFlagOverrideSchema),
  user_overrides: z.array(adminUserFeatureFlagOverrideSchema),
});
export type AdminFeatureFlag = z.infer<typeof adminFeatureFlagSchema>;

/**
 * Full replace: a blank or omitted field clears that side of the metadata, and
 * clearing both restores the code-declared description with no owner.
 */
export const updateAdminFeatureFlagMetadataRequestSchema = z.object({
  description: z
    .string()
    .trim()
    .max(MAX_FEATURE_FLAG_DESCRIPTION_LENGTH)
    .nullable(),
  owner: z.string().trim().max(MAX_FEATURE_FLAG_OWNER_LENGTH).nullable(),
});
export type UpdateAdminFeatureFlagMetadataRequest = z.infer<
  typeof updateAdminFeatureFlagMetadataRequestSchema
>;

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
