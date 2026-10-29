import { z } from "zod";
import { requestBinaryJson, requestJson } from "./http";
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
    signal?: AbortSignal,
  ): Promise<void> {
    await requestJson(assistantAttachmentPath(scope, attachmentId), {
      method: "DELETE",
      signal,
    });
  },
};
