import { z } from "zod";
import { platformProfileSchema } from "./platform-onboarding";

export const nativePlatformSchema = z.enum([
  "discord",
  "slack",
  "whatsapp",
  "lark",
  "feishu",
  "x",
]);
export const nativeTransportSchema = z.enum(["direct", "gateway"]);
const digest = z.string().regex(/^[0-9a-f]{64}$/);
const sha256 = z.string().regex(/^sha256:[0-9a-f]{64}$/);
export const gatewayChannelIdSchema = z
  .string()
  .max(512)
  .refine((value) => {
    const match = /^cmaeg1\.ch\.(0|[1-9]\d{0,2})\.([A-Za-z0-9_-]+)$/.exec(
      value,
    );
    if (!match || Number(match[1]) > 255) return false;
    const payload = match[2]!;
    try {
      const decoded = atob(payload.replaceAll("-", "+").replaceAll("_", "/"));
      return (
        decoded.length >= 28 &&
        btoa(decoded)
          .replaceAll("+", "-")
          .replaceAll("/", "_")
          .replace(/=+$/, "") === payload
      );
    } catch {
      return false;
    }
  }, "Invalid gateway channel ID");
export const nativeTransportsSchema = z
  .array(
    z.strictObject({
      platform: nativePlatformSchema,
      transports: z.array(nativeTransportSchema).min(1).max(2),
    }),
  )
  .max(6);
export const nativeSetupRecoverySchema = z.discriminatedUnion("kind", [
  z.strictObject({
    kind: z.literal("gateway"),
    record_id: z.uuid(),
    attempt_id: z.uuid(),
    revision: z.number().int().positive(),
    request_hash: sha256,
    credential_fingerprint: digest,
    remote_channel_id: gatewayChannelIdSchema.nullable(),
    remote_version: z.number().int().positive().nullable(),
    remote_ready: z.boolean(),
  }),
  z.strictObject({
    kind: z.literal("discord"),
    attempt_id: z.uuid(),
    application_id: z.string().regex(/^\d{1,20}$/),
    command_id: z
      .string()
      .regex(/^\d{1,20}$/)
      .nullable(),
    observed_schema_digest: digest.nullable(),
    schema_digest: digest,
    requested_by: z.uuid(),
    channel_bot_id: z.uuid(),
    credential_fingerprint: digest,
  }),
]);
export const nativeConversionResponseSchema = z.discriminatedUnion("status", [
  z.strictObject({
    status: z.literal("converted"),
    profile: platformProfileSchema,
  }),
  z.strictObject({
    status: z.literal("recovery"),
    review: nativeSetupRecoverySchema,
  }),
]);
export type NativeSetupRecovery = z.infer<typeof nativeSetupRecoverySchema>;
export type NativeTransport = z.infer<typeof nativeTransportSchema>;

export const nativeLifecycleReviewSchema = z.strictObject({
  channel_bot_id: z.uuid(),
  profile_id: z.uuid(),
  generation: z.number().int().positive().nullable(),
  reviewed_by: z.uuid(),
  snapshot_digest: digest,
});
export type NativeLifecycleReview = z.infer<typeof nativeLifecycleReviewSchema>;
