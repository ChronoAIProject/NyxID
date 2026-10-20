import { z } from "zod";

export const concurrencyLimitSchema = z.number().int().min(1).max(10000).nullable();
const overrideSchema = z.object({ id: z.uuid(), limit: concurrencyLimitSchema });
export const concurrencyPolicySchema = z.object({
  default_limit: concurrencyLimitSchema,
  users: z.array(overrideSchema).max(500),
  orgs: z.array(overrideSchema).max(500),
}).superRefine((policy, ctx) => {
  const ids = [...policy.users, ...policy.orgs].map((row) => row.id);
  if (ids.length > 500) ctx.addIssue({ code: "custom", message: "Choose at most 500 override targets", path: ["users"] });
  if (new Set(ids).size !== ids.length) ctx.addIssue({ code: "custom", message: "Each target may appear only once", path: ["users"] });
});
export const concurrencyFormSchema = z.object({ enabled: z.boolean(), policy: concurrencyPolicySchema });
export type ConcurrencyPolicy = z.infer<typeof concurrencyPolicySchema>;
export type ConcurrencyForm = z.infer<typeof concurrencyFormSchema>;
export type ConcurrencyResponse = { policy: ConcurrencyPolicy | null };
