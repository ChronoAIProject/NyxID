import { z } from "zod";

export const AUTH_TYPES = [
  "api_key",
  "oauth2",
  "basic",
  "bearer",
  "oidc",
] as const;

export type AuthType = (typeof AUTH_TYPES)[number];

// CR-6: Aligned with backend max length of 200 characters
export const createServiceSchema = z.object({
  name: z
    .string()
    .min(1, "Name is required")
    .max(200, "Name must be at most 200 characters"),
  description: z
    .string()
    .max(500, "Description must be at most 500 characters")
    .optional(),
  base_url: z
    .string()
    .min(1, "Base URL is required")
    .url("Must be a valid URL"),
  auth_type: z.enum(AUTH_TYPES),
});

export type CreateServiceFormData = z.infer<typeof createServiceSchema>;

export const updateServiceSchema = z.object({
  name: z
    .string()
    .min(1, "Name is required")
    .max(200, "Name must be at most 200 characters"),
  description: z
    .string()
    .max(500, "Description must be at most 500 characters")
    .optional()
    .or(z.literal("")),
  base_url: z.string().min(1, "Base URL is required").url("Must be a valid URL"),
  api_spec_url: z
    .string()
    .url("Must be a valid URL")
    .optional()
    .or(z.literal("")),
});

export type UpdateServiceFormData = z.infer<typeof updateServiceSchema>;

// SEC-1: Restrict redirect URIs to http/https schemes only
export const redirectUriSchema = z
  .string()
  .min(1, "URI is required")
  .url("Must be a valid URL")
  .refine(
    (val) => val.startsWith("https://") || val.startsWith("http://"),
    "URI must use https:// or http://"
  );
