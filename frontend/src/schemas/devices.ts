import { z } from "zod";

const USER_CODE_ALPHABET = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const USER_CODE_ALPHABET_SET = new Set(USER_CODE_ALPHABET.split(""));

export function formatDeviceUserCodeInput(value: string): string {
  const compact = value
    .toUpperCase()
    .split("")
    .filter((char) => USER_CODE_ALPHABET_SET.has(char))
    .slice(0, 12)
    .join("");

  return compact.match(/.{1,4}/g)?.join("-") ?? "";
}

export function normalizeDeviceUserCode(value: string): string {
  const compact = value
    .toUpperCase()
    .split("")
    .filter((char) => char !== "-" && !/\s/.test(char))
    .join("");

  if (
    compact.length !== 12 ||
    !compact.split("").every((char) => USER_CODE_ALPHABET_SET.has(char))
  ) {
    throw new Error("Enter a 12-character code using A-H, J-N, P-Z, and 2-9.");
  }

  return compact.match(/.{4}/g)?.join("-") ?? compact;
}

const userCodeSchema = z.string().transform((value, ctx) => {
  try {
    return normalizeDeviceUserCode(value);
  } catch (error) {
    ctx.addIssue({
      code: "custom",
      message:
        error instanceof Error
          ? error.message
          : "Enter a valid device user code.",
    });
    return z.NEVER;
  }
});

const orgIdSchema = z
  .string()
  .uuid("Select a valid organization")
  .nullable()
  .optional()
  .transform((value) => value ?? undefined);

const labelSchema = z
  .string()
  .optional()
  .transform((value) => {
    const trimmed = value?.trim() ?? "";
    return trimmed.length === 0 ? undefined : trimmed;
  })
  .pipe(
    z.string().max(200, "Label must be 200 characters or fewer").optional(),
  );

const onboardLabelSchema = z
  .string()
  .transform((value) => value.trim())
  .pipe(
    z
      .string()
      .min(1, "Label is required")
      .max(128, "Label must be 128 characters or fewer"),
  );

const wifiSsidSchema = z
  .string()
  .transform((value) => value.trim())
  .pipe(
    z
      .string()
      .min(1, "WiFi SSID is required")
      .max(32, "WiFi SSID must be 32 characters or fewer"),
  );

const wifiPasswordSchema = z
  .string()
  .min(8, "WiFi password must be at least 8 characters")
  .max(63, "WiFi password must be 63 characters or fewer");

export const approveDeviceFormSchema = z.object({
  user_code: userCodeSchema,
  org_id: orgIdSchema,
  label: labelSchema,
  default_services: z.array(z.string()).optional(),
});
export type ApproveDeviceFormData = z.input<typeof approveDeviceFormSchema>;
export type ApproveDeviceRequest = z.output<typeof approveDeviceFormSchema>;

export const approveDeviceResponseSchema = z.object({
  device_label: z.string(),
  hw_id: z.string(),
  api_key_id: z.string(),
  node_id: z.string(),
  owner_user_id: z.string(),
  org_id: z.string().nullable(),
});
export type ApproveDeviceResponse = z.infer<typeof approveDeviceResponseSchema>;

export const onboardDeviceFormSchema = z.object({
  org_id: orgIdSchema,
  label: onboardLabelSchema,
  wifi_ssid: wifiSsidSchema,
  wifi_password: wifiPasswordSchema,
  default_services: z.array(z.string()).optional(),
});
export type OnboardDeviceFormData = z.input<typeof onboardDeviceFormSchema>;
export type OnboardDeviceRequest = z.output<typeof onboardDeviceFormSchema>;

export const onboardDeviceResponseSchema = z.object({
  qr_payload: z.string().min(1),
  node_id: z.string(),
  api_key_id: z.string(),
  label: z.string(),
});
export type OnboardDeviceResponse = z.infer<typeof onboardDeviceResponseSchema>;

export function maskIdentifier(value: string): string {
  if (value.length <= 12) return value;
  return `${value.slice(0, 8)}...`;
}
