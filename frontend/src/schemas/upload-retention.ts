import { z } from "zod";

export const uploadRetentionPolicySchema = z.object({
  pending_hours: z.number().int().min(1).max(8760),
  image_days: z.number().int().min(1).max(365),
  images_delete_after_turn: z.boolean(),
  document_days: z.number().int().min(1).max(365),
  tool_image_days: z.number().int().min(1).max(365).nullable(),
});
export type UploadRetentionPolicy = z.infer<typeof uploadRetentionPolicySchema>;
export const uploadRetentionResponseSchema = z.object({
  effective: uploadRetentionPolicySchema,
  defaults: uploadRetentionPolicySchema,
  overridden: z.boolean(),
  revision: z.number().int(),
  updated_at: z.string().nullable(),
  replica_revision: z.number().int(),
  replica_effective: uploadRetentionPolicySchema,
  replica_refreshed_at: z.string().nullable(),
  refresh_seconds: z.number(),
});
export type UploadRetentionResponse = z.infer<
  typeof uploadRetentionResponseSchema
>;
