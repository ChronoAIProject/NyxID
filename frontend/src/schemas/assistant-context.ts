import { z } from "zod";

const checkpointActionSchema = z.object({ label: z.string(), turn_id: z.string() });

/** The durable checkpoint a conversation resumes from (absent until a turn
 * settles after checkpoints shipped). Metadata and bounded excerpts only. */
export const assistantCheckpointSchema = z.object({
  turn_id: z.string(),
  status: z.string(),
  error_code: z.string().nullable().optional(),
  goal: z.string().nullable().optional(),
  completed_actions: z.array(checkpointActionSchema).default([]),
  failed_actions: z.array(checkpointActionSchema).default([]),
  output: z.string().nullable().optional(),
  last_successful_turn_id: z.string().nullable().optional(),
  last_successful_at: z.string().nullable().optional(),
  updated_at: z.string(),
});

export const assistantContextSchema = z.object({
  conversation_id: z.string(),
  agent_id: z.string().nullable().optional(),
  binding_state: z.string(),
  reset_reason: z.string().nullable().optional(),
  reset_at: z.string().nullable().optional(),
  active_turn_id: z.string().nullable().optional(),
  pending_actions: z.array(z.string()).default([]),
  checkpoint: assistantCheckpointSchema.nullable().optional(),
});
export type AssistantContext = z.infer<typeof assistantContextSchema>;
export type AssistantCheckpoint = z.infer<typeof assistantCheckpointSchema>;
