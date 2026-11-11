import { z } from "zod";
import { creditsSchema } from "./credits";

export const freeGrantsReportSchema = z.object({
  window: z.object({ from: z.string(), to: z.string() }),
  generated_at: z.string(),
  issued: creditsSchema,
  used: creditsSchema,
  grants: z.number(),
  total_accounts: z.number(),
  page: z.number(),
  per_page: z.number(),
  total_services: z.number(),
  service_page: z.number(),
  interval: z.literal("day"),
  services: z.array(
    z.object({ service_slug: z.string().nullable(), used: creditsSchema }),
  ),
  trend: z.array(
    z.object({
      start: z.string(),
      end: z.string(),
      issued: creditsSchema,
      used: creditsSchema,
    }),
  ),
  accounts: z.array(
    z.object({
      owner_id: z.string(),
      label: z.string(),
      account_kind: z.string(),
      issued: creditsSchema,
      used: creditsSchema,
      grants: z.number(),
    }),
  ),
});

export type FreeGrantsReport = z.infer<typeof freeGrantsReportSchema>;
