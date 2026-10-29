import { z } from "zod";

function utf8Length(value: string): number {
  let length = 0;
  for (const character of value) {
    const point = character.codePointAt(0) ?? 0;
    length += point <= 0x7f ? 1 : point <= 0x7ff ? 2 : point <= 0xffff ? 3 : 4;
  }
  return length;
}

export const updateProfileSchema = z.object({
  display_name: z
    .string()
    .trim()
    .refine(
      (value) => utf8Length(value) <= 200,
      "Display name must be 200 bytes or less",
    )
    .optional(),
  avatar_url: z
    .string()
    .trim()
    .url("Enter a valid avatar URL")
    .refine(
      (value) => /^https?:\/\//.test(value),
      "Avatar URL must use https:// or http://",
    )
    .refine((value) => utf8Length(value) <= 2048, "Avatar URL is too long")
    .optional(),
});

export const sessionSchema = z.object({
  id: z.string().min(1),
  ip_address: z.string().nullable().optional(),
  user_agent: z.string().nullable().optional(),
  created_at: z.string(),
  expires_at: z.string(),
  // Never guess the current session from user-agent or IP metadata.
  is_current: z.boolean().default(false),
});

export const totpCodeSchema = z
  .string()
  .trim()
  .regex(/^\d{6}$/, "Enter the six-digit code from your authenticator");
export const mfaSetupSchema = z.object({
  factor_id: z.string().min(1),
  secret: z.string().regex(/^[A-Z2-7]+=*$/),
  qr_code_url: z.string().startsWith("otpauth://totp/"),
});
export const mfaConfirmSchema = z.object({
  message: z.string(),
  recovery_codes: z.array(z.string().min(1)).min(1),
});

export type AccountSession = z.infer<typeof sessionSchema>;
export type MfaEnrollment = z.infer<typeof mfaSetupSchema>;
export type UpdateProfileInput = z.infer<typeof updateProfileSchema>;
