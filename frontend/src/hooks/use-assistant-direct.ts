import { useCallback, useMemo, useSyncExternalStore } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import {
  directAssistantTransport,
  type DirectConversationSettings,
  type DirectConversationHistory,
} from "@/lib/assistant/direct-transport";
import type {
  ChatMessage,
  ChatSessionState,
} from "@/lib/assistant/chat-types";
import {
  directEffortsSchema,
  directModelsSchema,
  directSkillsSchema,
} from "@/schemas/assistant-direct";

const directCatalogKeys = {
  skills: ["assistant", "direct", "skills"] as const,
  models: ["assistant", "direct", "models"] as const,
  efforts: ["assistant", "direct", "efforts"] as const,
};

export function useDirectSkills() {
  return useQuery({
    queryKey: directCatalogKeys.skills,
    queryFn: async () =>
      directSkillsSchema.parse(
        await api.get<unknown>("/assistant/direct/skills"),
      ),
    staleTime: Number.POSITIVE_INFINITY,
  });
}

export function useDirectModels() {
  return useQuery({
    queryKey: directCatalogKeys.models,
    queryFn: async () =>
      directModelsSchema.parse(
        await api.get<unknown>("/assistant/direct/models"),
      ),
    staleTime: Number.POSITIVE_INFINITY,
  });
}

export function useDirectEfforts() {
  return useQuery({
    queryKey: directCatalogKeys.efforts,
    queryFn: async () =>
      directEffortsSchema.parse(
        await api.get<unknown>("/assistant/direct/efforts"),
      ),
    staleTime: Number.POSITIVE_INFINITY,
  });
}

export function useDirectConversationSettings(
  conversationId: string | undefined,
  defaultModel: string | undefined,
): {
  readonly settings: DirectConversationSettings;
  readonly canUpdate: boolean;
  readonly setModel: (model: string) => void;
  readonly setSkill: (skillSlug: string | null) => void;
  readonly setEffort: (effort: string | null) => void;
} {
  if (defaultModel) {
    directAssistantTransport.seedDefaultModel(conversationId, defaultModel);
  }
  const settings = useSyncExternalStore(
    directAssistantTransport.subscribeSettings,
    () => directAssistantTransport.getSettings(conversationId),
    () => directAssistantTransport.getSettings(conversationId),
  );

  return {
    settings,
    canUpdate: directAssistantTransport.canUpdateSettings(conversationId),
    setModel: (model) => {
      directAssistantTransport.setModel(conversationId, model);
    },
    setSkill: (skillSlug) => {
      directAssistantTransport.setSkill(conversationId, skillSlug);
    },
    setEffort: (effort) => {
      directAssistantTransport.setEffort(conversationId, effort);
    },
  };
}

function directMessageContent(
  message: DirectConversationHistory["messages"][number],
): string {
  return message.blocks.map((block) => block.text).filter(Boolean).join("\n\n");
}

function directSession(
  history: DirectConversationHistory | null,
): ChatSessionState {
  if (!history) {
    return {
      clientId: "direct-draft",
      expectedTurnCount: 0,
      messages: [],
      status: "draft",
      title: "New chat",
    };
  }

  let messages: ChatMessage[] = history.messages.map((message) => ({
    id: message.id,
    role: message.role,
    content: directMessageContent(message),
    timestamp: Date.parse(message.created_at),
    status: "complete",
  }));
  const turn = history.activeTurn;
  const active = turn?.status === "running" || turn?.status === "waiting";

  if (active) {
    const tail = messages.at(-1);
    if (tail?.role === "assistant") {
      messages = [
        ...messages.slice(0, -1),
        { ...tail, status: "streaming", turnId: turn.turnId },
      ];
    } else {
      messages = [
        ...messages,
        {
          id: `${turn.turnId ?? history.conversation.id}-assistant-message`,
          role: "assistant",
          content: "",
          timestamp: Date.now(),
          status: "streaming",
          turnId: turn.turnId,
        },
      ];
    }
  } else if (turn?.status === "failed" || turn?.status === "blocked") {
    const error = turn.error?.message ?? "The direct model run failed.";
    const tail = messages.at(-1);
    if (tail?.role === "assistant") {
      messages = [
        ...messages.slice(0, -1),
        { ...tail, status: "error", error, turnId: turn.turnId },
      ];
    } else {
      messages = [
        ...messages,
        {
          id: `${turn.turnId ?? history.conversation.id}-assistant-error`,
          role: "assistant",
          content: "",
          timestamp: Date.now(),
          status: "error",
          error,
          turnId: turn.turnId,
        },
      ];
    }
  }

  return {
    clientId: history.conversation.id,
    conversationId: history.conversation.id,
    expectedTurnCount: messages.filter((message) => message.role === "user")
      .length,
    latestTurnId: turn?.turnId ?? undefined,
    messages,
    status: active
      ? "streaming"
      : turn?.status === "failed" || turn?.status === "blocked"
        ? "error"
        : turn?.status === "cancelled"
          ? "stopped"
          : "completed_text",
    title: history.conversation.title,
  };
}

export function useDirectAssistantChat({
  selectedConversationId,
  onConversationAdopted,
}: {
  readonly selectedConversationId?: string;
  readonly onConversationAdopted: (conversationId: string) => void;
}) {
  useSyncExternalStore(
    directAssistantTransport.subscribeState,
    directAssistantTransport.getRevision,
    directAssistantTransport.getRevision,
  );

  const history = selectedConversationId
    ? directAssistantTransport.getHistorySnapshot(selectedConversationId)
    : null;
  const session = useMemo(() => directSession(history), [history]);
  const conversations = directAssistantTransport.getConversationsSnapshot();

  const send = useCallback(
    async (content: string) => {
      let conversationId = selectedConversationId;
      if (!conversationId) {
        conversationId = (await directAssistantTransport.createConversation()).id;
        onConversationAdopted(conversationId);
      }
      await new Promise<void>((resolve, reject) => {
        try {
          directAssistantTransport.sendMessage(conversationId, content, (event) => {
            if (event.event === "turn.completed") resolve();
          });
        } catch (error) {
          reject(error instanceof Error ? error : new Error(String(error)));
        }
      });
    },
    [onConversationAdopted, selectedConversationId],
  );

  const stop = useCallback(async () => {
    if (selectedConversationId) {
      directAssistantTransport.cancelActiveTurn(selectedConversationId);
    }
  }, [selectedConversationId]);

  const deleteConversation = useCallback(async (conversationId: string) => {
    await directAssistantTransport.deleteConversation(conversationId);
  }, []);

  return {
    conversations,
    deleteConversation,
    isMissing: Boolean(selectedConversationId && !history),
    isStreaming: session.status === "streaming",
    send,
    session,
    stop,
  };
}
