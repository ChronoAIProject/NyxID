import { z } from "zod";

/** Only public text blocks are retained; arbitrary tool payloads are not cached. */
export function cmaPublicText(value: unknown): string {
  if (typeof value === "string") return value.slice(0, 128 * 1024);
  if (!Array.isArray(value)) return "";
  return value
    .slice(0, 100)
    .flatMap((part: unknown) => {
      if (!part || typeof part !== "object") return [];
      const block = part as Record<string, unknown>;
      return ["text", "input_text", "output_text"].includes(
        String(block.type),
      ) && typeof block.text === "string"
        ? [block.text]
        : [];
    })
    .join("\n")
    .slice(0, 128 * 1024);
}

export const cmaAgentSchema = z.object({
  id: z.string(),
  cma_agent_id: z.string(),
  label: z.string(),
  status: z.string(),
  sandbox_id: z.string().nullable(),
  cma_sandbox_id: z.string().nullable(),
  cma_workspace_id: z.string().nullable().optional(),
  cma_worktree_id: z.string().nullable().optional(),
  last_observed: z.string().nullable(),
  available: z.boolean(),
  unavailable_reason: z.string().nullable(),
});
export const cmaTranscriptMessageSchema = z.object({
  id: z.string().nullable(),
  sequence: z.number().int().nonnegative().nullable(),
  role: z.string(),
  content: z.unknown().transform(cmaPublicText),
  created_at: z.string().nullable(),
});
export const cmaTranscriptSchema = z.object({
  messages: z.array(cmaTranscriptMessageSchema).max(10000),
  turns: z
    .array(
      z.object({
        turn_sequence: z.number().int().positive(),
        started_at: z.string().nullable(),
        status: z.enum([
          "active",
          "completed",
          "failed",
          "interrupted",
          "unknown",
        ]),
        messages: z.array(cmaTranscriptMessageSchema).max(10000),
      }),
    )
    .max(10000),
  next_cursor: z.string().max(1024).nullable(),
});
export const cmaEventSchema = z.object({
  id: z.string().nullable(),
  sequence: z.number().int().nonnegative().nullable(),
  cursor: z.string().max(1024).nullish(),
  message_sequence: z.number().int().nonnegative().nullish(),
  type: z.string(),
  status: z.string().nullable(),
  message: z
    .string()
    .max(128 * 1024)
    .nullable(),
  created_at: z.string().nullable(),
  response_id: z.string().max(256).nullish(),
  turn_sequence: z.number().int().positive().nullish(),
});
export const cmaEventSnapshotSchema = z.object({
  events: z.array(cmaEventSchema).max(1000),
  next_cursor: z.string().max(1024).nullable(),
  through_cursor: z.string().max(1024).nullish(),
});
export const cmaResponseSchema = z.object({
  id: z.string(),
  agent_id: z.string(),
  status: z.string(),
  output: z
    .array(
      z.object({
        id: z.string(),
        type: z.string(),
        role: z.string().optional(),
        status: z.string().optional(),
        name: z.string().optional(),
        content: z.unknown().transform(cmaPublicText),
      }),
    )
    .nullable(),
});
export const cmaDesktopSchema = z.object({
  available: z.boolean(),
  reason: z.string().nullable(),
  shared: z.boolean(),
  width: z.number().positive().nullable(),
  height: z.number().positive().nullable(),
  websocket_path: z.string().nullable(),
});
export type CmaAgent = z.infer<typeof cmaAgentSchema>;
export type CmaTranscriptMessage = z.infer<typeof cmaTranscriptMessageSchema>;
export type CmaTranscriptTurn = z.infer<
  typeof cmaTranscriptSchema
>["turns"][number];
export type CmaResourceEvent = z.infer<typeof cmaEventSchema>;
