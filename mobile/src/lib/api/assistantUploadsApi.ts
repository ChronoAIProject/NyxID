import { z } from "zod";
import { requestBinaryJson, requestJson } from "./http";
import type { RequestOptions } from "./authenticatedTransport";
import { ApiError } from "./ApiError";
import { isAttachmentBindRefusal } from "../../features/assistant/assistantTurnRequest";
import { localizedError } from "../../i18n/localizedError";
import {
  attachmentSchema,
  conversationIdSchema,
  type AssistantAttachment,
} from "../../features/assistant/assistantTypes";
import {
  assistantAttachmentPath,
  attachmentFileError,
  attachmentUploadHeaders,
  withAttachmentUploadDeadline,
  type AssistantUploadFile,
  type AssistantUploadScope,
} from "../../features/assistant/assistantUploads";

const draftSchema = z.object({ id: conversationIdSchema });

export const assistantUploadsApi = {
  async createDraft(agentId?: string, signal?: AbortSignal): Promise<string> {
    const draft = draftSchema.parse(
      await requestJson("/assistant/nyxagent/drafts", {
        method: "POST",
        body: agentId ? { agent_id: agentId } : {},
        signal,
      }),
    );
    return draft.id;
  },

  async upload(
    scope: AssistantUploadScope,
    file: AssistantUploadFile,
    bytes: Uint8Array,
    signal?: AbortSignal,
    onProgress?: (value: number) => void,
  ): Promise<AssistantAttachment> {
    const error = attachmentFileError(file, bytes.byteLength);
    if (error) throw new Error(error);
    const path = assistantAttachmentPath(scope);
    return withAttachmentUploadDeadline(
      async (uploadSignal) =>
        attachmentSchema.parse(
          await requestBinaryJson(path, bytes, {
            headers: attachmentUploadHeaders(file.name),
            signal: uploadSignal,
            onProgress,
          }),
        ),
      signal,
    );
  },

  async remove(
    scope: AssistantUploadScope,
    attachmentId: string,
    options?: Pick<RequestOptions, "expectedAccountId" | "authorityGuard" | "signal">,
  ): Promise<void> {
    try {
      await withAttachmentUploadDeadline(
        (signal) => requestJson(assistantAttachmentPath(scope, attachmentId), {
          method: "DELETE",
          ...options,
          signal,
        }),
        options?.signal,
        15_000,
        () => localizedError("The attachment could not be removed. Try again."),
      );
    } catch (cause) {
      // A pending-only DELETE cannot remove an attachment already bound to a
      // message. Its scoped 404 also covers a deleted or no longer visible chat.
      if (isAttachmentBindRefusal(cause)) return;
      if (cause instanceof ApiError && cause.hasErrorEnvelope &&
          cause.statusCode === 404 && cause.errorKey === "not_found" &&
          cause.errorCode === 1003 && cause.message ===
            (scope.kind === "groups" ? "Not found: Group not found" : "Not found: Conversation not found"))
        return;
      throw cause;
    }
  },
};
