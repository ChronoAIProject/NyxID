import { z } from "zod";

export const poolStrategySchema = z.enum([
  "round_robin",
  "weighted",
  "priority",
]);
export type PoolStrategy = z.infer<typeof poolStrategySchema>;
export const poolContractSchema = z.enum(["same_api", "ai_chat"]);
export const retryTriggers = [
  "connect_error",
  "node_offline",
  "timeout",
  "transport_error",
  "http_408",
  "http_429",
  "http_500",
  "http_502",
  "http_503",
  "http_504",
  "http_529",
  "http_401",
  "http_403",
] as const;
export const failoverPolicySchema = z.object({
  max_attempts: z.number().int().min(1).max(5),
  per_attempt_timeout_ms: z.number().int().min(1000).max(300000),
  overall_deadline_ms: z.number().int().min(1000).max(600000),
  max_replay_body_bytes: z.number().int().min(1).max(4294967295),
  retry_on: z.array(z.enum(retryTriggers)),
  retry_ambiguous_dispatch: z.boolean(),
  cooldown: z
    .object({
      base_ms: z.number().int().min(1).max(3600000),
      max_ms: z.number().int().min(1).max(3600000),
      failures_to_open: z.number().int().min(1).max(4294967295),
      honor_retry_after: z.boolean(),
    })
    .refine((v) => v.max_ms >= v.base_ms, {
      message: "Maximum cooldown must be at least the base cooldown",
      path: ["max_ms"],
    }),
});
export type FailoverPolicy = z.infer<typeof failoverPolicySchema>;
export const defaultFailoverPolicy: FailoverPolicy = {
  max_attempts: 3,
  per_attempt_timeout_ms: 60000,
  overall_deadline_ms: 120000,
  max_replay_body_bytes: 8388608,
  retry_on: [
    "connect_error",
    "node_offline",
    "timeout",
    "http_429",
    "http_502",
    "http_503",
    "http_504",
    "http_529",
  ],
  retry_ambiguous_dispatch: false,
  cooldown: {
    base_ms: 5000,
    max_ms: 300000,
    failures_to_open: 1,
    honor_retry_after: true,
  },
};
const slugSchema = z
  .string()
  .trim()
  .min(1, "Slug is required")
  .max(80)
  .regex(
    /^[a-z0-9]+(?:-[a-z0-9]+)*$/,
    "Use lowercase letters, numbers, and single hyphens",
  );
export const poolMemberSchema = z.object({
  user_service_id: z.string().min(1, "Select a service"),
  weight: z.number().int().min(1).max(1000),
  enabled: z.boolean(),
  priority: z.number().int().min(0).max(4294967295).optional(),
  model: z.string().trim().max(256).nullable().optional(),
  same_api_compatible: z.boolean().optional(),
});
export const servicePoolSchema = z.object({
  id: z.string(),
  user_id: z.string(),
  slug: z.string(),
  name: z.string(),
  description: z.string().nullable().optional(),
  strategy: poolStrategySchema,
  tier_balance: z.enum(["round_robin", "weighted"]).optional(),
  member_contract: poolContractSchema.optional(),
  failover: failoverPolicySchema.nullable().optional(),
  config_revision: z.number().int().optional(),
  members: z.array(poolMemberSchema),
  rr_counter: z.number().int(),
  is_active: z.boolean(),
  created_at: z.string(),
  updated_at: z.string(),
});
export const servicePoolListResponseSchema = z.object({
  pools: z.array(servicePoolSchema),
});
const poolInputSchema = z.object({
  slug: slugSchema,
  name: z.string().trim().min(1, "Name is required").max(128),
  description: z.string().max(1024).optional(),
  strategy: poolStrategySchema,
  tier_balance: z.enum(["round_robin", "weighted"]).optional(),
  member_contract: poolContractSchema.optional(),
  failover: failoverPolicySchema.nullable().optional(),
  members: z.array(poolMemberSchema).max(50),
  is_active: z.boolean().optional(),
  org_id: z.string().optional(),
});
export const createServicePoolSchema = poolInputSchema.superRefine(
  (value, ctx) => {
    if (value.strategy === "priority" && value.member_contract === "ai_chat") {
      value.members.forEach((member, index) => {
        if (!member.model?.trim())
          ctx.addIssue({
            code: "custom",
            message: "AI chat members require a model",
            path: ["members", index, "model"],
          });
      });
    }
    if (
      new Set(value.members.map((m) => m.user_service_id)).size !==
      value.members.length
    )
      ctx.addIssue({
        code: "custom",
        message: "Select each member only once",
        path: ["members"],
      });
  },
);
export const updateServicePoolSchema = poolInputSchema
  .omit({ org_id: true })
  .partial()
  .extend({
    expected_revision: z.number().int().optional(),
    description: z.string().max(1024).nullable().optional(),
  });
export const setPoolMembersSchema = z.object({
  members: z.array(poolMemberSchema).max(50),
});
export type ServicePoolMember = z.infer<typeof poolMemberSchema>;
export type ServicePool = z.infer<typeof servicePoolSchema>;
export type ServicePoolListResponse = z.infer<
  typeof servicePoolListResponseSchema
>;
export type CreateServicePoolInput = z.infer<typeof createServicePoolSchema>;
export type UpdateServicePoolInput = z.infer<typeof updateServicePoolSchema>;
export type SetPoolMembersInput = z.infer<typeof setPoolMembersSchema>;
export interface PoolCandidate {
  user_service_id: string;
  slug: string;
  is_active: boolean;
  eligible: boolean;
  reason: string | null;
  credential_binding: string;
  protocol: string | null;
  catalog_service_id: string | null;
  requires_compatibility_declaration: boolean;
  cooldown_until: string | null;
  consecutive_failures: number;
  last_status: number | null;
}
export interface PoolCandidatesResponse {
  candidates: PoolCandidate[];
  next_cursor: string | null;
  has_more: boolean;
}
