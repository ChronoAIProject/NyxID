import { z } from "zod";
export const telegramManagerSchema = z.object({
  manager_bot_id: z.string().regex(/^[1-9][0-9]*$/),
  username: z.string(),
  ready: z.boolean(),
  legacy_default: z.boolean(),
});
export const telegramManagerInputSchema = z.object({
  token: z.string().trim().min(1).max(512),
});
