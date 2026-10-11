import { z } from "zod";

export const windowDragEventSchema = z.discriminatedUnion("phase", [
  z
    .object({
      phase: z.literal("moving"),
      direction: z.enum(["left", "right"]),
    })
    .strict(),
  z
    .object({
      phase: z.literal("settled"),
      direction: z.null(),
    })
    .strict(),
]);

export type WindowDragEvent = z.infer<typeof windowDragEventSchema>;
