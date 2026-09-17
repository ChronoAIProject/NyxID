import { z } from "zod";

export const optionItemSchema = z.object({
  value: z.string(),
  label: z.string(),
  description: z.string(),
  group: z.string(),
  source: z.enum(["backend_definition", "configured_scope"]),
  owner_id: z.string().nullable(),
  resource_id: z.string().nullable(),
  disabled: z.boolean(),
  disabled_reason: z.string().nullable(),
});

export const optionsResponseSchema = z.object({
  option_set: z.literal("service-scope"),
  principal_type: z.literal("service_account"),
  owner_id: z.string(),
  service_account_id: z.string().nullable(),
  items: z.array(optionItemSchema),
  selected_items: z.array(optionItemSchema),
  total: z.number().int().nonnegative(),
  next_offset: z.number().int().nonnegative().nullable(),
  version: z.string(),
  freshness: z.object({
    definitions_version: z.string(),
    resources: z.literal("live"),
    evaluated_at: z.string(),
    max_age_seconds: z.number().int().nonnegative(),
  }),
});
