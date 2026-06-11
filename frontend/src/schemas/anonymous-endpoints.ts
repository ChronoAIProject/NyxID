import { z } from "zod";

export const anonymousEndpointMethodSchema = z.enum([
  "GET",
  "POST",
  "PUT",
  "PATCH",
  "DELETE",
  "HEAD",
  "OPTIONS",
]);

export const anonymousEndpointRuleSchema = z.object({
  id: z.string().trim().min(1),
  enabled: z.boolean(),
  method: anonymousEndpointMethodSchema,
  path_pattern: z
    .string()
    .trim()
    .min(1, "Path pattern is required")
    .max(512, "Path pattern must be 512 characters or less")
    .transform((value) => (value.startsWith("/") ? value : `/${value}`))
    .refine((value) => !value.includes("\\") && !value.includes("\0"), {
      message: "Path pattern contains invalid characters",
    })
    .refine((value) => !value.includes("?") && !value.includes("#"), {
      message: "Path pattern must not include query strings or fragments",
    })
    .refine((value) => !value.includes("//"), {
      message: "Path pattern must not contain empty segments",
    })
    .refine(
      (value) =>
        value
          .split("/")
          .every((segment) => segment !== "." && segment !== ".."),
      { message: "Path pattern must not contain dot segments" },
    )
    .refine((value) => !value.includes("*") || value.endsWith("/**"), {
      message: "Wildcard must be a trailing /** segment",
    }),
  daily_quota: z.coerce
    .number()
    .int("Daily quota must be a whole number")
    .min(1, "Daily quota must be at least 1"),
});

export const anonymousEndpointCreateSchema = anonymousEndpointRuleSchema.omit({
  id: true,
});

export const anonymousEndpointUpdateSchema =
  anonymousEndpointCreateSchema.partial();

export type AnonymousEndpointRuleFormData = z.infer<
  typeof anonymousEndpointCreateSchema
>;
export type AnonymousEndpointRuleFormInput = z.input<
  typeof anonymousEndpointCreateSchema
>;
export type AnonymousEndpointRule = z.infer<typeof anonymousEndpointRuleSchema>;
export type AnonymousEndpointUpdateData = z.infer<
  typeof anonymousEndpointUpdateSchema
>;
