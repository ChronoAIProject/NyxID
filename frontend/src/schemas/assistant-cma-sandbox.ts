import { z } from "zod";

const optionalTime = z.string().nullable();
export const cmaSandboxSchema = z.object({
  id: z.string().min(1),
  label: z.string(),
  cma_sandbox_id: z.string().nullable(),
  generation: z.number().int().positive(),
  occupies_account_slot: z.boolean(),
  admission_state: z.enum([
    "reserved",
    "creating",
    "associated",
    "retiring",
    "unknown",
    "retired",
  ]),
  observation_state: z.enum([
    "unknown",
    "provisioning",
    "ready",
    "paused",
    "retiring",
    "retired",
    "failed",
  ]),
  resource_state: z.string().nullable(),
  machine_included: z.boolean().nullable(),
  cpu_millis: z.number().int().nonnegative().nullable(),
  memory_mib: z.number().int().nonnegative().nullable(),
  capacity: z
    .object({
      cpu_millis: z.number().int().nonnegative().nullable(),
      memory_mib: z.number().int().nonnegative().nullable(),
      source: z
        .enum(["profile_revision", "runtime_allocation"])
        .nullable()
        .default(null),
    })
    .optional(),
  last_activity_at: optionalTime,
  last_observed: optionalTime,
  retire_requested_at: optionalTime,
  retired_at: optionalTime,
  billing: z.object({
    provider: z.literal("cma"),
    cost_status: z.literal("unknown"),
  }),
});
export const cmaSandboxListSchema = z.object({
  sandboxes: z.array(cmaSandboxSchema),
  occupied: z.number().int().nonnegative(),
  limit: z.number().int().positive(),
  conflict: z.boolean(),
  inventory_complete: z.boolean(),
  next_cursor: z.string().max(1024).nullish(),
});
export const cmaSandboxMutationSchema = z.object({
  sandbox: cmaSandboxSchema,
  pending: z.boolean(),
});
export const cmaSandboxLabelSchema = z.object({
  label: z
    .string()
    .trim()
    .min(1, "Enter a name")
    .max(100, "Use at most 100 characters"),
});
export type CmaSandbox = z.infer<typeof cmaSandboxSchema>;
export type CmaSandboxList = z.infer<typeof cmaSandboxListSchema>;
