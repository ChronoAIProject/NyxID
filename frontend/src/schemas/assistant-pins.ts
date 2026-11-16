import { z } from "zod";

export const assistantPinsSchema = z.object({
  version: z.literal(1),
  ownerUserId: z.string().min(1).max(256),
  ids: z.array(z.string().min(1).max(256)).max(1000),
});
