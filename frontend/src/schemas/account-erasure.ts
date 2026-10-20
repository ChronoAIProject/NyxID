import { z } from "zod";

const erasureEffectsSchema = z.array(z.object({
    _id: z.string(), kind: z.string().nullable(), source_id: z.string().nullable(),
    target_owner_id: z.string().nullable(), provider_config_id: z.string().nullable(),
    status: z.string(), reason: z.string().optional().nullable(),
    retry_generation: z.number().int().nonnegative().optional(),
    last_reconciliation_request_digest: z.string().regex(/^[0-9a-f]{64}$/).optional().nullable(),
    created_at: z.string().nullable(), updated_at: z.string().nullable(),
  }).strict());

export const accountErasureSchema = z.object({
  job_id: z.string().uuid().optional(),
  status: z.enum(["accepted", "pending", "operator_action", "complete"]),
  accepted_at: z.string(),
  completed_at: z.string().nullable(),
  effects: erasureEffectsSchema.optional(),
  next_effect_after: z.string().nullable().optional(),
  domains: z.array(z.object({
    domain: z.string(),
    retry_generation: z.number().int().nonnegative().optional(),
    status: z.enum(["pending", "operator_action", "complete"]),
    reason: z.string().nullable(),
    completed_at: z.string().nullable(),
  }).strict()),
}).strict();
export type AccountErasure = z.infer<typeof accountErasureSchema>;
export const erasureProgressSchema = z.object({
  erasure: accountErasureSchema.nullable(),
  next_after: z.string().nullable(),
  next_node_after: z.string().nullable(),
  node_drains: z.array(z.object({node_id:z.string(),fence:z.object({node_id:z.string(),instance_name:z.string(),generation_id:z.string(),connection_id:z.string()}).strict().nullable()}).strict()),
  effects: erasureEffectsSchema,

}).strict();
