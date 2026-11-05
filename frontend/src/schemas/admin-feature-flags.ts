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

export const MAX_FEATURE_FLAG_GROUP_LENGTH = 80;
export const MAX_FEATURE_FLAG_TAG_LENGTH = 40;
export const MAX_FEATURE_FLAG_TAGS = 12;

export const featureFlagTagsSchema = z
  .array(
    z
      .string()
      .trim()
      .min(1)
      .max(MAX_FEATURE_FLAG_TAG_LENGTH)
      .regex(
        /^[a-z0-9][a-z0-9_-]*$/,
        "Use lowercase letters, digits, hyphens or underscores.",
      ),
  )
  .max(MAX_FEATURE_FLAG_TAGS);

export const adminFeatureFlagSchema = z.object({
  key: z.string(),
  /** Effective description: the admin-authored one when set, else the code one. */
  description: z.string(),
  /** The code-declared description — the reset target for the editor. */
  code_description: z.string(),
  /** The admin-authored description, when one exists. */
  custom_description: z.string().nullable(),
  owner: z.string().nullable(),
  group: z.string().nullable().optional(),
  tags: z.array(z.string()).optional(),
  sunset: z.boolean().optional(),
  metadata_updated_at: z.string().nullable(),
  metadata_updated_by: z.string().nullable(),
  default_enabled: z.boolean(),
  created_by: z.string().nullable().optional(),
  can_edit: z.boolean().optional(),
  global_override: z.boolean().nullable(),
  org_overrides: z.array(adminOrgFeatureFlagOverrideSchema),
  user_overrides: z.array(adminUserFeatureFlagOverrideSchema),
});
export type AdminFeatureFlag = z.infer<typeof adminFeatureFlagSchema>;

/**
 * Sparse PATCH: omission preserves, null/blank clears.
 */
export const updateAdminFeatureFlagMetadataRequestSchema = z
  .object({
    description: z
      .string()
      .trim()
      .max(MAX_FEATURE_FLAG_DESCRIPTION_LENGTH)
      .nullable(),
    owner: z.string().trim().max(MAX_FEATURE_FLAG_OWNER_LENGTH).nullable(),
    group: z.string().trim().max(MAX_FEATURE_FLAG_GROUP_LENGTH).nullable(),
    tags: featureFlagTagsSchema,
    expected_tags: featureFlagTagsSchema,
    sunset: z.boolean(),
  })
  .partial()
  .strict();
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

export const createFeatureFlagRequestSchema = z.object({
  key: z
    .string()
    .min(1)
    .max(128)
    .regex(
      /^[a-z0-9][a-z0-9:_-]*$/,
      "Use lowercase letters, digits, colons, underscores or hyphens.",
    ),
  description: z
    .string()
    .trim()
    .min(1)
    .max(MAX_FEATURE_FLAG_DESCRIPTION_LENGTH),
  default_enabled: z.boolean(),
});
export type CreateFeatureFlagRequest = z.infer<
  typeof createFeatureFlagRequestSchema
>;

export interface FeatureFlagTarget {
  id: string;
  email: string;
  display_name: string | null;
  slug: string | null;
}

export const featureFlagHistoryEntrySchema = z.object({
  id: z.string(),
  actor_id: z.string().nullable(),
  actor_name: z.string().nullable(),
  actor_email: z.string().nullable(),
  target_name: z.string().nullable().optional(),
  event_type: z.string(),
  details: z.record(z.string(), z.unknown()),
  created_at: z.string(),
  legacy: z.boolean().optional(),
});
export type FeatureFlagHistoryEntry = z.infer<
  typeof featureFlagHistoryEntrySchema
>;
export interface FeatureFlagHistoryResponse {
  entries: FeatureFlagHistoryEntry[];
  next_cursor: string | null;
  legacy_ambiguous_omitted: boolean;
  origin: {
    created_by: string | null;
    creator_name: string | null;
    created_at: string | null;
  };
}
