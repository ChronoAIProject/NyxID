import { z } from "zod";

export const modelCapabilitiesSchema = z.object({
  protocols: z.array(z.string()),
  input_modalities: z.array(z.string()),
  output_modalities: z.array(z.string()),
  context_tokens: z.number().nullable().optional(),
  output_tokens: z.number().nullable().optional(),
  streaming: z.boolean().nullable().optional(),
  tools: z.boolean().nullable().optional(),
  structured_output: z.boolean().nullable().optional(),
});
const selectorSchema = z.object({
  kind: z.string(),
  ttl: z.string().optional(),
  modality: z.string().optional(),
});
export const retailComponentSchema = z.object({
  metric: z.string(),
  selector: selectorSchema,
  credits_per_unit: z.string(),
  explicit_free: z.boolean(),
});
export const publicModelSchema = z.object({
  id: z.string(),
  display_name: z.string(),
  description: z.string(),
  capabilities: modelCapabilitiesSchema,
  availability: z.string(),
  retail_prices: z
    .array(
      z.object({
        variant: z.record(z.string(), z.string()),
        available: z.boolean(),
        components: z.array(retailComponentSchema),
      }),
    )
    .default([]),
});
export const publicModelListSchema = z.object({
  models: z.array(publicModelSchema),
  next_after: z.string().nullable(),
});
export type PublicModel = z.infer<typeof publicModelSchema>;
export type RetailComponent = z.infer<typeof retailComponentSchema>;
export const routeSchema = z.object({
  id: z.uuid(),
  credential_service_id: z.uuid(),
  enabled: z.boolean(),
  priority: z.number().int().min(0).max(65535),
  weight: z.number().int().min(1).max(65535),
  target: z.discriminatedUnion("kind", [
    z.object({
      kind: z.literal("direct_provider"),
      provider: z.enum(["openai", "anthropic"]),
      native_model: z.string().min(1),
    }),
    z.object({
      kind: z.literal("vertex_publisher"),
      publisher: z.enum(["google", "anthropic"]),
      project: z.string().min(1),
      location: z.string().min(1),
      native_model: z.string().min(1),
    }),
    z.object({
      kind: z.literal("vertex_openapi_maas"),
      publisher: z.string().min(1),
      project: z.string().min(1),
      location: z.string().min(1),
      native_model: z.string().min(1),
    }),
  ]),
});
export const entitlementsSchema = z.object({
  all_people: z.boolean(),
  rules: z
    .array(
      z.object({
        subject_kind: z.enum(["person", "org", "group"]),
        subject_id: z.uuid(),
        effect: z.enum(["allow", "deny"]),
      }),
    )
    .max(500),
});
export const adminModelSchema = z.object({
  id: z.string(),
  public_id: z.string(),
  display_name: z.string(),
  description: z.string(),
  family: z.enum(["gemini", "claude", "openai", "open_model"]),
  capabilities: modelCapabilitiesSchema,
  lifecycle: z.enum(["draft", "active", "retired"]),
  enabled: z.boolean(),
  revision: z.number().int(),
  authorization_revision: z.number().int(),
  billing_service_id: z.string().nullable(),
  entitlements: entitlementsSchema,
  routing_revision_id: z.string().nullable(),
  routes: z.array(routeSchema),
  execution_ready: z.boolean(),
  blocking_reason: z.string(),
  required_readiness: z.array(z.string()),
  created_at: z.string(),
  updated_at: z.string(),
});
export const adminModelListSchema = z.object({
  models: z.array(adminModelSchema),
  next_after: z.string().nullable(),
});
export type AdminModel = z.infer<typeof adminModelSchema>;
export const accessExplanationSchema = z.object({
  model_id: z.string(),
  user_id: z.string(),
  revision: z.number(),
  policy_reason: z.string(),
  model_reason: z.string(),
  flag_enabled: z.boolean(),
  executable: z.boolean(),
  route_count: z.number().nullable(),
  route_revision_checked: z.number().nullable(),
});
export const tariffSchema = z.object({
  id: z.string(),
  revision: z.number(),
  service_id: z.string(),
  model_id: z.string(),
  public_model: z.string(),
  lane: z.string(),
  variant: z.record(z.string(), z.string()),
  contract: z.record(z.string(), z.unknown()),
  bounds: z.record(z.string(), z.number()),
  components: z.array(
    retailComponentSchema.extend({
      key: z.string(),
      max_quantity: z.number(),
      lago_metric_code: z.string(),
    }),
  ),
  source: z.string(),
  status: z.string(),
  sync_error: z.string().nullable(),
  created_at: z.string(),
});
export type Tariff = z.infer<typeof tariffSchema>;
export const exactPrice = z
  .string()
  .regex(
    /^\d+(?:\.\d{1,12})?$/,
    "Enter nonnegative credits with at most 12 decimal places",
  );
export const modelFormSchema = z
  .object({
    public_id: z
      .string()
      .max(126)
      .regex(
        /^nyxid\/[a-z0-9][a-z0-9._-]*$/,
        "Use an immutable nyxid/model-name ID",
      ),
    display_name: z
      .string()
      .trim()
      .min(1)
      .max(160)
      .refine(
        (s) => new TextEncoder().encode(s).length <= 160 && !/\p{Cc}/u.test(s),
        "Use a display name of at most 160 bytes without control characters",
      ),
    description: z
      .string()
      .max(2000)
      .refine(
        (s) =>
          new TextEncoder().encode(s).length <= 2000 &&
          !/\p{Cc}/u.test(s.replaceAll("\n", "")),
        "Use a description of at most 2000 bytes; only newline control characters are allowed",
      ),
    family: z.enum(["gemini", "claude", "openai", "open_model"]),
    protocols: z.array(z.string()),
    input_modalities: z.array(z.string()),
    output_modalities: z.array(z.string()),
    context_tokens: z
      .string()
      .regex(/^\d*$/)
      .refine(
        (s) => !s || (Number(s) > 0 && Number(s) <= 100000000),
        "Enter a positive limit up to 100000000",
      ),
    output_tokens: z
      .string()
      .regex(/^\d*$/)
      .refine(
        (s) => !s || (Number(s) > 0 && Number(s) <= 100000000),
        "Enter a positive limit up to 100000000",
      ),
    streaming: z.enum(["unknown", "yes", "no"]),
    tools: z.enum(["unknown", "yes", "no"]),
    structured_output: z.enum(["unknown", "yes", "no"]),
    lifecycle: z.enum(["draft", "active", "retired"]),
    enabled: z.boolean(),
    billing_service_id: z.union([z.literal(""), z.uuid()]),
  })
  .superRefine((v, ctx) => {
    const name = v.public_id.replace(/^nyxid\//, "");
    const matches =
      v.family === "open_model" ||
      (v.family === "gemini"
        ? name.startsWith("gemini-")
        : v.family === "claude"
          ? name.startsWith("claude-")
          : /^(gpt-|o1|o3|o4)/.test(name));
    if (!matches)
      ctx.addIssue({
        code: "custom",
        path: ["public_id"],
        message: "Use a real model name matching the selected family",
      });
    if (
      v.context_tokens &&
      v.output_tokens &&
      Number(v.output_tokens) > Number(v.context_tokens)
    )
      ctx.addIssue({
        code: "custom",
        path: ["output_tokens"],
        message: "Output token limit cannot exceed the context limit",
      });
  });
export type ModelForm = z.infer<typeof modelFormSchema>;
export function modelDefaults(model?: AdminModel): ModelForm {
  const tri = (value?: boolean | null) =>
    value == null ? "unknown" : value ? "yes" : "no";
  return {
    public_id: model?.public_id ?? "nyxid/",
    display_name: model?.display_name ?? "",
    description: model?.description ?? "",
    family: model?.family ?? "gemini",
    protocols: model?.capabilities.protocols ?? [],
    input_modalities: model?.capabilities.input_modalities ?? ["text"],
    output_modalities: model?.capabilities.output_modalities ?? ["text"],
    context_tokens: model?.capabilities.context_tokens?.toString() ?? "",
    output_tokens: model?.capabilities.output_tokens?.toString() ?? "",
    streaming: tri(model?.capabilities.streaming),
    tools: tri(model?.capabilities.tools),
    structured_output: tri(model?.capabilities.structured_output),
    lifecycle: model?.lifecycle ?? "draft",
    enabled: model?.enabled ?? false,
    billing_service_id: model?.billing_service_id ?? "",
  };
}
export function modelPayload(form: ModelForm, revision?: number) {
  const capabilities = {
    protocols: form.protocols,
    input_modalities: form.input_modalities,
    output_modalities: form.output_modalities,
    context_tokens: form.context_tokens ? Number(form.context_tokens) : null,
    output_tokens: form.output_tokens ? Number(form.output_tokens) : null,
    streaming: form.streaming === "unknown" ? null : form.streaming === "yes",
    tools: form.tools === "unknown" ? null : form.tools === "yes",
    structured_output:
      form.structured_output === "unknown"
        ? null
        : form.structured_output === "yes",
  };
  return revision === undefined
    ? {
        public_id: form.public_id,
        family: form.family,
        display_name: form.display_name,
        description: form.description,
        capabilities,
      }
    : {
        expected_revision: revision,
        display_name: form.display_name,
        description: form.description,
        capabilities,
        lifecycle: form.lifecycle,
        enabled: form.enabled,
        billing_service_id: form.billing_service_id || null,
      };
}
export function componentLabel(
  component: Pick<RetailComponent, "metric" | "selector">,
) {
  const labels: Record<string, string> = {
    input: "Input tokens",
    output: "Output tokens",
    reasoning: "Reasoning tokens",
    cache_read: "Cache read tokens",
    cache_write: "Cache write tokens",
    image_input: "Image input tokens",
    image_output: "Image output tokens",
    audio_input: "Audio input tokens",
    audio_output: "Audio output tokens",
    images: "Images",
    requests: "Requests",
    cached_modality: "Cached tokens",
  };
  const ttl: Record<string, string> = {
    five_minutes: "5 minutes",
    one_hour: "1 hour",
    thirty_minutes: "30 minutes",
    twelve_hours: "12 hours",
  };
  return [
    labels[component.selector.kind] ?? component.metric.replaceAll("_", " "),
    component.selector.ttl
      ? (ttl[component.selector.ttl] ?? component.selector.ttl)
      : undefined,
    component.selector.modality,
  ]
    .filter(Boolean)
    .join(" · ");
}

export function titleCase(value: string) {
  return value.replaceAll("_", " ").replace(/^./, (s) => s.toUpperCase());
}
