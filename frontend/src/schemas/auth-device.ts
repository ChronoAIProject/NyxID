import { z } from "zod";

export const AUTH_DEVICE_ERROR_MESSAGES: Record<number, string> = {
  11200: "That code is no longer valid. Run `nyxid login --device` again.",
  11201: "This code has expired.",
  11205: "This code was already used.",
  11206: "Too many attempts. Try again in a few minutes.",
  11207: "That code is no longer valid. Run `nyxid login --device` again.",
};

export const userCodeSchema = z
  .string()
  .transform((value) => value.replace(/[-\s]/g, "").toUpperCase())
  .pipe(z.string().regex(/^[0-9A-HJKMNP-TV-Z]{8}$/, "Invalid code"));

export const approveBodySchema = z.object({
  user_code: userCodeSchema,
});

export const approveResponseSchema = z.object({
  ok: z.literal(true),
});

export const previewResponseSchema = z.object({
  client_label: z.string().nullable(),
  client_user_agent: z.string().nullable(),
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

  if (parsedEnvelope.success) {
    return parsedEnvelope.data.message;
  }

  return typeof maybeApiError.message === "string"
    ? maybeApiError.message
    : "Device login failed.";
}
