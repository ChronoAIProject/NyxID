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
const page = {
  items: z.array(optionItemSchema),
  total: z.number().int().nonnegative(),
  next_offset: z.number().int().nonnegative().nullable(),
  version: z.string(),
};
const freshness = {
  definitions_version: z.string(),
  evaluated_at: z.string(),
  max_age_seconds: z.number().int().nonnegative(),
};
export const optionsResponseSchema = z.discriminatedUnion("option_set", [
  z.object({
    ...page,
    option_set: z.literal("service-scope"),
    principal_type: z.literal("service_account"),
    owner_id: z.string(),
    service_account_id: z.string().nullable(),
    selected_items: z.array(optionItemSchema),
    freshness: z.object({ ...freshness, resources: z.literal("live") }),
  }),
  ...(["service-history-action", "service-history-field"] as const).map((set) =>
    z
      .object({
        ...page,
        option_set: z.literal(set),
        selected_items: z.array(optionItemSchema).max(0).default([]),
        freshness: z.object({ ...freshness, resources: z.literal("static") }),
      })
      .strict(),
  ),
]);
