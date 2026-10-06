import { z } from "zod";

export const MAX_ORDERED_SERVICES = 200;
export const MAX_EXPECTED_VERSION = Number.MAX_SAFE_INTEGER - 1;
const serviceId = z
  .uuidv4()
  .refine((id) => id === id.toLowerCase(), "Use canonical UUID v4 IDs");
const ordered = z
  .array(serviceId)
  .max(MAX_ORDERED_SERVICES)
  .refine((ids) => new Set(ids).size === ids.length, "Duplicate service IDs");
export const servicePreferenceRequestSchema = z.strictObject({
  ordered,
  expected_version: z.number().int().min(0).max(MAX_EXPECTED_VERSION),
});
export const servicePreferenceResponseSchema = z.strictObject({
  ordered,
  version: z.number().int().min(0).max(Number.MAX_SAFE_INTEGER),
  updated_at: z.string().nullable(),
});
export type ServicePreference = z.infer<typeof servicePreferenceResponseSchema>;
export type ServicePreferenceRequest = z.infer<
  typeof servicePreferenceRequestSchema
>;
