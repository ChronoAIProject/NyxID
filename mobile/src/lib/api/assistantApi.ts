import { requestAuthenticatedResponse, requestJson } from "./http";
import {
  acknowledgementSchema,
  conversationIdSchema,
  conversationSchema,
  historySchema,
  indexSchema,
  modelListSchema,
  type AssistantTurnEvent,
} from "../../features/assistant/assistantTypes";
import { readAssistantResponse } from "../../features/assistant/assistantStreamReader";
import {
  assistantTurnBody,
  type AssistantTurnInput,
} from "../../features/assistant/assistantTurnRequest";
import { assistantHistoryRequestPath } from "../../features/assistant/assistantHistoryTarget";

const ROOT = "/assistant/nyxagent";
const path = (id: string) =>
  `${ROOT}/conversations/${conversationIdSchema.parse(id)}`;

export const assistantApi = {
  async list(cursor?: string, agentId?: string, signal?: AbortSignal) {
    const query = new URLSearchParams({ limit: "50" });
    if (cursor) query.set("cursor", cursor);
    if (agentId) query.set("agent_id", agentId);
    return indexSchema.parse(
      await requestJson(`${ROOT}/conversations?${query}`, { signal }),
    );
  },
  async history(
    id: string,
    beforeSeq?: number,
    signal?: AbortSignal,
    acknowledgementId?: string,
  ) {
    const result = historySchema.parse(
      await requestJson(
        assistantHistoryRequestPath(id, beforeSeq, acknowledgementId),
        { signal },
      ),
    );
    if (result.conversation.id !== id)
      throw new Error("Assistant conversation mismatch.");
    return result;
  },
  async models() {
    return modelListSchema.parse(await requestJson(`${ROOT}/models`));
  },
  async rename(id: string, title: string) {
    const clean = title.trim();
    if (!clean || [...clean].length > 200)
      throw new Error("Use a title between 1 and 200 characters.");
    return conversationSchema.parse(
      await requestJson(path(id), { method: "PATCH", body: { title: clean } }),
    );
  },
  async changeModel(id: string, model: string, signal?: AbortSignal) {
    const row = conversationSchema.parse(
      await requestJson(`${path(id)}/model`, {
        method: "PATCH",
        body: { model },
        signal,
      }),
    );
    if (row.id !== id || row.model !== model)
      throw new Error("The upload draft model could not be confirmed.");
    return row;
  },
  async delete(id: string) {
    await requestJson(path(id), { method: "DELETE" });
  },
  async stop(id: string) {
    await requestJson(`${path(id)}/stop`, { method: "POST" });
  },
  async decide(
    id: string,
    acknowledgementId: string,
    decision: "allow" | "deny",
  ) {
    const result = acknowledgementSchema.parse(
      await requestJson(
        `${path(id)}/acknowledgements/${encodeURIComponent(acknowledgementId)}`,
        { method: "POST", body: { decision } },
      ),
    );
    if (result.id !== acknowledgementId)
      throw new Error("Assistant acknowledgement mismatch.");
    return result;
  },
};

export type AssistantSendInput = AssistantTurnInput;

/** A disconnected subscription never replays the POST: NyxID continues the durable turn. */
export async function streamAssistantTurn(
  input: AssistantSendInput,
  onEvent: (event: AssistantTurnEvent) => void,
  controller: AbortController,
  onAdmitted?: () => void,
): Promise<void> {
  const body = assistantTurnBody(input);
  const response = await withDeadline(
    requestAuthenticatedResponse(`${ROOT}/turns`, {
      method: "POST",
      signal: controller.signal,
      headers: { Accept: "text/event-stream" },
      body,
    }),
    45000,
    controller,
  );
  onAdmitted?.();
  if (
    !response.headers.get("content-type")?.includes("text/event-stream") ||
    !response.body
  ) {
    throw new Error(
      "The connection was interrupted. Refresh history before sending again.",
    );
  }
  await readAssistantResponse(response.body, onEvent, controller);
}

async function withDeadline<T>(
  promise: Promise<T>,
  milliseconds: number,
  controller: AbortController,
): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      promise,
      new Promise<never>((_, reject) => {
        timer = setTimeout(() => {
          controller.abort();
          reject(
            new Error(
              "The connection was interrupted. Refresh history before sending again.",
            ),
          );
        }, milliseconds);
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}
