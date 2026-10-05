import { z } from "zod";
import { voiceMetadataSchema } from "./platform-keys";
import { voicePreferencesSchema } from "./assistant-nyxagent";

export const voiceSessionSchema = z.object({
  created_at: z.string().datetime(),
  closed_at: z.string().datetime().nullable(),
  id: z.string().uuid(),
  generation: z.number().int(),
  state: z.enum(["starting", "active", "closing", "closed", "failed"]),
  muted: z.boolean(),
  input_muted: z.boolean(),
  end_requested: z.boolean(),
  control_revision: z.number().int(),
  observed_seconds: z.number().nonnegative(),
  reserved_until: z.number().nonnegative(),
  final_usage_confirmed: z.boolean(),
  end_reason: z.string().nullable(),
  idle_warning: z.boolean(),
});
export const voiceSnapshotSchema = z.object({
  type: z.literal("snapshot"),
  session: voiceSessionSchema,
  captions: z.array(
    z.object({
      id: z.string(),
      speaker: z.enum(["user", "assistant"]),
      text: z.string(),
      sealed: z.boolean(),
      complete: z.boolean(),
    }),
  ),
  tasks: z.array(
    z.object({
      id: z.string().uuid(),
      turn_id: z.string(),
      state: z.enum([
        "queued",
        "claimed",
        "awaiting_confirmation",
        "completed",
        "cancelled",
      ]),
      pending_acknowledgement_ids: z.array(z.string()),
    }),
  ),
});
export const voiceStartedSchema = z.object({
  session: voiceSessionSchema,
  sdp_answer: z.string(),
});
export type VoicePreferences = z.infer<typeof voicePreferencesSchema>;
export type VoiceSession = z.infer<typeof voiceSessionSchema>;
export type VoiceSnapshot = z.infer<typeof voiceSnapshotSchema>;
const priceSchema = z.object({
  metric: z.string(),
  credits_per_unit: z.string(),
  sync_status: z.string(),
});
export const voiceOptionsSchema = z.object({
  options: z.array(
    z.object({
      service_id: z.string().uuid(),
      connection_id: z.string().uuid().nullable(),
      key_source: z.enum(["platform", "own"]),
      model: z.string(),
      model_label: z.string(),
      default_model: z.boolean(),
      voice: voiceMetadataSchema,
      available: z.boolean(),
      unavailable_reason: z.string().nullable(),
      billing_owner: z.string(),
      reported_token_pricing: priceSchema
        .extend({ components: z.array(priceSchema) })
        .nullable()
        .optional(),
      pricing: priceSchema
        .extend({ components: z.array(priceSchema) })
        .nullable(),
    }),
  ),
});
export type VoiceOption = z.infer<typeof voiceOptionsSchema>["options"][number];

const identifier = z.string().regex(/^[A-Za-z0-9_.[\]]{1,128}$/);
export const voiceStartDetailsSchema = z.object({
  stage: z.enum([
    "flag",
    "thread",
    "origin",
    "credential",
    "billing_reservation",
    "provider_create",
    "provider_answer",
    "transport",
  ]),
  reason: z.string().max(400),
  provider: z.enum(["openai", "xai"]).optional(),
  provider_status: z.number().int().min(100).max(599).optional(),
  provider_type: identifier.optional(),
  provider_code: identifier.optional(),
  provider_param: identifier.optional(),
});
export const voiceStartFailureSchema = z.object({
  type: z.literal("start_failed"),
  error: z.object({
    error: z.string(),
    error_code: z.number().int(),
    message: z.string().max(256),
    details: voiceStartDetailsSchema,
  }),
});
