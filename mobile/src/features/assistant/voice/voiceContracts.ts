import { z } from "zod";

const providerId = z
  .string()
  .min(1)
  .max(128)
  .regex(/^[A-Za-z0-9_.-]+$/);
const providerLabel = z
  .string()
  .trim()
  .min(1)
  .max(128)
  .refine((value) => !/\p{Cc}/u.test(value), "Invalid provider label");
export const voiceProtocolSchema = z.enum(["openai_live", "xai_realtime"]);
export const voiceBillingMetricSchema = z.enum([
  "voice_seconds",
  "tokens",
  "input_tokens",
  "output_tokens",
  "cache_read_tokens",
  "cache_write_tokens",
]);

/** Provider discovery describes a transport, not a credential or entitlement. */
export const voiceMetadataSchema = z
  .object({
    protocol: voiceProtocolSchema,
    models: z
      .array(
        z.object({
          id: providerId,
          label: providerLabel,
          default: z.boolean().optional(),
        }),
      )
      .min(1)
      .max(32),
    voices: z
      .array(z.object({ id: providerId, label: providerLabel }))
      .min(1)
      .max(64),
    usage_source: z.enum(["provider_reported", "server_measured"]),
    billing_metrics: z.array(voiceBillingMetricSchema).min(1).max(6),
  })
  .superRefine((value, context) => {
    for (const field of ["models", "voices"] as const) {
      if (
        new Set(value[field].map((item) => item.id)).size !==
        value[field].length
      )
        context.addIssue({
          code: "custom",
          path: [field],
          message: "Provider IDs must be unique",
        });
    }
    if (value.models.filter((model) => model.default).length > 1)
      context.addIssue({
        code: "custom",
        path: ["models"],
        message: "Voice source has multiple defaults",
      });
    if (
      new Set(value.billing_metrics).size !== value.billing_metrics.length ||
      !value.billing_metrics.includes("voice_seconds")
    )
      context.addIssue({
        code: "custom",
        path: ["billing_metrics"],
        message: "Unique metrics including voice seconds are required",
      });
    if (
      value.usage_source !==
      (value.protocol === "openai_live"
        ? "provider_reported"
        : "server_measured")
    )
      context.addIssue({
        code: "custom",
        path: ["usage_source"],
        message: "Usage source must match the provider protocol",
      });
  });

export const voicePreferencesSchema = z
  .object({
    service_id: z.string().uuid(),
    connection_id: z.string().uuid().nullable(),
    key_source: z.enum(["platform", "own"]),
    model: providerId,
    voice: providerId.nullable(),
    input_mode: z.enum(["push_to_talk", "automatic"]),
    language: z.string().min(1).max(35).nullable(),
    notify_on_completion: z.boolean(),
  })
  .strict()
  .refine(
    (value) => (value.key_source === "own") === (value.connection_id !== null),
    {
      path: ["connection_id"],
      message: "An own-key voice source requires its connection ID",
    },
  );

export const voiceSettingsSchema = z.object({
  voice: voicePreferencesSchema.nullish(),
});
export const voiceIdentitySchema = z.object({
  id: z.string().uuid(),
  capabilities: z
    .object({ enabled_features: z.array(z.string()).default([]) })
    .nullish(),
});

const voicePriceSchema = z.object({
  metric: z.string(),
  credits_per_unit: z.string(),
  sync_status: z.string(),
});
const voiceTariffSchema = voicePriceSchema.extend({
  components: z.array(voicePriceSchema),
});
export const voiceOptionSchema = z.object({
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
  reported_token_pricing: voiceTariffSchema.nullish(),
  pricing: voiceTariffSchema.nullable(),
});
export const voiceOptionsSchema = z.object({
  options: z.array(voiceOptionSchema),
  limits: z
    .object({
      call_seconds: z.number().int().positive(),
      idle_seconds: z.number().int().positive(),
      idle_warning_seconds: z.number().int().nonnegative(),
      transmitting_calls: z.number().int().positive(),
      queued_requests: z.number().int().nonnegative(),
    })
    .optional(),
  recording: z.literal(false).optional(),
});

export const voiceSessionSchema = z.object({
  created_at: z.string().datetime({ offset: true }),
  closed_at: z.string().datetime({ offset: true }).nullable(),
  id: z.string().uuid(),
  generation: z.number().int().nonnegative(),
  state: z.enum(["starting", "active", "closing", "closed", "failed"]),
  muted: z.boolean(),
  input_muted: z.boolean(),
  end_requested: z.boolean(),
  control_revision: z.number().int().nonnegative(),
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
export const voiceTaskSchema = z.object({
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
  title: z.string().max(120).nullable().optional(),
  result_message_id: z.string().nullable().optional(),
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
  tasks: z.array(voiceTaskSchema),
});
export const voiceStartedSchema = z.object({
  session: voiceSessionSchema,
  sdp_answer: z.string(),
});
export const voiceStartRequestSchema = z
  .object({
    client_request_id: z.string().uuid(),
    preferences: voicePreferencesSchema,
    sdp_offer: z.string().max(64 * 1024),
  })
  .strict();
export const voiceControlRequestSchema = z
  .object({
    command_id: z.string().uuid(),
    expected_revision: z.number().int().nonnegative(),
    action: z.enum(["mute", "unmute", "end"]),
  })
  .strict();
export const voiceRequestSchema = z.object({
  pending_acknowledgement_ids: z.array(z.string()),
  id: z.string().uuid(),
  state: voiceTaskSchema.shape.state,
  turn_id: z.string(),
});

const diagnosticIdentifier = z.string().regex(/^[A-Za-z0-9_.[\]]{1,128}$/);
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
  provider_type: diagnosticIdentifier.optional(),
  provider_code: diagnosticIdentifier.optional(),
  provider_param: diagnosticIdentifier.optional(),
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

export type VoiceProtocol = z.infer<typeof voiceProtocolSchema>;
export type VoicePreferences = z.infer<typeof voicePreferencesSchema>;
export type VoiceSettings = z.infer<typeof voiceSettingsSchema>;
export type VoiceIdentity = z.infer<typeof voiceIdentitySchema>;
export type VoiceOption = z.infer<typeof voiceOptionSchema>;
export type VoiceOptions = z.infer<typeof voiceOptionsSchema>;
export type VoiceSession = z.infer<typeof voiceSessionSchema>;
export type VoiceSnapshot = z.infer<typeof voiceSnapshotSchema>;
export type VoiceTask = z.infer<typeof voiceTaskSchema>;
export type VoiceStartRequest = z.infer<typeof voiceStartRequestSchema>;
export type VoiceControlRequest = z.infer<typeof voiceControlRequestSchema>;
