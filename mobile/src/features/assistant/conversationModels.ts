import { z } from "zod";

export const conversationModelsSchema = z.array(
  z.object({
    id: z.string().min(1),
    label: z.string(),
    available: z.boolean(),
    description: z.string().nullish(),
  }),
);
export type ConversationModel = z.infer<
  typeof conversationModelsSchema
>[number];

export const platformModelPageSchema = z.object({
  models: z.array(
    z.object({
      id: z.string().min(1),
      display_name: z.string(),
      availability: z.string(),
      retail_prices: z.array(
        z.object({
          available: z.boolean(),
          variant: z.record(z.string(), z.string()),
        }),
      ),
    }),
  ),
  next_after: z.string().nullable(),
});
