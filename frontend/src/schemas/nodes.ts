import { z } from "zod";

export const createRegistrationTokenSchema = z.object({
  name: z
    .string()
    .min(1, "Name is required")
    .max(64, "Name must be 64 characters or less")
    .regex(
      /^[a-z0-9]([a-z0-9-]*[a-z0-9])?$/,
      "Lowercase alphanumeric and hyphens only, cannot start or end with hyphen",
    ),
});

export const createBindingSchema = z.object({
  service_id: z.string().min(1, "Service is required"),
});

export type CreateRegistrationTokenFormData = z.infer<
  typeof createRegistrationTokenSchema
>;
export type CreateBindingFormData = z.infer<typeof createBindingSchema>;
