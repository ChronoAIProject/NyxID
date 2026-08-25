import { z } from "zod";

export const AUTH_DEVICE_ERROR_MESSAGES: Record<number, string> = {
  11200: "That code is no longer valid. Run `nyxid login --device` again.",
  11201: "This code has expired.",
  11204: "This login request was already denied.",
  11205: "This code was already used.",
  11206: "Too many attempts. Try again in a few minutes.",
  11207: "That code is no longer valid. Run `nyxid login --device` again.",
};

const AUTH_DEVICE_CONNECTION_ERROR_MESSAGE =
  "Couldn't reach NyxID. Check your connection and try again.";

export const userCodeSchema = z
  .string()
  .transform((value) => value.replace(/[-\s]/g, "").toUpperCase())
  .pipe(z.string().regex(/^[0-9A-HJKMNP-TV-Z]{8}$/, "Invalid code"));

export const approveBodySchema = z.object({
  user_code: userCodeSchema,
});

export const denyBodySchema = z.object({
  user_code: userCodeSchema,
});

export const approveResponseSchema = z.object({
  ok: z.literal(true),
});

export const requestBodySchema = z.object({
  client_label: z.string().trim().min(1).max(128),
  client_user_agent: z.string().trim().min(1).max(512),
});

export const requestResponseSchema = z.object({
  device_code: z.string().min(1),
  user_code: z.string().min(1),
  verification_uri: z.string().url(),
  verification_uri_complete: z.string().url(),
  expires_in: z.number().int().positive(),
  interval: z.number().int().positive(),
});

export const pollBodySchema = z.object({
  device_code: z.string().min(1),
});

export const pollWebResponseSchema = z.object({
  ok: z.literal(true),
});

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

export const previewResponseSchema = z.object({
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

export const errorEnvelopeSchema = z.object({
  error: z.string(),
  error_code: z.number(),
  message: z.string(),
});

export type ApproveAuthDeviceBody = z.output<typeof approveBodySchema>;
export type ApproveAuthDeviceResponse = z.infer<typeof approveResponseSchema>;
export type DenyAuthDeviceBody = z.output<typeof denyBodySchema>;
export type AuthDeviceRequestBody = z.output<typeof requestBodySchema>;
export type AuthDeviceRequestResponse = z.infer<typeof requestResponseSchema>;
export type AuthDevicePollBody = z.output<typeof pollBodySchema>;
export type AuthDevicePollWebResponse = z.infer<typeof pollWebResponseSchema>;
export type PreviewAuthDeviceResponse = z.infer<typeof previewResponseSchema>;
export type AuthDeviceErrorEnvelope = z.infer<typeof errorEnvelopeSchema>;

export function formatAuthDeviceUserCodeInput(value: string): string {
  const compact = value
    .replace(/[-\s]/g, "")
    .toUpperCase()
    .replace(/[^0-9A-Z]/g, "")
    .slice(0, 8);

  return compact.length > 4
    ? `${compact.slice(0, 4)}-${compact.slice(4)}`
    : compact;
}

export function friendlyAuthDeviceErrorMessage(error: unknown): string {
  const maybeApiError = error as {
    readonly errorCode?: unknown;
    readonly errorResponse?: unknown;
    readonly message?: unknown;
  };
  const parsedEnvelope = errorEnvelopeSchema.safeParse(
    maybeApiError.errorResponse,
  );
  const errorCode =
    typeof maybeApiError.errorCode === "number"
      ? maybeApiError.errorCode
      : parsedEnvelope.success
        ? parsedEnvelope.data.error_code
        : null;

  if (errorCode !== null && errorCode in AUTH_DEVICE_ERROR_MESSAGES) {
    return AUTH_DEVICE_ERROR_MESSAGES[errorCode] ?? "Device login failed.";
  }

  return AUTH_DEVICE_CONNECTION_ERROR_MESSAGE;
}

export function friendlyAuthDeviceStatusMessage(
  status: PreviewAuthDeviceResponse["status"],
): string | null {
  switch (status) {
    case "pending":
      return null;
    case "denied":
      return AUTH_DEVICE_ERROR_MESSAGES[11204] ?? null;
    case "expired":
      return AUTH_DEVICE_ERROR_MESSAGES[11201] ?? null;
    case "approved":
    case "delivered":
      return AUTH_DEVICE_ERROR_MESSAGES[11205] ?? null;
  }
}
