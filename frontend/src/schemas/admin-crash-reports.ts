import { z } from "zod";
export const crashPlatformSchema = z.enum(["web", "ios", "android"]);
export const crashKindSchema = z.enum([
  "native_crash",
  "js_fatal",
  "js_error",
  "unhandled_rejection",
  "render_error",
  "recoverable_error",
  "resource_error",
]);
export const crashReportSchema = z.object({
  id: z.string(),
  event_id: z.string(),
  captured_at: z.string(),
  platform: crashPlatformSchema,
  kind: crashKindSchema,
  exception_class: z.string(),
  native_signal: z.string().nullable(),
  app_version: z.string(),
  app_build: z.string(),
  artifact_id: z.string().nullable(),
  release_verified: z.boolean().optional(),
  os_version: z.string(),
  runtime: z.object({ name: z.string(), major: z.number() }).nullable(),
  js_frames: z.array(
    z.object({ asset: z.string(), line: z.number(), column: z.number() }),
  ),
  native_images: z.array(
    z.object({ uuid: z.string(), name: z.string(), size: z.number() }),
  ),
  native_frames: z.array(
    z.object({ image: z.number().nullable(), offset: z.number() }),
  ),
  environment: z.string(),
  severity: z.string(),
  fingerprint: z.string(),
  received_at: z.string(),
  expires_at: z.string(),
  export: z.object({
    status: z.string(),
    destination_id: z.string().nullable(),
    attempts: z.number(),
    last_error_code: z.string().nullable(),
    exported_at: z.string().nullable(),
    provider_event_id: z.string(),
  }),
});
export const crashListSchema = z.object({
  reports: z.array(crashReportSchema),
  cursor: z.string().nullable(),
});
export const crashVersionSchema = z.object({
  platform: crashPlatformSchema,
  environment: z.string(),
  app_version: z.string(),
  app_build: z.string(),
  artifact_id: z.string().nullable(),
  artifact_ids: z.array(z.string()).max(10),
  counts: z.record(z.string(), z.number()),
  total: z.number(),
  first_seen: z.string(),
  last_seen: z.string(),
});
export const crashSummarySchema = z.object({
  health: z.object({
    outcomes: z.record(z.string(), z.number()),
    export_statuses: z.record(z.string(), z.number()),
    destination_ids: z.array(z.string()),
    retention_days: z.number(),
    flag_enabled: z.boolean(),
    rollout: z.enum(["off", "killed", "global", "cohort"]),
    export_enabled: z.boolean(),
  }),
  versions: z.array(crashVersionSchema),
  unverified_releases: z.array(crashVersionSchema),
});
export type CrashReportDetail = z.infer<typeof crashReportSchema>;
export type CrashFilters = {
  platform?: string;
  app_version?: string;
  app_build?: string;
  kind?: string;
  fingerprint?: string;
  cursor?: string;
};
