import { z } from "zod";

export const codexConnectionSchema = z.object({
  account_id: z.uuid(),
  account_email: z.string(),
  provider_id: z.uuid(),
  provider_slug: z.literal("openai"),
  connection: z.object({ id: z.uuid(), state_version: z.number().int() }).nullable(),
  status: z.enum(["not_connected", "saved", "usable", "reconnect_required"]),
  service_id: z.uuid().nullable(),
  feature: z.literal("openai_responses"),
});

export type CodexConnection = z.infer<typeof codexConnectionSchema>;
