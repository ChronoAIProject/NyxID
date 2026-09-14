import { z } from "zod";

export const inferenceMetadataSchema = z.object({
  wire_protocol: z.enum([
    "anthropic_messages",
    "openai_responses",
    "openai_completions",
  ]),
  model_list: z.boolean(),
  realtime: z.boolean().optional(),
});
export const inferenceViewSchema = inferenceMetadataSchema.extend({
  binding: z.enum(["platform", "user"]),
  status_slug: z.string().optional(),
});
export const lanePricingViewSchema = z.object({
  metric: z.enum(["tokens", "requests", "bytes"]),
  credits_per_unit: z.string().regex(/^\d+(?:\.\d{1,6})?$/),
  sync_status: z.enum(["pending", "synced", "failed"]).optional(),
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
  const unit = { tokens: "token", requests: "request", bytes: "byte" }[
    price.metric
  ];
  return `${price.credits_per_unit} credits / ${unit}${price.sync_status && price.sync_status !== "synced" ? " (price pending; current billing applies)" : ""}`;
}
