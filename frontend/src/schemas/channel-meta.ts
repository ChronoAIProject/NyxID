import { z } from "zod";

export const metaStartedSchema = z.object({
  session_id: z.uuid(),
  authorization_url: z.url(),
});
export const metaProgressSchema = z.object({
  session_id: z.uuid(),
  platform: z.enum(["messenger", "instagram"]),
  status: z.enum([
    "pending",
    "exchanging",
    "assets",
    "installing",
    "completed",
    "cancelled",
    "failed",
    "no_eligible_assets",
    "revoked",
  ]),
  bot_id: z.string().nullable(),
  selected_asset_id: z
    .string()
    .regex(/^\d{1,32}$/)
    .nullish(),
  subscription_state: z.string().nullish(),
  assets: z.array(
    z.object({ id: z.string().regex(/^\d{1,32}$/), name: z.string() }),
  ),
});
export const metaConnectedSchema = z.object({
  id: z.string(),
  platform: z.string(),
  label: z.string(),
  platform_bot_username: z.string(),
  status: z.string(),
  credential_source: z.string(),
  webhook_ingestion: z.boolean(),
  webhook_url: z.string(),
});
export type MetaProgress = z.infer<typeof metaProgressSchema>;
