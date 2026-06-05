import { z } from "zod";

export const runtimeConfigSchema = z.object({
  api_base_url: z
    .string()
    .trim()
    .url("API base URL must be a valid URL")
    .transform((value) => value.replace(/\/+$/, "")),
  release_integrity: z.object({
    enabled: z.boolean(),
    manifest_url: z
      .string()
      .trim()
      .url("Release integrity manifest URL must be a valid URL")
      .nullable(),
    verification_ttl_secs: z.number().int().positive(),
  }),
});

export type RuntimeConfig = z.infer<typeof runtimeConfigSchema>;
