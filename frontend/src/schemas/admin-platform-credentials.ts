import { z } from "zod";

export const platformCredentialFieldSchema = z
  .object({
    name: z.string().regex(/^[a-z][a-z0-9_]*$/),
    label: z.string(),
    secret: z.boolean(),
    help: z.string(),
    required: z.boolean(),
    numeric: z.boolean(),
    configured: z.boolean(),
    value: z.string().optional(),
  })
  .refine(
    (field) => !field.secret || field.value === undefined,
    "Secret values must not be returned",
  );

export const platformCredentialsSchema = z.object({
  provider: z.string(),
  label: z.string(),
  platform: z.string(),
  available: z.boolean(),
  fields: z.array(platformCredentialFieldSchema),
  setup_checklist: z.array(z.string()),
  callback_url: z.string().nullable(),
  webhook_verify_token: z.string().nullable(),
  updated_at: z.string().nullable(),
});

export const platformCredentialsListSchema = z.array(platformCredentialsSchema);
export const platformCredentialFormSchema = z.object({
  fields: z.record(z.string(), z.string().max(4096).nullable()),
});
export type PlatformCredentialForm = z.infer<
  typeof platformCredentialFormSchema
>;
