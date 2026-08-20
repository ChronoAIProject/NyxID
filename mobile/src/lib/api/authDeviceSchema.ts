import { z } from "zod";

const authDevicePreviewSchema = z.object({
  client_label: z.string().nullable(),
  client_user_agent: z.string().nullable(),
  client_ip: z
    .string()
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
