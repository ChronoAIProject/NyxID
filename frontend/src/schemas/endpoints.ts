import { z } from "zod";

export const ENDPOINT_METHODS = [
  "GET",
  "POST",
  "PUT",
  "DELETE",
  "PATCH",
] as const;

export type EndpointMethod = (typeof ENDPOINT_METHODS)[number];

const optionalJsonString = z
  .string()
  .optional()
  .or(z.literal(""))
  .refine(
    (val) => {
      if (!val || val.trim() === "") return true;
      try {
        JSON.parse(val);
        return true;
      } catch {
        return false;
      }
    },
    { message: "Must be valid JSON" },
  );

export const createEndpointSchema = z.object({
  name: z
    .string()
    .min(1, "Name is required")
    .max(100, "Name must be at most 100 characters")
    .regex(
      /^[a-z][a-z0-9_]*$/,
      "Must start with a lowercase letter and contain only lowercase letters, digits, and underscores",
    ),
  description: z
    .string()
    .max(500, "Description must be at most 500 characters")
    .optional()
    .or(z.literal("")),
  method: z.enum(ENDPOINT_METHODS),
  path: z
    .string()
    .min(1, "Path is required")
    .max(2048, "Path must be at most 2048 characters")
    .regex(/^\//, "Path must start with /"),
  parameters: optionalJsonString,
  request_body_schema: optionalJsonString,
  response_description: z
    .string()
    .max(500, "Response description must be at most 500 characters")
    .optional()
    .or(z.literal("")),
});

export type CreateEndpointFormData = z.infer<typeof createEndpointSchema>;

export const updateEndpointSchema = z.object({
  name: z
    .string()
    .min(1, "Name is required")
    .max(100, "Name must be at most 100 characters")
    .regex(
      /^[a-z][a-z0-9_]*$/,
      "Must start with a lowercase letter and contain only lowercase letters, digits, and underscores",
    )
    .optional(),
  description: z
    .string()
    .max(500, "Description must be at most 500 characters")
    .optional()
    .or(z.literal("")),
  method: z.enum(ENDPOINT_METHODS).optional(),
  path: z
    .string()
    .min(1, "Path is required")
    .max(2048, "Path must be at most 2048 characters")
    .regex(/^\//, "Path must start with /")
    .optional(),
  parameters: optionalJsonString,
  request_body_schema: optionalJsonString,
  response_description: z
    .string()
    .max(500, "Response description must be at most 500 characters")
    .optional()
    .or(z.literal("")),
  is_active: z.boolean().optional(),
});

export type UpdateEndpointFormData = z.infer<typeof updateEndpointSchema>;
