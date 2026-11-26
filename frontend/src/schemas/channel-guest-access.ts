import { z } from "zod";

const actionSchema = z.enum(["create", "read", "update", "delete"]);
const audienceSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("all_guests") }),
  z.object({ kind: z.literal("guest"), guest_id: z.string().max(128) }),
  z.object({
    kind: z.literal("verified_audience"),
    audience_id: z.string().max(256),
    guest_ids: z.array(z.string().max(128)).max(64),
    room_id: z.string().max(128).optional(),
  }),
]);
const admissionSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("owner_only") }),
  z.object({ kind: z.literal("channel_policy") }),
  z.object({ kind: z.literal("selected"), guest_ids: z.array(z.string().max(128)).max(32) }),
]);
const fieldsSchema = z.object({ read: z.array(z.string()), write: z.array(z.string()) });
const ruleSchema = z.object({
  source_id: z.string(),
  collection_id: z.string(),
  audience: audienceSchema,
  scope: z.discriminatedUnion("kind", [
    z.object({ kind: z.literal("own_records") }),
    z.object({ kind: z.literal("resources"), record_ids: z.array(z.string().max(448)).max(64) }),
  ]),
  delivery: z.enum(["private", "shared"]),
  operations: z.partialRecord(actionSchema, fieldsSchema),
});
const declarationSchema = z.object({
  version: z.literal(1),
  skill_id: z.string().min(1).max(128),
  skill_revision: z.number().int().nonnegative(),
});
export const configurationSchema = z.object({
  audience: admissionSchema.nullable(),
  rules: z.array(ruleSchema).max(32),
  declarations: z.array(declarationSchema).max(8),
});
export const guestSetupFormSchema = z.object({ configuration: configurationSchema });

const sourceSchema = z.object({
  id: z.string(),
  collection_id: z.string(),
  actions: z.array(actionSchema),
  fields: z.array(z.string()),
});
const observationSchema = z.object({ id: z.string(), platform: z.string(), last_seen_at: z.string() });
const settingsSchema = z.object({
  revision: z.number().int().nonnegative(),
  policy_revision: z.number().int().nonnegative(),
  prepared_draft_revision: z.number().int().nonnegative().nullish(),
  draft: configurationSchema.nullable(),
  applied: configurationSchema.nullable(),
  effective_rule_count: z.number().int().nonnegative(),
  verified_guests: z.array(observationSchema),
  verified_rooms: z.array(observationSchema),
  sheets_connections: z.array(z.object({ id: z.string(), label: z.string(), actions: z.array(actionSchema) })),
  sources: z.array(sourceSchema),
});
export const guestSetupResponseSchema = z.object({
  available: z.boolean(),
  enabled: z.boolean(),
  bot_revision: z.number().int().nonnegative(),
  activation_issue: z.literal("prepared_revision_stale").optional(),
  settings: settingsSchema.optional(),
});
export const guestPreviewSchema = z.object({
  guests_admitted: z.tuple([z.boolean(), z.boolean()]),
  rule_count: z.number().int().nonnegative(),
  checks: z.array(z.object({
    rule: z.number().int().positive(),
    source_id: z.string(),
    action: actionSchema,
    guest_a: z.boolean(),
    guest_b: z.boolean(),
    guest_b_on_a: z.boolean().nullable(),
    guest_a_in_group: z.boolean(),
    resource: z.string().nullable(),
    read_fields: z.array(z.string()),
    write_fields: z.array(z.string()),
  })),
});

export type GuestAction = z.infer<typeof actionSchema>;
export type GuestDataRule = z.infer<typeof ruleSchema>;
export type GuestSetupConfiguration = z.infer<typeof configurationSchema>;
export type GuestSetupSettings = z.infer<typeof settingsSchema>;
export type GuestSetupSource = z.infer<typeof sourceSchema>;
export type GuestSetupPreview = z.infer<typeof guestPreviewSchema>;
export type GuestObservation = z.infer<typeof observationSchema>;
