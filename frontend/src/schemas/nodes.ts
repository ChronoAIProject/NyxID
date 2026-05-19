import { z } from "zod";

const blankToUndefined = (value: unknown) =>
  typeof value === "string" && value.trim() === "" ? undefined : value;

export const createRegistrationTokenSchema = z.object({
  name: z
    .string()
    .min(1, "Name is required")
    .max(64, "Name must be 64 characters or less")
    .regex(
      /^[a-z0-9]([a-z0-9-]*[a-z0-9])?$/,
      "Lowercase alphanumeric and hyphens only, cannot start or end with hyphen",
    ),
  owner_user_id: z.string().min(1).nullable().optional(),
});

export const createBindingSchema = z.object({
  service_id: z.string().min(1, "Service is required"),
});

export const transferNodeSchema = z.object({
  new_owner_user_id: z.string().min(1, "Destination owner is required"),
});

export const nodePendingCredentialInjectionMethodSchema = z.enum([
  "header",
  "query-param",
  "path-prefix",
]);

export const pushNodeCredentialSchema = z.object({
  service_slug: z
    .string()
    .trim()
    .min(1, "Service slug is required")
    .max(64, "Service slug must be 64 characters or less")
    .regex(
      /^[a-z0-9]([a-z0-9-]*[a-z0-9])?$/,
      "Use lowercase letters, numbers, and hyphens only",
    ),
  injection_method: nodePendingCredentialInjectionMethodSchema,
  field_name: z
    .string()
    .trim()
    .min(1, "Field name is required")
    .max(128, "Field name must be 128 characters or less")
    .refine(
      (value) =>
        Array.from(value).every((char) => {
          const code = char.charCodeAt(0);
          return code >= 32 && code !== 127;
        }),
      "Field name cannot contain control characters",
    ),
  target_url: z.preprocess(
    blankToUndefined,
    z.string().trim().url("Target URL must be valid").optional(),
  ),
  label: z.preprocess(
    blankToUndefined,
    z.string().trim().min(1).max(128, "Label must be 128 characters or less").optional(),
  ),
});

export type CreateRegistrationTokenFormData = z.infer<
  typeof createRegistrationTokenSchema
>;
export type CreateBindingFormData = z.infer<typeof createBindingSchema>;
export type TransferNodeFormData = z.infer<typeof transferNodeSchema>;
export type PushNodeCredentialFormData = z.infer<
  typeof pushNodeCredentialSchema
>;
