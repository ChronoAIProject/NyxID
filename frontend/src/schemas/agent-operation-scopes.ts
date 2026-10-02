import { z } from "zod";

export const operationRuleSchema = z.object({
  method: z.enum(["GET", "HEAD", "OPTIONS", "POST", "PUT", "PATCH", "DELETE"]),
  path_template: z.string().min(1).max(2048).startsWith("/"),
});
export const operationSelectionSchema = z.object({
  expected_revision: z.number().int().nonnegative(),
  all_operations: z.boolean(),
  endpoint_ids: z.array(z.string()).max(256),
  rules: z.array(operationRuleSchema).max(256),
});
export const agentServiceOperationsSchema = z.object({
  service_id: z.string(),
  service_slug: z.string(),
  service_name: z.string(),
  revision: z.number().int().nonnegative(),
  all_operations: z.boolean(),
  allows_explicit_rules: z.boolean(),
  endpoint_ids: z.array(z.string()),
  rules: z.array(operationRuleSchema),
  operations: z.array(
    z.object({
      endpoint_id: z.string(),
      method: z.string(),
      path: z.string(),
      summary: z.string().nullable(),
      read_only: z.boolean(),
      changes_existing: z.boolean(),
    }),
  ),
});
export type OperationSelection = z.infer<typeof operationSelectionSchema>;
export type AgentServiceOperations = z.infer<
  typeof agentServiceOperationsSchema
>;
