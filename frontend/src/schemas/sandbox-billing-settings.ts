import { z } from "zod";

export const sandboxBillingSettingsSchema = z.object({
  service_id: z.string().uuid(),
  authorized_producer_service_account_id: z.string().uuid().nullable(),
  revision: z.number().int().nonnegative(),
});
export type SandboxBillingSettings = z.infer<
  typeof sandboxBillingSettingsSchema
>;
export const sandboxProducerFormSchema = z.object({
  producer: z.union([z.literal(""), z.string().uuid()]),
});
export type SandboxProducerForm = z.infer<typeof sandboxProducerFormSchema>;
export type SandboxBillingSettingsUpdate = {
  readonly authorized_producer_service_account_id: string | null;
  readonly expected_revision: number;
};
