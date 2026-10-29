import { conversationIdSchema } from "./assistantTypes";
import { z } from "zod";

export type AssistantTurnInput = {
  conversationId?: string;
  agentId?: string;
  text: string;
  retryOf?: string;
  attachmentIds?: readonly string[];
};

export function assistantTurnBody(input: AssistantTurnInput) {
  const ids = input.attachmentIds ?? [];
  if (ids.length > 10 || new Set(ids).size !== ids.length)
    throw new Error("Choose up to 10 distinct attachments.");
  ids.forEach((id) => z.string().uuid().parse(id));
  if (
    (!input.text.trim() && !ids.length && !input.retryOf) ||
    [...input.text].length > 32768
  )
    throw new Error(
      "Enter a message or attach a file (up to 32,768 characters).",
    );
  if (ids.length && !input.conversationId)
    throw new Error("Upload files to this chat before sending.");
  if (input.retryOf && ids.length)
    throw new Error("Retry uses the original message's attachments.");
  if (input.conversationId) conversationIdSchema.parse(input.conversationId);
  return {
    ...(input.conversationId
      ? { conversation_id: input.conversationId }
      : {
          ...(input.agentId ? { agent_id: input.agentId } : {}),
        }),
    text: input.text,
    ...(input.retryOf ? { retry_of: input.retryOf } : {}),
    ...(ids.length ? { attachment_ids: [...ids] } : {}),
  };
}

export function restoreRefusedAssistantSend(
  cause: unknown,
  admitted: boolean,
): boolean {
  // A response/stream error after admission cannot free already-bound uploads.
  return (
    !admitted &&
    typeof cause === "object" &&
    cause !== null &&
    "statusCode" in cause &&
    typeof cause.statusCode === "number" &&
    cause.statusCode >= 400 &&
    cause.statusCode < 500
  );
}
