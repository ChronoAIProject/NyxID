import { z } from "zod";
import { isHttpUrl } from "@/lib/http-origin";

export const runtimeConfigSchema = z.object({
  api_base_url: z
    .string()
    .trim()
    .url("API base URL must be a valid URL")
    .refine((value) => isHttpUrl(value, true), "API base URL must be an HTTP(S) origin")
    .transform((value) => new URL(value).origin),
  release_integrity: z.object({
    enabled: z.boolean(),
    manifest_url: z
      .string()
      .trim()
      .url("Release integrity manifest URL must be a valid URL")
      .refine((value) => isHttpUrl(value), "Manifest URL must use HTTP(S) without credentials or fragment")
      .nullable(),
    verification_ttl_secs: z.number().int().positive(),
  }),
});

export type RuntimeConfig = z.infer<typeof runtimeConfigSchema>;
