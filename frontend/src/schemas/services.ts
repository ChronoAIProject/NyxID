import { z } from "zod"

export const AUTH_TYPES = [
  "api_key",
  "oauth2",
  "basic",
  "bearer",
] as const

export type AuthType = (typeof AUTH_TYPES)[number]

export const createServiceSchema = z.object({
  name: z
    .string()
    .min(1, "Name is required")
    .max(64, "Name must be at most 64 characters"),
  base_url: z
    .string()
    .min(1, "Base URL is required")
    .url("Must be a valid URL"),
  auth_type: z.enum(AUTH_TYPES),
})

export type CreateServiceFormData = z.infer<typeof createServiceSchema>
