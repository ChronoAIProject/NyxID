import { z } from "zod";

const bytes = z.number().int().nonnegative().max(Number.MAX_SAFE_INTEGER);
const volume = z.object({
  capacity_bytes: bytes,
  used_bytes: bytes,
  available_bytes: bytes.nullable(),
  measurement: z.string(),
  complete: z.boolean(),
});

// Current temporary telemetry is a partial path measurement with no allocation.
const temporaryVolume = volume.extend({
  measurement: z.literal("node_paths_lower_bound"),
  complete: z.literal(false),
  capacity_bytes: z.null().optional(),
  available_bytes: z.null().optional(),
});

/** Actual owner /remote-machines/{id}/usage projection; absence is not zero. */
export const remoteMachineUsageSchema = z.object({
  available: z.boolean(),
  stale: z.boolean(),
  received_at: z.string().datetime({ offset: true }).nullable(),
  sampled_at: z.string().datetime({ offset: true }).nullish(),
  workspace: volume.nullable(),
  temporary: temporaryVolume.nullable(),
  platform_storage: z
    .object({
      source: z.string(),
      used_bytes: bytes.nullable(),
      capacity_bytes: z.null().optional(),
      available_bytes: z.null(),
      complete: z.boolean(),
      stale: z.boolean(),
      received_at: z.string().datetime({ offset: true }),
      components: z.array(
        z.object({
          kind: z.enum([
            "container_rootfs",
            "container_logs",
            "disk_empty_dir",
            "pod_ephemeral_aggregate",
          ]),
          used_bytes: bytes,
          sampled_at: z.string().datetime({ offset: true }),
        }),
      ),
      missing_reasons: z.array(z.string()),
    })
    .nullable()
    .optional(),
  disk_full: z.boolean(),
  write_limit_bytes: bytes,
  assistant_arguments_json_limit_bytes: bytes.optional(),
  edit_file_limit_bytes: bytes.optional(),
  recovery: z.string(),
});
export type RemoteMachineUsage = z.infer<typeof remoteMachineUsageSchema>;
