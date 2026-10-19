import { z } from "zod";
const actorSchema = z.object({
  kind: z.string(),
  id: z.string(),
  name: z.string(),
  person_id: z.string().nullable(),
  api_key_id: z.string().nullable(),
  app_id: z.string().nullable(),
});
const summarySchema = z.object({
  actor: actorSchema,
  at: z.string(),
  action: z.string(),
  action_label: z.string().optional(),
  change_group_id: z.string(),
});
export const authorshipSchema = z.object({
  created_by: summarySchema.nullable(),
  last_change: summarySchema.nullable(),
});
export type ServiceAuthorship = z.infer<typeof authorshipSchema>;
export const historyResponseSchema = z.object({
  service_id: z.string(),
  next_cursor: z.string().nullable(),
  tracked_since: z.string().nullable(),
  legacy: z.boolean(),
  deleted: z.boolean(),
  groups: z.array(
    z.object({
      id: z.string(),
      events: z.array(
        z.object({
          id: z.string(),
          schema_version: z.number().int(),
          action: z.string(),
          action_label: z.string().optional(),
          actor: actorSchema,
          committed_at: z.string(),
          additional_changes: z.boolean(),
          audit_status: z.enum(["pending", "published", "mismatch"]),
          changes: z.array(
            z.object({
              field: z.string(),
              label: z.string().optional(),
              before: z.unknown().nullable(),
              after: z.unknown().nullable(),
            }),
          ),
        }),
      ),
    }),
  ),
});
export const archiveResponseSchema = z.object({
  services: z.array(
    z.object({
      service_id: z.string(),
      service_slug: z.string(),
      last_changed_at: z.string(),
    }),
  ),
  next_cursor: z.string().nullable(),
});
