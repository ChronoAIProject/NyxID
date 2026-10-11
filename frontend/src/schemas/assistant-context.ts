import { z } from "zod";
export const assistantContextSchema = z.object({
  conversation_id: z.string().optional().default(""), goal: z.string().nullable().optional(),
  completed_actions: z.array(z.string()), pending_actions: z.array(z.string()),
  failed_actions: z.array(z.string()).default([]), last_successful_turn: z.string().nullable().optional(),
  reset_reason: z.string().nullable().optional(), binding_state: z.string().optional().default("unknown"),
});
export type AssistantContext = z.infer<typeof assistantContextSchema>;
