import { z } from "zod";

export const CRM_VIEW_FORMAT_VERSION = 1 as const;
export const CRM_VIEW_RENDERER = "json-render" as const;
export const CRM_VIEW_RENDERER_SPEC_VERSION = "nyxid-crm-v1" as const;

export const crmViewIdentifierSchema = z
  .string()
  .regex(
    /^[A-Za-z][A-Za-z0-9_-]{0,63}$/,
    "Must start with a letter and contain at most 64 letters, numbers, underscores, or hyphens",
  );

export const crmFieldKeySchema = z
  .string()
  .regex(/^[a-z][a-z0-9_]{0,47}$/, "Must be a valid CRM field key");

const crmFilterOperatorSchema = z.enum([
  "eq",
  "neq",
  "in",
  "nin",
  "contains",
  "gt",
  "gte",
  "lt",
  "lte",
  "is_set",
  "is_unset",
]);

export const crmFilterSchema = z
  .object({
    field: crmFieldKeySchema,
    op: crmFilterOperatorSchema,
    value: z.unknown().optional(),
  })
  .strict();

export const crmTableQuerySchema = z
  .object({
    object: crmFieldKeySchema,
    select: z.array(crmFieldKeySchema).optional(),
    filters: z.array(crmFilterSchema).max(20),
    sort: z
      .object({
        field: crmFieldKeySchema,
        direction: z.enum(["asc", "desc"]),
      })
      .strict()
      .optional(),
    page_size: z.number().int().min(1).max(200).optional(),
    cursor: z.string().min(1).optional(),
    include_archived: z.boolean().optional(),
  })
  .strict();

const crmAggregateMeasureSchema = z.discriminatedUnion("op", [
  z.object({ op: z.literal("count") }).strict(),
  z
    .object({
      op: z.enum(["sum", "avg", "min", "max"]),
      field: crmFieldKeySchema,
    })
    .strict(),
]);

export const crmAggregateQuerySchema = z
  .object({
    object: crmFieldKeySchema,
    filters: z.array(crmFilterSchema).max(20),
    group_by: crmFieldKeySchema.optional(),
    measure: crmAggregateMeasureSchema,
    order: z.enum(["option_order", "value_desc", "label_asc"]),
  })
  .strict();

export const crmNamedTableQuerySchema = z
  .object({
    kind: z.literal("table"),
    query: crmTableQuerySchema,
  })
  .strict();

export const crmNamedAggregateQuerySchema = z
  .object({
    kind: z.literal("aggregate"),
    query: crmAggregateQuerySchema,
  })
  .strict();

export const crmNamedQuerySchema = z.discriminatedUnion("kind", [
  crmNamedTableQuerySchema,
  crmNamedAggregateQuerySchema,
]);

export const crmViewElementSchema = z
  .object({
    type: z.string().min(1),
    props: z.record(z.string(), z.unknown()),
    children: z.array(crmViewIdentifierSchema),
  })
  .strict();

export const crmViewSpecSchema = z
  .object({
    root: crmViewIdentifierSchema,
    elements: z.record(crmViewIdentifierSchema, crmViewElementSchema),
  })
  .strict();

export const crmViewEnvelopeBaseSchema = z
  .object({
    formatVersion: z.literal(CRM_VIEW_FORMAT_VERSION),
    renderer: z.literal(CRM_VIEW_RENDERER),
    rendererSpecVersion: z.literal(CRM_VIEW_RENDERER_SPEC_VERSION),
    queries: z.record(crmViewIdentifierSchema, crmNamedQuerySchema),
    spec: crmViewSpecSchema,
  })
  .strict()
  .superRefine((envelope, context) => {
    if (Object.keys(envelope.queries).length > 12) {
      context.addIssue({
        code: "too_big",
        maximum: 12,
        origin: "object",
        path: ["queries"],
        message: "A saved view can define at most 12 queries",
      });
    }
    if (Object.keys(envelope.spec.elements).length > 60) {
      context.addIssue({
        code: "too_big",
        maximum: 60,
        origin: "object",
        path: ["spec", "elements"],
        message: "A saved view can define at most 60 elements",
      });
    }
  });

export const crmValidationErrorSchema = z
  .object({
    path: z.string(),
    message: z.string(),
  })
  .strict();

export const crmAggregatePointSchema = z
  .object({
    key: z.string().nullable(),
    label: z.string(),
    value: z.number().finite(),
    archived: z.boolean(),
  })
  .strict();

export const crmAggregateResultSchema = z
  .object({
    points: z.array(crmAggregatePointSchema),
    unit: z
      .object({
        format: z.enum(["count", "number", "currency"]),
        currency: z.string().optional(),
      })
      .strict(),
    total: z.number().finite(),
    truncated: z.boolean(),
  })
  .strict();

export type CrmTableQuery = z.infer<typeof crmTableQuerySchema>;
export type CrmAggregateQuery = z.infer<typeof crmAggregateQuerySchema>;
export type CrmNamedTableQuery = z.infer<typeof crmNamedTableQuerySchema>;
export type CrmNamedAggregateQuery = z.infer<
  typeof crmNamedAggregateQuerySchema
>;
export type CrmNamedQuery = z.infer<typeof crmNamedQuerySchema>;
export type CrmViewElement = z.infer<typeof crmViewElementSchema>;
export type CrmViewSpec = z.infer<typeof crmViewSpecSchema>;
export type CrmViewEnvelope = z.infer<typeof crmViewEnvelopeBaseSchema>;
export type CrmValidationError = z.infer<typeof crmValidationErrorSchema>;
export type CrmAggregatePoint = z.infer<typeof crmAggregatePointSchema>;
export type CrmAggregateResult = z.infer<typeof crmAggregateResultSchema>;
