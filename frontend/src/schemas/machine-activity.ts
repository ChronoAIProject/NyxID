import { z } from "zod";

export const machineReceiptSchema = z.object({
  operation_id: z.string(),
  node_id: z.string(),
  machine_name: z.string().nullish(),
  agent_id: z.string(),
  context_mode: z.enum(["shared_legacy", "separated"]).nullish(),
  action: z.string(),
  status: z.enum([
    "running",
    "completed",
    "finished",
    "error",
    "cancelled",
    "unknown",
  ]),
  job_id: z.string().nullable(),
  exit_code: z.number().nullable(),
  bytes: z.number().nonnegative().nullable(),
  duration_ms: z.number().nonnegative().nullable(),
  error_code: z.number().nullable(),
  screenshot_id: z.string().nullable(),
  preview_id: z.string().nullable(),
  preview_enabled: z.boolean(),
});
export type MachineReceipt = z.infer<typeof machineReceiptSchema>;
export const machineActivityPageSchema = z.object({
  machine_name: z.string().nullish(),
  agents: z
    .array(
      z.object({
        id: z.string(),
        name: z.string(),
        display_name: z.string().nullable(),
        kind: z.enum(["nyxbot", "specialist"]),
      }),
    )
    .default([]),
  entries: z.array(
    z.object({
      id: z.string(),
      agent_id: z.string().nullable(),
      actor_id: z.string().nullable(),
      action: z.string(),
      outcome: z.string(),
      operation_id: z.string().nullable(),
      activity_id: z.string().nullable(),
      job_id: z.string().nullable(),
      exit_code: z.number().nullable(),
      duration_ms: z.number().nullable(),
      created_at: z.string(),
    }),
  ),
  next_cursor: z.string().nullable(),
});
