import { z } from "zod";
import { decodeBase64UrlNoPad, decodeBase64UrlNoPadExact, MAX_CIPHERTEXT_SIZE } from "@/lib/crypto";

export const MAX_REMOTE_CREDENTIAL_PLAINTEXT_SIZE =
  MAX_CIPHERTEXT_SIZE - 16;
export const MAX_FAN_OUT_TARGETS = 10;
export const MAX_FAN_OUT_CIPHERTEXT_TOTAL_SIZE =
  MAX_FAN_OUT_TARGETS * MAX_CIPHERTEXT_SIZE;

const optionalTrimmedString = z
  .string()
  .trim()
  .optional()
  .transform((value) => (value === "" ? undefined : value));

export const createRegistrationTokenSchema = z.object({
  name: z
    .string()
    .min(1, "Name is required")
    .max(64, "Name must be 64 characters or less")
    .regex(
      /^[a-z0-9]([a-z0-9-]*[a-z0-9])?$/,
      "Lowercase alphanumeric and hyphens only, cannot start or end with hyphen",
    ),
  owner_user_id: z.string().min(1).nullable().optional(),
});

export const createBindingSchema = z.object({
  service_id: z.string().min(1, "Service is required"),
});

export const transferNodeSchema = z.object({
  new_owner_user_id: z.string().min(1, "Destination owner is required"),
});

export const nodePendingCredentialInjectionMethodSchema = z.enum([
  "header",
  "query-param",
  "path-prefix",
]);

export const pushNodeCredentialSchema = z.object({
  service_slug: z
    .string()
    .trim()
    .min(1, "Service slug is required")
    .max(64, "Service slug must be 64 characters or less")
    .regex(
      /^[a-z0-9]([a-z0-9-]*[a-z0-9])?$/,
      "Use lowercase letters, numbers, and hyphens only",
    ),
  injection_method: nodePendingCredentialInjectionMethodSchema,
  field_name: z
    .string()
    .trim()
    .min(1, "Field name is required")
    .max(128, "Field name must be 128 characters or less")
    .refine(
      (value) =>
        Array.from(value).every((char) => {
          const code = char.charCodeAt(0);
          return code >= 32 && code !== 127;
        }),
      "Field name cannot contain control characters",
    ),
  target_url: optionalTrimmedString.pipe(
    z.string().url("Target URL must be valid").optional(),
  ),
  label: optionalTrimmedString.pipe(
    z
      .string()
      .min(1)
      .max(128, "Label must be 128 characters or less")
      .optional(),
  ),
  remote_crypto: z.literal(true).default(true),
});

export const pushNodeCredentialFanOutSchema = pushNodeCredentialSchema.extend({
  owner_user_id: z.string().min(1, "Owner is required"),
  service_id: z.string().min(1, "Service is required"),
});

const ciphertextEnvelopeSchema = z
  .object({
    version: z.literal("v1"),
    admin_pubkey: z.string().min(1),
    nonce: z.string().min(1),
    ciphertext: z.string().min(1),
  })
  .superRefine((value, ctx) => {
    try {
      decodeBase64UrlNoPadExact(value.admin_pubkey, "admin_pubkey", 32);
    } catch (err) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        path: ["admin_pubkey"],
        message: err instanceof Error ? err.message : "Invalid admin_pubkey",
      });
    }
    try {
      decodeBase64UrlNoPadExact(value.nonce, "nonce", 24);
    } catch (err) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        path: ["nonce"],
        message: err instanceof Error ? err.message : "Invalid nonce",
      });
    }
    try {
      const ciphertext = decodeBase64UrlNoPad(value.ciphertext, "ciphertext");
      if (ciphertext.length > MAX_CIPHERTEXT_SIZE) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          path: ["ciphertext"],
          message: `Ciphertext must be ${String(MAX_CIPHERTEXT_SIZE)} bytes or less.`,
        });
      }
    } catch (err) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        path: ["ciphertext"],
        message: err instanceof Error ? err.message : "Invalid ciphertext",
      });
    }
  });

export const integrityVerificationSchema = z.discriminatedUnion("mode", [
  z.object({
    mode: z.literal("admin_verified"),
    fingerprint_sha384_hex: z.string().regex(/^[0-9a-f]{96}$/),
    verified_at: z.string().datetime({ offset: true }),
    manifest_url_configured: z.literal(true),
  }),
  z.object({
    mode: z.literal("org_policy_opt_out"),
    fingerprint_sha384_hex: z.null(),
    verified_at: z.null(),
    manifest_url_configured: z.boolean(),
  }),
]);

export const pendingCredentialCiphertextRequestSchema =
  ciphertextEnvelopeSchema.extend({
    integrity_verification: integrityVerificationSchema.optional(),
  });

export const fanOutCiphertextItemSchema = ciphertextEnvelopeSchema.extend({
  node_id: z.string().min(1),
  generation: z.number().int().nonnegative(),
});

export const fanOutCiphertextsSchema = z
  .object({
    fan_out_revision: z.number().int().positive(),
    items: z
      .array(fanOutCiphertextItemSchema)
      .min(1)
      .max(MAX_FAN_OUT_TARGETS),
    integrity_verification: integrityVerificationSchema.optional(),
  })
  .superRefine((value, ctx) => {
    let total = 0;
    for (const [index, item] of value.items.entries()) {
      try {
        total += decodeBase64UrlNoPad(item.ciphertext, "ciphertext").length;
      } catch {
        continue;
      }
      if (total > MAX_FAN_OUT_CIPHERTEXT_TOTAL_SIZE) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          path: ["items", index, "ciphertext"],
          message: `Total fan-out ciphertext bytes must be ${String(MAX_FAN_OUT_CIPHERTEXT_TOTAL_SIZE)} or less.`,
        });
        break;
      }
    }
  });

export const acceptNodeCredentialSecretSchema = z
  .instanceof(Uint8Array)
  .refine((value) => value.length > 0, "Credential value is required.")
  .refine(
    (value) => value.length <= MAX_REMOTE_CREDENTIAL_PLAINTEXT_SIZE,
    `Credential value must be ${String(MAX_REMOTE_CREDENTIAL_PLAINTEXT_SIZE)} bytes or less.`,
  );

export type CreateRegistrationTokenFormData = z.infer<
  typeof createRegistrationTokenSchema
>;
export type CreateBindingFormData = z.infer<typeof createBindingSchema>;
export type TransferNodeFormData = z.infer<typeof transferNodeSchema>;
export type PushNodeCredentialFormData = z.infer<
  typeof pushNodeCredentialSchema
>;
export type PushNodeCredentialFanOutFormData = z.infer<
  typeof pushNodeCredentialFanOutSchema
>;
export type PushNodeCredentialFormInput = z.input<
  typeof pushNodeCredentialSchema
>;
export type AcceptNodeCredentialSecretData = z.infer<
  typeof acceptNodeCredentialSecretSchema
>;
export type FanOutCiphertextsData = z.infer<typeof fanOutCiphertextsSchema>;
