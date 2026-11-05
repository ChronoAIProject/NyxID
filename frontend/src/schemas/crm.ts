import { z } from "zod";

const crmIdSchema = z.string().min(1);
const crmKeySchema = z.string().regex(/^[a-z][a-z0-9_]{0,47}$/);
const crmOptionValueSchema = z.string().regex(/^[a-z0-9_]{1,48}$/);
const crmInstantSchema = z.string().datetime({ offset: true });
const crmCalendarDateSchema = z.string().regex(/^\d{4}-\d{2}-\d{2}$/);
const crmRevisionSchema = z.number().int().positive();
const crmViewIdentifierSchema = z
  .string()
  .regex(/^[A-Za-z][A-Za-z0-9_-]{0,63}$/);

export const crmAccessSchema = z.enum(["read", "work", "schema"]);

export const crmCurrencyCodeSchema = z.enum([
  "USD",
  "EUR",
  "GBP",
  "SGD",
  "AUD",
  "CAD",
  "HKD",
  "CNY",
]);

export const crmSelectOptionSchema = z.object({
  value: crmOptionValueSchema,
  label: z.string().min(1),
  color: z.string().max(64).nullable().optional(),
  archived: z.boolean(),
});

export const crmFieldSchema = z.discriminatedUnion("type", [
  z.object({
    key: crmKeySchema,
    label: z.string().min(1),
    required: z.boolean(),
    archived: z.boolean(),
    type: z.literal("text"),
    max_length: z.number().int().min(1).max(10_000),
  }),
  z.object({
    key: crmKeySchema,
    label: z.string().min(1),
    required: z.boolean(),
    archived: z.boolean(),
    type: z.literal("number"),
    format: z.enum(["plain", "percent"]),
  }),
  z.object({
    key: crmKeySchema,
    label: z.string().min(1),
    required: z.boolean(),
    archived: z.boolean(),
    type: z.literal("currency"),
    currency: crmCurrencyCodeSchema,
  }),
  z.object({
    key: crmKeySchema,
    label: z.string().min(1),
    required: z.boolean(),
    archived: z.boolean(),
    type: z.literal("boolean"),
  }),
  z.object({
    key: crmKeySchema,
    label: z.string().min(1),
    required: z.boolean(),
    archived: z.boolean(),
    type: z.literal("date"),
  }),
  z.object({
    key: crmKeySchema,
    label: z.string().min(1),
    required: z.boolean(),
    archived: z.boolean(),
    type: z.literal("datetime"),
  }),
  z.object({
    key: crmKeySchema,
    label: z.string().min(1),
    required: z.boolean(),
    archived: z.boolean(),
    type: z.literal("select"),
    options: z.array(crmSelectOptionSchema),
  }),
  z.object({
    key: crmKeySchema,
    label: z.string().min(1),
    required: z.boolean(),
    archived: z.boolean(),
    type: z.literal("record"),
    target: crmKeySchema,
    cardinality: z.enum(["one", "many"]),
  }),
  z.object({
    key: crmKeySchema,
    label: z.string().min(1),
    required: z.boolean(),
    archived: z.boolean(),
    type: z.literal("user"),
  }),
]);

export const crmPipelineSchema = z.object({
  stage_field: crmKeySchema,
  closed_stages: z.array(crmOptionValueSchema),
});

export const crmObjectMetadataSchema = z.object({
  id: crmIdSchema,
  owner_id: crmIdSchema,
  key: crmKeySchema,
  label: z.string().min(1),
  plural_label: z.string().min(1),
  schema_revision: crmRevisionSchema,
  fields: z.array(crmFieldSchema),
  pipeline: crmPipelineSchema.nullable(),
  default_columns: z.array(crmKeySchema),
  seeded: z.boolean(),
  created_at: crmInstantSchema,
  updated_at: crmInstantSchema,
});

export const crmObjectSchema = crmObjectMetadataSchema.extend({
  access: crmAccessSchema,
});

export const crmObjectsResponseSchema = z.object({
  objects: z.array(crmObjectSchema),
});

export const crmRecordLinkSchema = z.object({
  id: crmIdSchema,
  object_key: crmKeySchema,
  label: z.string(),
  archived: z.boolean(),
});

export const crmActorSchema = z.object({
  kind: z.enum(["human", "agent"]),
  user_id: crmIdSchema,
  agent_id: crmIdSchema.nullable().optional(),
  agent_name: z.string().nullable().optional(),
  api_key_id: crmIdSchema.nullable().optional(),
  source: z
    .object({
      conversation_id: crmIdSchema,
      turn_id: z.string(),
      run_id: z.string().nullable(),
      ordered_at: crmInstantSchema,
    })
    .nullable()
    .optional(),
});

export const crmRecordRowSchema = z.object({
  id: crmIdSchema,
  revision: crmRevisionSchema,
  values: z.record(z.string(), z.unknown()),
  links: z.record(z.string(), z.array(crmRecordLinkSchema)),
  overrides: z.array(crmKeySchema),
  archived: z.boolean(),
});

export const crmRecordSchema = crmRecordRowSchema.extend({
  owner_id: crmIdSchema,
  object_key: crmKeySchema,
  created_by: crmActorSchema.nullish(),
  updated_by: crmActorSchema.nullish(),
  field_sources: z.record(z.string(), crmInstantSchema).optional(),
  created_at: crmInstantSchema,
  updated_at: crmInstantSchema,
});

export const crmFilterOperatorSchema = z.enum([
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

export const crmFilterSchema = z.object({
  field: crmKeySchema,
  op: crmFilterOperatorSchema,
  value: z.unknown().optional(),
});

export const crmTableQuerySchema = z.object({
  object: crmKeySchema,
  select: z.array(crmKeySchema).optional(),
  filters: z.array(crmFilterSchema).max(20),
  sort: z
    .object({
      field: crmKeySchema,
      direction: z.enum(["asc", "desc"]),
    })
    .optional(),
  page_size: z.number().int().min(1).max(200).default(50),
  cursor: z.string().min(1).optional(),
  include_archived: z.boolean().default(false),
});

export const crmTableResultSchema = z.object({
  object: crmObjectMetadataSchema,
  rows: z.array(crmRecordRowSchema),
  next_cursor: z.string().nullable(),
  total_count: z.number().int().nonnegative(),
});

export const crmAggregateMeasureSchema = z.discriminatedUnion("op", [
  z.object({ op: z.literal("count") }),
  z.object({
    op: z.enum(["sum", "avg", "min", "max"]),
    field: crmKeySchema,
  }),
]);

export const crmAggregateQuerySchema = z.object({
  object: crmKeySchema,
  filters: z.array(crmFilterSchema).max(20),
  group_by: crmKeySchema.optional(),
  date_bucket: z
    .object({
      unit: z.enum(["day", "week", "month"]),
      timezone: z.string().min(1).max(100),
    })
    .strict()
    .optional(),
  measure: crmAggregateMeasureSchema,
  order: z.enum(["option_order", "value_desc", "label_asc"]),
});

export const crmAggregateResultSchema = z.object({
  points: z.array(
    z.object({
      key: z.union([z.string(), z.boolean(), z.null()]),
      label: z.string(),
      value: z.number().nullable(),
      archived: z.boolean(),
    }),
  ),
  unit: z.object({
    format: z.enum(["count", "number", "percent", "currency"]),
    currency: crmCurrencyCodeSchema
      .nullish()
      .transform((value) => value ?? undefined),
  }),
  total: z.number().nullable(),
  truncated: z.boolean(),
});

export const crmRecordResponseSchema = crmRecordSchema;

export const crmCreateRecordRequestSchema = z.object({
  owner_id: crmIdSchema.optional(),
  object: crmKeySchema,
  values: z.record(z.string(), z.unknown()),
  protect: z.array(crmKeySchema).optional(),
});

export const crmPatchRecordRequestSchema = z.object({
  owner_id: crmIdSchema.optional(),
  expected_revision: crmRevisionSchema,
  values: z.record(z.string(), z.unknown()),
  protect: z.array(crmKeySchema).optional(),
  unprotect: z.array(crmKeySchema).optional(),
});

export const crmRevisionRequestSchema = z.object({
  owner_id: crmIdSchema.optional(),
  expected_revision: crmRevisionSchema,
});

export const crmMoveRecordRequestSchema = crmRevisionRequestSchema.extend({
  to: crmOptionValueSchema,
});

export const crmActivitySchema = z.object({
  id: crmIdSchema,
  record_id: crmIdSchema,
  object_key: crmKeySchema,
  kind: z.enum(["created", "updated", "stage_changed", "note"]),
  actor: crmActorSchema,
  summary: z.object({
    changed_fields: z
      .array(crmKeySchema)
      .nullish()
      .transform((value) => value ?? undefined),
    from: z
      .string()
      .nullish()
      .transform((value) => value ?? undefined),
    to: z
      .string()
      .nullish()
      .transform((value) => value ?? undefined),
  }),
  body: z
    .string()
    .min(1)
    .max(10_000)
    .nullish()
    .transform((value) => value ?? undefined),
  created_at: crmInstantSchema,
});

export const crmActivitiesResponseSchema = z.object({
  activities: z.array(crmActivitySchema),
  next_cursor: z.string().nullable(),
});

export const crmNoteRequestSchema = z.object({
  owner_id: crmIdSchema.optional(),
  body: z.string().trim().min(1).max(10_000),
});

export const crmRelatedGroupSchema = z.object({
  object_key: crmKeySchema,
  field_key: crmKeySchema,
  records: z.array(crmRecordSchema).max(50),
  count: z.number().int().nonnegative(),
});

export const crmRelatedResponseSchema = z.object({
  groups: z.array(crmRelatedGroupSchema),
});

export const crmValidationIssueSchema = z.object({
  path: z.string().min(1),
  message: z.string().min(1),
});

export const crmRevisionConflictDetailsSchema = z.object({
  current: crmRecordSchema,
});

export const crmObjectRevisionConflictDetailsSchema = z.object({
  current: crmObjectMetadataSchema,
});

export const crmValidationDetailsSchema = z.object({
  errors: z.array(crmValidationIssueSchema),
});

const crmStackElementSchema = z.object({
  type: z.literal("Stack"),
  props: z.object({
    gap: z.enum(["sm", "md", "lg"]),
    direction: z.enum(["vertical", "horizontal"]).optional(),
  }),
  children: z.array(z.string()),
});
const crmGridElementSchema = z.object({
  type: z.literal("Grid"),
  props: z.object({ columns: z.number().int().min(1).max(4) }),
  children: z.array(z.string()),
});
const crmTextElementSchema = z.object({
  type: z.literal("Text"),
  props: z.object({
    content: z.string().min(1).max(2_000),
    variant: z.enum(["title", "heading", "body", "muted"]),
  }),
  children: z.array(z.string()).max(0),
});
const crmMetricElementSchema = z.object({
  type: z.literal("Metric"),
  props: z.object({ queryId: z.string().min(1), title: z.string().min(1) }),
  children: z.array(z.string()).max(0),
});
const crmChartPropsSchema = z.object({
  queryId: z.string().min(1),
  title: z.string().min(1),
  height: z.enum(["small", "medium", "large"]).optional(),
});
const crmBarChartElementSchema = z.object({
  type: z.literal("BarChart"),
  props: crmChartPropsSchema,
  children: z.array(z.string()).max(0),
});
const crmPieChartElementSchema = z.object({
  type: z.literal("PieChart"),
  props: crmChartPropsSchema,
  children: z.array(z.string()).max(0),
});
const crmObjectTableElementSchema = z.object({
  type: z.literal("ObjectTable"),
  props: z.object({
    queryId: z.string().min(1),
    columns: z.array(crmKeySchema).optional(),
  }),
  children: z.array(z.string()).max(0),
});
const crmPipelineBoardElementSchema = z.object({
  type: z.literal("PipelineBoard"),
  props: z.object({ queryId: z.string().min(1) }),
  children: z.array(z.string()).max(0),
});
const crmRecordFormElementSchema = z.object({
  type: z.literal("RecordForm"),
  props: z.object({
    object: crmKeySchema,
    fields: z.array(crmKeySchema).optional(),
    title: z.string().min(1),
    submitLabel: z.string().min(1).optional(),
  }),
  children: z.array(z.string()).max(0),
});

export const crmViewElementSchema = z.discriminatedUnion("type", [
  crmStackElementSchema,
  crmGridElementSchema,
  crmTextElementSchema,
  crmMetricElementSchema,
  crmBarChartElementSchema,
  crmPieChartElementSchema,
  crmObjectTableElementSchema,
  crmPipelineBoardElementSchema,
  crmRecordFormElementSchema,
]);

export const crmSavedViewEnvelopeSchema = z.object({
  formatVersion: z.literal(1),
  renderer: z.literal("json-render"),
  rendererSpecVersion: z.literal("nyxid-crm-v1"),
  queries: z.record(
    crmViewIdentifierSchema,
    z.discriminatedUnion("kind", [
      z.object({ kind: z.literal("table"), query: crmTableQuerySchema }),
      z.object({
        kind: z.literal("aggregate"),
        query: crmAggregateQuerySchema,
      }),
    ]),
  ),
  spec: z.object({
    root: crmViewIdentifierSchema,
    elements: z.record(crmViewIdentifierSchema, crmViewElementSchema),
  }),
});

export const crmSavedViewSchema = z.object({
  id: crmIdSchema,
  owner_id: crmIdSchema,
  title: z.string().min(1),
  revision: crmRevisionSchema,
  envelope: crmSavedViewEnvelopeSchema,
  created_by: crmActorSchema,
  created_at: crmInstantSchema,
  updated_at: crmInstantSchema,
  valid: z
    .boolean()
    .nullish()
    .transform((value) => value ?? undefined),
  errors: z
    .array(crmValidationIssueSchema)
    .nullish()
    .transform((value) => value ?? undefined),
});

export const crmSavedViewsResponseSchema = z.object({
  views: z.array(crmSavedViewSchema),
});

export const crmViewValidationResponseSchema = z.object({
  valid: z.boolean(),
  errors: z.array(crmValidationIssueSchema),
});

export type CrmField = z.infer<typeof crmFieldSchema>;
export type CrmObjectMetadata = z.infer<typeof crmObjectMetadataSchema>;
export type CrmObject = z.infer<typeof crmObjectSchema>;
export type CrmRecordLink = z.infer<typeof crmRecordLinkSchema>;
export type CrmRecordRow = z.infer<typeof crmRecordRowSchema>;
export type CrmRecord = z.infer<typeof crmRecordSchema>;
export type CrmFilter = z.infer<typeof crmFilterSchema>;
export type CrmFilterOperator = z.infer<typeof crmFilterOperatorSchema>;
export type CrmTableQuery = z.input<typeof crmTableQuerySchema>;
export type CrmTableResult = z.infer<typeof crmTableResultSchema>;
export type CrmAggregateQuery = z.infer<typeof crmAggregateQuerySchema>;
export type CrmAggregateResult = z.infer<typeof crmAggregateResultSchema>;
export type CrmActivity = z.infer<typeof crmActivitySchema>;
export type CrmRelatedGroup = z.infer<typeof crmRelatedGroupSchema>;
export type CrmSavedViewEnvelope = z.infer<typeof crmSavedViewEnvelopeSchema>;

export { crmCalendarDateSchema, crmInstantSchema, crmKeySchema };
