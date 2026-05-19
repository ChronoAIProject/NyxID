import { z } from "zod";

export const PLATFORM_OPTIONS = [
  "claude-code",
  "cursor",
  "codex",
  "openclaw",
  "generic",
] as const;

export type Platform = (typeof PLATFORM_OPTIONS)[number];

export const createBindingSchema = z.object({
  user_service_id: z.string().min(1, "Service is required"),
  user_api_key_id: z.string().min(1, "Credential is required"),
});

export type CreateBindingFormData = z.infer<typeof createBindingSchema>;

export const updateRateLimitSchema = z.object({
  rate_limit_per_second: z
    .number()
    .int("Must be a whole number")
    .min(1, "Must be at least 1")
    .max(10000, "Must be at most 10,000")
    .nullable(),
  rate_limit_burst: z
    .number()
    .int("Must be a whole number")
    .min(1, "Must be at least 1")
    .max(100000, "Must be at most 100,000")
    .nullable(),
});

export type UpdateRateLimitFormData = z.infer<typeof updateRateLimitSchema>;
