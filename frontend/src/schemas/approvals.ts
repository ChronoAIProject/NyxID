import { z } from "zod";

export const updateNotificationSettingsSchema = z.object({
  telegram_enabled: z.boolean(),
  approval_required: z.boolean(),
  approval_timeout_secs: z
    .number()
    .int()
    .min(10, "Minimum timeout is 10 seconds")
    .max(300, "Maximum timeout is 300 seconds"),
  grant_expiry_days: z
    .number()
    .int()
    .min(1, "Minimum expiry is 1 day")
    .max(365, "Maximum expiry is 365 days"),
});

export type UpdateNotificationSettingsFormData = z.infer<
  typeof updateNotificationSettingsSchema
>;
