import { z } from "zod";

const metaId = z.string().trim().regex(/^\d{1,32}$/, "Enter the numeric ID from Meta.");
export const whatsappNativeSchema = z.object({
  meta_app_id: metaId,
  waba_id: metaId,
  dedicated_app: z.boolean().refine((value) => value, "A dedicated Meta app is required."),
});
export type WhatsappNativeForm = z.infer<typeof whatsappNativeSchema>;
