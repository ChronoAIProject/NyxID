import { z } from "zod";

export const MAX_ORDERED_SERVICES = 200;
export const MAX_EXPECTED_VERSION = Number.MAX_SAFE_INTEGER - 1;
export const serviceGroupKeySchema = z
  .string()
  .regex(
    /^catalog:[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/,
    "Use a canonical RFC-variant catalog UUID with version 1–8",
  );
const serviceId = z
  .uuidv4()
  .refine((id) => id === id.toLowerCase(), "Use canonical UUID v4 IDs");
const ordered = z
  .array(serviceId)
  .max(MAX_ORDERED_SERVICES)
  .refine((ids) => new Set(ids).size === ids.length, "Duplicate service IDs");
export const servicePreferenceGroupRequestSchema = z.strictObject({
  ordered,
  expected_version: z.number().int().min(0).max(MAX_EXPECTED_VERSION),
});
export const releaseHiddenPreferenceRequestSchema =
  servicePreferenceGroupRequestSchema.pick({ expected_version: true });
export const servicePreferenceResponseSchema = z.strictObject({
  groups: z
    .array(z.strictObject({ group: serviceGroupKeySchema, ordered }))
    .refine(
      (groups) =>
        new Set(groups.map(({ group }) => group)).size === groups.length,
      "Duplicate groups",
    ),
  version: z.number().int().min(0).max(Number.MAX_SAFE_INTEGER),
  updated_at: z.string().nullable(),
});
export type ServicePreference = z.infer<typeof servicePreferenceResponseSchema>;
export type ServicePreferenceGroupRequest = z.infer<
  typeof servicePreferenceGroupRequestSchema
>;
