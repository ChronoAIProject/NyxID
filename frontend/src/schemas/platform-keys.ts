import { z } from "zod";
import {
  BILLING_METRICS,
  metricLabel,
  unitPriceSchema,
} from "./billing-metrics";

export const VOICE_BILLING_METRICS = [
  "voice_seconds",
  "tokens",
  "input_tokens",
  "output_tokens",
  "cache_read_tokens",
  "cache_write_tokens",
] as const;
const voiceId = z
  .string()
  .min(1)
  .max(128)
  .regex(/^[A-Za-z0-9_.-]+$/, "Use a provider identifier, not a URL");
const voiceLabel = z
  .string()
  .trim()
  .min(1)
  .max(128)
  .refine((v) => !/\p{Cc}/u.test(v), "Control characters are not allowed");
export const voiceMetadataSchema = z
  .object({
    protocol: z.enum(["openai_live", "xai_realtime"]),
    models: z
      .array(
        z.object({
          id: voiceId,
          label: voiceLabel,
          default: z.boolean().optional(),
        }),
      )
      .min(1)
      .max(32),
    voices: z
      .array(z.object({ id: voiceId, label: voiceLabel }))
      .min(1)
      .max(64),
    usage_source: z.enum(["provider_reported", "server_measured"]),
    billing_metrics: z.array(z.enum(VOICE_BILLING_METRICS)).min(1).max(6),
  })
  .superRefine((voice, ctx) => {
    for (const field of ["models", "voices"] as const) {
      if (new Set(voice[field].map((v) => v.id)).size !== voice[field].length)
        ctx.addIssue({
          code: "custom",
          path: [field],
          message: "Provider IDs must be unique",
        });
    }
    if (voice.models.filter((m) => m.default).length > 1)
      ctx.addIssue({
        code: "custom",
        path: ["models"],
        message: "Choose at most one default model",
      });
    if (
      new Set(voice.billing_metrics).size !== voice.billing_metrics.length ||
      !voice.billing_metrics.includes("voice_seconds")
    )
      ctx.addIssue({
        code: "custom",
        path: ["billing_metrics"],
        message: "Unique metrics including voice seconds are required",
      });
    if (
      voice.usage_source !==
      (voice.protocol === "openai_live"
        ? "provider_reported"
        : "server_measured")
    )
      ctx.addIssue({
        code: "custom",
        path: ["usage_source"],
        message: "Usage source must match the provider protocol",
      });
  });
export type VoiceMetadata = z.infer<typeof voiceMetadataSchema>;

export const inferenceMetadataSchema = z.object({
  wire_protocol: z.enum([
    "anthropic_messages",
    "openai_responses",
    "openai_completions",
  ]),
  model_list: z.boolean(),
  realtime: z.boolean().optional(),
  voice: voiceMetadataSchema.nullish(),
});
export const inferenceViewSchema = inferenceMetadataSchema.extend({
  binding: z.enum(["platform", "user"]),
  status_slug: z.string().optional(),
});
const componentViewSchema = z.object({
  metric: z.string(),
  credits_per_unit: unitPriceSchema,
  sync_status: z.enum(["pending", "synced", "failed"]).optional(),
});
export const lanePricingViewSchema = componentViewSchema.extend({
  components: z.array(componentViewSchema).nullish(),
});
const componentInputSchema = componentViewSchema.extend({
  metric: z.enum(BILLING_METRICS),
});
export const lanePricingInputSchema = componentInputSchema
  .extend({
    components: z
      .array(componentInputSchema)
      .max(BILLING_METRICS.length - 1)
      .nullish(),
  })
  .superRefine((lane, ctx) => {
    const seen = new Set([lane.metric]);
    lane.components?.forEach((component, index) => {
      if (seen.has(component.metric))
        ctx.addIssue({
          code: "custom",
          path: ["components", index, "metric"],
          message: "Each unit may appear only once per lane",
        });
      seen.add(component.metric);
    });
  });
export const platformKeyConfigSchema = z.object({
  enabled: z.boolean(),
  audience: z.enum(["public", "restricted"]),
  allowed_owner_ids: z.array(z.string().uuid()).max(1000),
});
/** Additive portion of catalog responses; older/non-LLM entries are valid. */
export const catalogInferenceSchema = z.object({
  inference: inferenceViewSchema.nullish(),
  platform_key: z
    .object({
      available: z.boolean(),
      pricing: lanePricingViewSchema.nullish(),
    })
    .optional(),
  byok_pricing: lanePricingViewSchema.nullish(),
});
export type InferenceMetadata = z.infer<typeof inferenceMetadataSchema>;
export type InferenceView = z.infer<typeof inferenceViewSchema>;
export type LanePricingView = z.infer<typeof lanePricingViewSchema>;
export type PlatformKeyConfig = z.infer<typeof platformKeyConfigSchema>;
export function lanePriceLabel(price?: LanePricingView | null): string {
  if (!price) return "free";
  return [price, ...(price.components ?? [])]
    .map(
      (component) =>
        `${component.credits_per_unit} credits / ${metricLabel(component.metric, 1)}${component.sync_status && component.sync_status !== "synced" ? " (price pending; current billing applies)" : ""}`,
    )
    .join(" + ");
}
