import { z } from "zod";
import {
  BILLING_METRICS,
  metricLabel,
  unitPriceSchema,
} from "./billing-metrics";

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
