import { z } from "zod";
import { voiceMetadataSchema } from "./platform-keys";
import { voicePreferencesSchema } from "./assistant-nyxagent";

export const voicePhaseTimingSchema = z.object({
  unit: z.literal("ms"),
  offset_origin: z.literal("call_created_at"),
  measurements: z
    .array(
      z.object({
        phase: z.enum([
          "input_start",
          "input_end",
          "transcript_observed",
          "delegation",
          "queued",
          "first_activity",
          "tool_start",
          "tool_end",
          "first_progress",
          "approval_wait",
          "result",
          "playback_confirmed",
          "model_start",
          "model_end",
          "model",
          "tool",
          "total",
          "output_start",
          "output_end",
          "response_start",
          "response_end",
          "response_generation",
          "first_audio",
          "mcp_round_trip",
          "transcription_turnaround",
        ]),
        source: z.enum([
          "coordinator",
          "provider_transcript",
          "relay_audio",
          "native_responses",
          "nyxagent",
          "mcp_client",
          "client_playback",
        ]),
        kind: z.enum(["offset", "duration"]),
        value_ms: z.number().int().min(0).max(86_400_000),
        tool_index: z.number().int().min(0).max(39).optional(),
      }),
    )
    .max(128),
  unavailable: z
    .array(
      z.enum([
        "asr_processing",
        "synthesis_processing",
        "device_playback",
        "model",
        "tool",
        "first_audio",
        "client_playback",
      ]),
    )
    .max(7),
});
export type VoicePhaseTiming = z.infer<typeof voicePhaseTimingSchema>;
const voiceTurnTimingSchema = z.object({
  turn_id: z.string().uuid(),
  related_turn_id: z.string().uuid().optional(),
  kind: z.enum([
    "input",
    "output",
    "native_response",
    "direct_tool",
    "voice_response",
  ]),
  phase_timing: voicePhaseTimingSchema,
});

export const voiceSessionSchema = z.object({
  client_request_id: z.string().optional(),
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
  resumable: z.boolean().default(true),
});
export const activeVoiceSessionSchema = z.object({
  session: voiceSessionSchema.nullable(),
});
const strictVoiceSnapshotSchema = z.object({
  type: z.literal("snapshot"),
  session: voiceSessionSchema,
  captions: z.array(
    z.object({
      id: z.string(),
      speaker: z.enum(["user", "assistant"]),
      text: z.string(),
      sealed: z.boolean(),
      complete: z.boolean(),
      interrupted: z.boolean().optional(),
      start_ms: z.number().nonnegative().nullable().optional(),
      end_ms: z.number().nonnegative().nullable().optional(),
      fragments: z
        .array(
          z.object({
            text: z.string(),
            start_ms: z.number().nonnegative(),
            end_ms: z.number().nonnegative(),
          }),
        )
        .nullable()
        .optional(),
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
      outcome: z.enum(["failed", "cancelled"]).nullable().optional(),
      pending_acknowledgement_ids: z.array(z.string()),
      title: z.string().max(120).nullable().optional(),
      result_message_id: z.string().nullable().optional(),
      phase_timing: voicePhaseTimingSchema.nullable().catch(null).optional(),
      progress: z
        .object({
          stage: z.string(),
          detail: z.string(),
          last_completed_step: z.string().nullable(),
          completed_steps: z.number().int().nonnegative(),
          updated_at: z.string().datetime(),
        })
        .nullable()
        .optional(),
    }),
  ),
});
// Presentation rows may evolve independently of the call-control protocol.
// Validate the session strictly; omit an incompatible caption/task rather than
// converting a display-only schema change into a lost audio call.
export const voiceSnapshotSchema = strictVoiceSnapshotSchema.extend({
  turn_timings: z
    .array(z.unknown())
    .catch([])
    .transform((rows) =>
      rows.slice(0, 32).flatMap((row) => {
        const result = voiceTurnTimingSchema.safeParse(row);
        return result.success ? [result.data] : [];
      }),
    )
    .optional(),
  captions: z
    .array(z.unknown())
    .catch([])
    .transform((rows) =>
      rows.flatMap((row) => {
        const result =
          strictVoiceSnapshotSchema.shape.captions.element.safeParse(row);
        return result.success ? [result.data] : [];
      }),
    ),
  tasks: z
    .array(z.unknown())
    .catch([])
    .transform((rows) =>
      rows.flatMap((row) => {
        const result =
          strictVoiceSnapshotSchema.shape.tasks.element.safeParse(row);
        return result.success ? [result.data] : [];
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
