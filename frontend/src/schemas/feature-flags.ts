import { z } from "zod";

// ─────────────────────────────────────────────────────────────────────────────
// Per-org feature flags (admin management surface)
// Mirrors backend `handlers/org_feature_flags.rs` wire types.
// ─────────────────────────────────────────────────────────────────────────────

/** Scope an override applies to. Mirrors `FlagTargetKind` on the backend. */
export const flagTargetKindSchema = z.enum(["org", "role", "user"]);
export type FlagTargetKind = z.infer<typeof flagTargetKindSchema>;

export const featureFlagOverrideSchema = z.object({
  target_kind: flagTargetKindSchema,
  /** null for org scope; role string for role scope; member_user_id for user scope. */
  target_value: z.string().nullable(),
  enabled: z.boolean(),
  updated_at: z.string(),
  updated_by: z.string(),
});
export type FeatureFlagOverrideItem = z.infer<typeof featureFlagOverrideSchema>;

export const featureFlagSchema = z.object({
  key: z.string(),
  description: z.string(),
  default_enabled: z.boolean(),
  org_manageable: z.boolean(),
  overrides: z.array(featureFlagOverrideSchema),
});
export type FeatureFlagItem = z.infer<typeof featureFlagSchema>;

export const featureFlagListResponseSchema = z.object({
  flags: z.array(featureFlagSchema),
});
export type FeatureFlagListResponse = z.infer<
  typeof featureFlagListResponseSchema
>;

/** Body for `PUT /orgs/{id}/feature-flags/{flag_key}`. */
export const setFeatureFlagRequestSchema = z.object({
  target_kind: flagTargetKindSchema,
  /** Required for role/user scope; omit for org scope. */
  target_value: z.string().nullable().optional(),
  enabled: z.boolean(),
});
export type SetFeatureFlagRequest = z.infer<typeof setFeatureFlagRequestSchema>;
