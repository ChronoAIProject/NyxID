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
  client_kind: z
    .enum(["cli", "browser", "mobile", "unknown"])
    .nullable()
    .optional()
    .transform((value) => value ?? "unknown"),
  client_app: boundedNullableString(96),
  client_platform: boundedNullableString(96),
  same_ip_as_viewer: z
    .boolean()
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
