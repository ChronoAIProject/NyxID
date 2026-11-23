import {
  conversationModelsSchema,
  platformModelPageSchema,
  type ConversationModel,
} from "../../features/assistant/conversationModels";
import { localizedError } from "../../i18n/localizedError";
import { parseHistoryResponse } from "../../features/assistant/assistantHistoryCompat";
import { assistantTitleLanguage } from "../../features/assistant/titleLanguage";
import {
  requestAuthenticatedResponse,
  requestJson,
  submitChallengeDecisionRequest,
} from "./http";
import {
  recoverySchema,
  assistantCapabilitiesSchema,
  assistantSteeringSchema,
  assistantContextSchema,
  acknowledgementSchema,
  conversationIdSchema,
  conversationSchema,
  historySchema,
  indexSchema,
  type AssistantTurnEvent,
} from "../../features/assistant/assistantTypes";
import { readAssistantResponse } from "../../features/assistant/assistantStreamReader";
import {
  assistantTurnBody,
  type AssistantTurnInput,
} from "../../features/assistant/assistantTurnRequest";
import { assistantHistoryRequestPath } from "../../features/assistant/assistantHistoryTarget";
import { accessModeSchema } from "../../features/agents/agentContracts";
import {
  composerSettingsSchema,
  type ComposerAccessMode,
} from "../../features/assistant/composerSettings";

const ROOT = "/assistant/nyxagent";
const path = (id: string) =>
  `${ROOT}/conversations/${conversationIdSchema.parse(id)}`;

export const assistantApi = {
  async capabilities(id: string, signal?: AbortSignal) {
    return assistantCapabilitiesSchema.parse(
      await requestJson(`${path(id)}/capabilities`, { signal }),
    );
  },
  async steer(
    id: string,
    text: string,
    turnId: string,
    clientRequestId: string,
  ) {
    const result = assistantSteeringSchema.parse(
      await requestJson(`${path(id)}/steer`, {
        method: "POST",
        body: { text, turn_id: turnId, clientRequestId },
      }),
    );
    if (result.client_request_id !== clientRequestId)
      throw localizedError("The guidance response could not be confirmed.");
    return result;
  },
  async models(signal?: AbortSignal) {
    return conversationModelsSchema.parse(
      await requestJson(`${ROOT}/models`, { signal }),
    );
  },
  async platformModels(scope: { conversationId?: string; agentId?: string }, signal?: AbortSignal) {
    const query = new URLSearchParams({ limit: "100" });
    if (scope.conversationId)
      query.set("conversation_id", conversationIdSchema.parse(scope.conversationId));
    else if (scope.agentId) query.set("agent_id", scope.agentId);
    const rows: ConversationModel[] = [];
    const cursors = new Set<string>();
    for (;;) {
      const page = platformModelPageSchema.parse(
        await requestJson(`${ROOT}/platform-models?${query}`, { signal }),
      );
      for (const model of page.models) {
        rows.push({
          id: model.id,
          label: model.display_name,
          available:
            model.availability !== "unavailable" &&
            model.retail_prices.some(
              (price) =>
                price.available && Object.keys(price.variant).length === 0,
            ),
        });
      }
      if (!page.next_after) return rows;
      if (cursors.has(page.next_after))
        throw localizedError("Couldn’t load models.");
      cursors.add(page.next_after);
      query.set("after", page.next_after);
    }
  },
  async changeModel(id: string, model: string) {
    const result = conversationSchema.parse(
      await requestJson(`${path(id)}/model`, {
        method: "PATCH",
        body: { model },
      }),
    );
    if (result.id !== id || result.model !== model)
      throw localizedError("Could not confirm the model change.");
    return result;
  },
  async settings(signal?: AbortSignal) {
    return composerSettingsSchema.parse(
      await requestJson(`${ROOT}/settings`, { signal }),
    );
  },
  async updateAccessMode(mode: ComposerAccessMode) {
    return composerSettingsSchema.parse(
      await requestJson(`${ROOT}/settings`, {
        method: "PUT",
        body: { access_mode: accessModeSchema.parse(mode) },
      }),
    );
  },
  async decideApproval(id: string, approved: boolean, idempotencyKey: string) {
    const result = await submitChallengeDecisionRequest(
      id,
      approved ? "APPROVE" : "DENY",
      undefined,
      idempotencyKey,
    );
    if (result.challenge_id !== id)
      throw localizedError("Approval request mismatch.");
    return result;
  },
  async list(cursor?: string, agentId?: string, signal?: AbortSignal) {
    const query = new URLSearchParams({ limit: "50" });
    if (cursor) query.set("cursor", cursor);
    if (agentId) query.set("agent_id", agentId);
    return parseHistoryResponse(
      indexSchema,
      await requestJson(`${ROOT}/conversations?${query}`, { signal }),
    );
  },
  async history(
    id: string,
    beforeSeq?: number,
    signal?: AbortSignal,
    acknowledgementId?: string,
  ) {
    const result = parseHistoryResponse(
      historySchema,
      await requestJson(
        assistantHistoryRequestPath(id, beforeSeq, acknowledgementId),
        { signal },
      ),
    );
    if (result.conversation.id !== id)
      throw localizedError("Assistant conversation mismatch.");
    return result;
  },
  async recovery(id: string, messageId: string) {
    return recoverySchema.parse(
      await requestJson(
        `${path(id)}/messages/${encodeURIComponent(messageId)}/recovery`,
      ),
    );
  },
  async context(id: string, signal?: AbortSignal) {
    const value = assistantContextSchema.parse(
      await requestJson(`${path(id)}/context`, { signal }),
    );
    if (value.conversation_id !== id)
      throw localizedError("Assistant context mismatch.");
    return value;
  },
  async resetContext(id: string) {
    const value = assistantContextSchema.parse(
      await requestJson(`${path(id)}/context/reset`, { method: "POST" }),
    );
    if (value.conversation_id !== id)
      throw localizedError("Assistant context mismatch.");
    return value;
  },
  async rename(id: string, title: string) {
    const clean = title.trim();
    if (!clean || [...clean].length > 200)
      throw localizedError("Use a title between 1 and 200 characters.");
    return conversationSchema.parse(
      await requestJson(path(id), { method: "PATCH", body: { title: clean } }),
    );
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
      throw localizedError("Assistant acknowledgement mismatch.");
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
      headers: { Accept: "text/event-stream", "Accept-Language": assistantTitleLanguage() },
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
    throw localizedError("The connection was interrupted. Refresh history before sending again.");
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
            localizedError("The connection was interrupted. Refresh history before sending again."),
          );
        }, milliseconds);
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}
