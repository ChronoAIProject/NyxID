import { z } from "zod";

export const connectionWebhookFormSchema = z.object({
  url: z
    .string()
    .trim()
    .url("Enter a valid URL")
    .refine(
      (value) => {
        try {
          return new URL(value).protocol === "https:";
        } catch {
          return false;
        }
      },
      { message: "Connection webhook URL must use HTTPS" },
    ),
});

export const connectionWebhookFieldsSchema = z.object({
  connection_webhook_url: z.string().url().nullable(),
  connection_webhook_enabled: z.boolean(),
});

export const connectionWebhookSecretResponseSchema = z.object({
  client_id: z.string().min(1),
  connection_webhook_url: z.string().url(),
  connection_webhook_enabled: z.boolean(),
  signing_secret: z.string().min(1),
});

export type ConnectionWebhookForm = z.infer<typeof connectionWebhookFormSchema>;
export type ConnectionWebhookSecretResponse = z.infer<
  typeof connectionWebhookSecretResponseSchema
>;
