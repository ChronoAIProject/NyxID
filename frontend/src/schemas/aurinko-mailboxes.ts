import { z } from "zod";
import { oauthAttemptNonceSchema } from "./oauth-popup";

export const AURINKO_PROTOCOL = "aurinko_account_code";
export const aurinkoProviderSchema = z.enum([
  "Google",
  "Office365",
  "Zoho",
  "IMAP",
  "EWS",
  "iCloud",
]);
export type AurinkoProvider = z.infer<typeof aurinkoProviderSchema>;
export const AURINKO_PROVIDERS: Readonly<Record<AurinkoProvider, string>> = {
  Google: "Google / Google Workspace",
  Office365: "Microsoft 365",
  Zoho: "Zoho Mail",
  IMAP: "IMAP / SMTP",
  EWS: "Exchange (EWS)",
  iCloud: "iCloud",
};

export const aurinkoMailboxSchema = z.object({
  connection_id: z.string().uuid(),
  service_id: z.string().uuid(),
  label: z.string(),
  service_type: aurinkoProviderSchema.nullish(),
  mailbox_address: z.string().nullish(),
  status: z.string(),
  is_active: z.boolean(),
});
export const aurinkoMailboxesSchema = z.object({
  mailboxes: z.array(aurinkoMailboxSchema),
});
export const aurinkoAuthorizationSchema = z.object({
  connection_id: z.string().uuid(),
  service_id: z.string().uuid(),
  authorization_url: z.string().url(),
  attempt_nonce: oauthAttemptNonceSchema,
});
export type AurinkoMailbox = z.infer<typeof aurinkoMailboxSchema>;
export type AurinkoAuthorization = z.infer<typeof aurinkoAuthorizationSchema>;
export interface AurinkoConnection {
  readonly connection_id: string;
  readonly service_id: string;
}
