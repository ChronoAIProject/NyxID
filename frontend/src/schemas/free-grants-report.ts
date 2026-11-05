import { z } from "zod";
import { creditsSchema } from "./credits";

export const freeGrantsReportSchema = z.object({
  window: z.object({ from: z.string(), to: z.string() }),
  generated_at: z.string(),
  issued: creditsSchema,
  grants: z.number(),
  total_accounts: z.number(),
  page: z.number(),
  per_page: z.number(),
  accounts: z.array(
    z.object({
      owner_id: z.string(),
      label: z.string(),
      account_kind: z.string(),
      issued: creditsSchema,
      grants: z.number(),
    }),
  ),
});
