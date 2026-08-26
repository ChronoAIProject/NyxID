import { z } from "zod";

function boundedNullableString(maxLength: number) {
  return z
    .string()
    .nullable()
    .optional()
    .transform((value) => {
      if (value === null || value === undefined) return null;
      const cleaned = Array.from(value)
        .filter((character) => {
          const codePoint = character.codePointAt(0) ?? 0;
          return codePoint > 31 && !(codePoint >= 127 && codePoint <= 159);
        })
        .slice(0, maxLength)
        .join("")
        .trim();
      return cleaned || null;
    });
}

function nullableNumber<T extends z.ZodType<number>>(schema: T) {
  return schema
    .nullable()
    .optional()
    .transform((value) => value ?? null);
}

const authDevicePreviewSchema = z.object({
  client_label: boundedNullableString(64),
  client_user_agent: boundedNullableString(256),
  client_ip: boundedNullableString(64),
  client_ip_attribution: z
    .enum(["verified", "unverified", "unavailable"])
    .nullable()
    .optional()
    .transform((value) => value ?? "unavailable"),
  client_country: boundedNullableString(2),
  client_city: boundedNullableString(96),
  client_region: boundedNullableString(96),
  client_continent: boundedNullableString(2),
  client_ip_timezone: boundedNullableString(64),
  initiating_origin: boundedNullableString(256),
  initiating_origin_status: z
    .enum(["absent", "matched", "mismatched", "malformed", "non_http"])
    .nullable()
    .optional()
    .transform((value) => value ?? "absent"),
  client_kind: z
    .enum(["cli", "browser", "mobile", "unknown"])
    .nullable()
    .optional()
    .transform((value) => value ?? "unknown"),
  client_app: boundedNullableString(96),
  client_platform: boundedNullableString(96),
  client_model: boundedNullableString(96),
  client_form_factor: z
    .enum(["desktop", "mobile", "tablet", "unknown"])
    .nullable()
    .optional()
    .transform((value) => value ?? null),
  client_timezone: boundedNullableString(64),
  client_timezone_matches_ip: z
    .boolean()
    .nullable()
    .optional()
    .transform((value) => value ?? null),
  client_locale: boundedNullableString(35),
  client_screen_width: nullableNumber(z.number().int().positive().max(32_768)),
  client_screen_height: nullableNumber(z.number().int().positive().max(32_768)),
  client_device_pixel_ratio: nullableNumber(z.number().positive().max(16)),
  client_hardware_concurrency: nullableNumber(
    z.number().int().positive().max(1_024),
  ),
  client_device_memory: nullableNumber(z.number().positive().max(1_024)),
  same_ip_as_viewer: z
    .boolean()
    .nullable()
    .optional()
    .transform((value) => value ?? null),
  network_relation: z
    .enum(["same_ip", "same_network", "different_network"])
    .nullable()
    .optional()
    .transform((value) => value ?? null),
  seconds_remaining: z
    .number()
    .int()
    .nonnegative()
    .nullable()
    .optional()
    .transform((value) => value ?? null),
  initiated_at: z.string().datetime(),
  expires_at: z.string().datetime(),
  status: z.enum(["pending", "approved", "denied", "expired", "delivered"]),
});

export type AuthDevicePreview = z.infer<typeof authDevicePreviewSchema>;

export function parseAuthDevicePreview(value: unknown): AuthDevicePreview {
  return authDevicePreviewSchema.parse(value);
}
