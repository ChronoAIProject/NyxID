import { localizedError } from "../../i18n/localizedError";
import { conversationIdSchema } from "./assistantTypes";
import { z } from "zod";
import { ApiError } from "../../lib/api/ApiError";

export type AssistantTurnInput = {
  conversationId?: string;
  agentId?: string;
  model?: string;
  text: string;
  retryOf?: string;
  /** The allowed card this turn resumes; the server records it as `continuation_of`. */
  acknowledgementId?: string;
  attachmentIds?: readonly string[];
};

export function assistantTurnBody(input: AssistantTurnInput) {
  const ids = input.attachmentIds ?? [];
  if (ids.length > 10 || new Set(ids).size !== ids.length)
    throw localizedError("Choose up to 10 distinct attachments.");
  ids.forEach((id) => z.string().uuid().parse(id));
  if (
    (!input.text.trim() && !ids.length && !input.retryOf) ||
    [...input.text].length > 32768
  )
    throw localizedError("Enter a message or attach a file (up to 32,768 characters).");
  if (ids.length && !input.conversationId)
    throw localizedError("Upload files to this chat before sending.");
  if (input.retryOf && ids.length)
    throw localizedError("Retry uses the original message's attachments.");
  if (input.conversationId) conversationIdSchema.parse(input.conversationId);
  if (input.acknowledgementId !== undefined) {
    z.string().uuid().parse(input.acknowledgementId);
    if (!input.conversationId)
      throw localizedError("A decision can only resume its own chat.");
  }
  return {
    ...(input.conversationId
      ? { conversation_id: input.conversationId }
      : {
          ...(input.agentId ? { agent_id: input.agentId } : {}),
          ...(input.model ? { model: input.model } : {}),
        }),
    text: input.text,
    ...(input.retryOf ? { retry_of: input.retryOf } : {}),
    ...(input.acknowledgementId
      ? { acknowledgement_id: input.acknowledgementId }
      : {}),
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

/** Bind failures are an atomic refusal. The response does not identify a row. */
export function isAttachmentBindRefusal(cause: unknown): boolean {
  return cause instanceof ApiError && cause.hasErrorEnvelope && (
    (cause.statusCode === 410 && cause.errorKey === "attachment_expired" &&
      cause.errorCode === 12101) ||
    (cause.statusCode === 404 && cause.errorKey === "not_found" && cause.errorCode === 1003 &&
      ["Not found: Attachment not found", "Not found: Attachment not found or expired"].includes(cause.message))
  );
}
