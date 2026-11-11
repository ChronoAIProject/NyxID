import { z } from "zod";
const count = z.number().int().nonnegative();
const rates = {
  completed: count,
  attempted: count,
  triggered: count,
  helpful: count,
  votes: count,
};
export const weeklyRealWorkSchema = z.object({
  threshold: z.number().int().min(1).max(1000),
  timezone: z.literal("Asia/Singapore"),
  weeks: z.array(
    z.object({
      week: z.string(),
      partial: z.boolean(),
      merchants: count,
      internal_owners: count,
      ...rates,
      internal_completed: count,
      internal_attempted: count,
      internal_triggered: count,
      internal_helpful: count,
      internal_votes: count,
      owners: z.array(
        z.object({ owner_id: z.string(), internal: z.boolean(), ...rates }),
      ),
      owners_truncated: z.boolean(),
      funnel: z.array(
        z.object({
          internal: z.boolean(),
          owners: count,
          has_nyxbot: count,
          connected_business: count,
          assigned_channel_work: count,
          first_real_work: count,
          median_seconds_to_first_work: z.number().nonnegative().nullable(),
        }),
      ),
    }),
  ),
});
export type WeeklyRealWork = z.infer<typeof weeklyRealWorkSchema>;
export const internalOwnersSchema = z.object({
  owner_ids: z.array(z.uuid()).max(5000),
});
export const internalOwnersFormSchema = z.object({
  owners: z
    .string()
    .max(200_000)
    .refine(
      (text) =>
        internalOwnersSchema.safeParse({ owner_ids: splitOwnerIds(text) })
          .success,
      "Enter up to 5,000 person or organization UUIDs, one per line.",
    ),
});
export function splitOwnerIds(text: string) {
  return [...new Set(text.split(/[\s,]+/).filter(Boolean))];
}
