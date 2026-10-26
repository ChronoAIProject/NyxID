import { z } from "zod";

export const cmaOperationSchema = z.object({
  id: z.string(),
  turn_id: z.string(),
  kind: z.enum(["cma_provisioning", "cma_response", "cma_agent"]),
  operation_id: z.string().nullable(),
  agent_id: z.string().nullable(),
  idempotency_key: z.string().nullable(),
  status: z.string().nullable(),
  phase: z.string().nullable(),
  state: z.string(),
  reason: z.string().nullable(),
  monitor_name: z.string(),
  created_at: z.string(),
  checked_at: z.string().nullable(),
});

export type CmaOperation = z.infer<typeof cmaOperationSchema>;

export function isCmaOperationPending(operation: CmaOperation): boolean {
  return ["submitting", "waiting", "cancelling"].includes(operation.state);
}
