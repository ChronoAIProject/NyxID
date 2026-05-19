import { z } from "zod";

export const runtimeConfigSchema = z.object({
  api_base_url: z
    .string()
    .trim()
    .url("API base URL must be a valid URL")
    .transform((value) => value.replace(/\/+$/, "")),
});

export type RuntimeConfig = z.infer<typeof runtimeConfigSchema>;
