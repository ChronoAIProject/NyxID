import { ApiError } from "@/lib/api-client";
import {
  assistantHttp,
  assistantJson,
} from "@/lib/assistant/assistant-http";
import {
  ChatHistoryContractError,
  decodeChatConversationDetail,
  decodeChatHistoryIndex,
} from "@/lib/assistant/chat-history-decoders";
import type {
  ChatConversationDetail,
  ConversationMeta,
} from "@/lib/assistant/chat-types";

export class ChatHistoryApiError extends Error {
  readonly code?: string | number;
  readonly status: number;

  constructor(message: string, status: number, code?: string | number) {
    super(message);
    this.name = "ChatHistoryApiError";
    this.code = code;
    this.status = status;
  }
}

export type ChatStateCursor = {
  readonly afterStateVersion?: number;
  readonly turnId?: string;
};

function encodeSegment(value: string): string {
  return encodeURIComponent(value.trim());
}

function conversationPath(conversationId: string): string {
  return `/assistant/conversations/${encodeSegment(conversationId)}`;
}

function apiError(error: unknown): never {
  if (error instanceof ApiError) {
    throw new ChatHistoryApiError(
      error.message,
      error.status,
      error.errorCode >= 0 ? error.errorCode : error.errorResponse.error,
    );
  }
  throw error;
}

async function requestJson<T>(
  endpoint: string,
  decoder: (value: unknown) => T,
  signal?: AbortSignal,
): Promise<T> {
  try {
    return decoder(
      await assistantJson<unknown>(endpoint, {
        headers: { Accept: "application/json" },
        preserveSessionOn401: true,
        signal,
      }),
    );
  } catch (error) {
    if (error instanceof ChatHistoryContractError) throw error;
    return apiError(error);
  }
}

export const chatHistoryApi = {
  async listConversationMetas(signal?: AbortSignal): Promise<ConversationMeta[]> {
    const page = await requestJson(
      "/assistant/conversations",
      decodeChatHistoryIndex,
      signal,
    );
    if (page.nextCursor?.trim()) {
      throw new ChatHistoryContractError(
        "$index.nextCursor",
        "no pagination from the NyxID drained index",
      );
    }
    return page.conversations;
  },

  async loadConversation(
    conversationId: string,
    signal?: AbortSignal,
  ): Promise<ChatConversationDetail> {
    return requestJson(
      conversationPath(conversationId),
      decodeChatConversationDetail,
      signal,
    );
  },

  async loadConversationState(
    conversationId: string,
    cursor: ChatStateCursor = {},
    signal?: AbortSignal,
  ): Promise<unknown> {
    const query = new URLSearchParams();
    if (cursor.afterStateVersion !== undefined) {
      if (
        !Number.isSafeInteger(cursor.afterStateVersion) ||
        cursor.afterStateVersion < 0
      ) {
        throw new ChatHistoryContractError(
          "$cursor.afterStateVersion",
          "a non-negative safe integer",
        );
      }
      query.set("afterStateVersion", String(cursor.afterStateVersion));
    }
    if (cursor.turnId?.trim()) query.set("turnId", cursor.turnId.trim());
    const suffix = query.size ? `?${query.toString()}` : "";
    try {
      return await assistantJson<unknown>(
        `${conversationPath(conversationId)}/state${suffix}`,
        {
          headers: { Accept: "application/json" },
          preserveSessionOn401: true,
          signal,
        },
      );
    } catch (error) {
      return apiError(error);
    }
  },

  async deleteConversation(conversationId: string): Promise<void> {
    try {
      await assistantHttp(conversationPath(conversationId), {
        headers: { Accept: "application/json" },
        method: "DELETE",
        preserveSessionOn401: true,
      });
    } catch (error) {
      return apiError(error);
    }
  },
};

export { ChatHistoryContractError };
