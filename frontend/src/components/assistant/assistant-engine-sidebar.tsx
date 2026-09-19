import { useSyncExternalStore, type ComponentProps } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { AssistantSidebar } from "@/components/assistant/assistant-sidebar";
import { useFeature } from "@/hooks/use-feature-flag";
import { useNyxAgentAssistantChat } from "@/hooks/use-assistant-nyxagent";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { chatHistoryApi } from "@/lib/assistant/chat-history-api";
import { directAssistantTransport } from "@/lib/assistant/direct-transport";
import { isDirectConversationId, isNyxAgentConversationId } from "@/lib/assistant/conversation-ids";
import { useAuthStore } from "@/stores/auth-store";
import type { Conversation } from "@/types/assistant";

const noop = () => undefined;

export function AssistantEngineSidebar({
  engine,
  ...props
}: ComponentProps<typeof AssistantSidebar> & {
  readonly engine: "actor" | "direct" | "nyxagent";
}) {
  const userId = useAuthStore((state) => state.user?.id);
  const queryClient = useQueryClient();
  const directEnabled = useFeature(FEATURE_FLAG.DIRECT_CHAT_ENGINE);
  const nyxagentEnabled = useFeature(FEATURE_FLAG.NYXAGENT_ENGINE);
  const nyx = useNyxAgentAssistantChat({
    enabled: nyxagentEnabled,
    onConversationAdopted: noop,
  });
  const actorKey = ["assistant", "actor", userId, "sidebar"];
  const actors = useQuery({
    queryKey: actorKey,
    queryFn: ({ signal }) => chatHistoryApi.listConversationMetas(signal),
    enabled: engine !== "actor",
    retry: false,
  });
  useSyncExternalStore(
    directAssistantTransport.subscribeState,
    directAssistantTransport.getRevision,
    directAssistantTransport.getRevision,
  );
  const rows = new Map<string, Conversation>();
  for (const row of actors.data ?? []) {
    rows.set(row.id, {
      id: row.id,
      title: row.title,
      created_at: row.createdAt,
      last_message_at: row.updatedAt,
      message_count: row.messageCount,
    });
  }
  if (directEnabled) {
    for (const row of directAssistantTransport.getConversationsSnapshot()) {
      const turn = directAssistantTransport.getHistorySnapshot(row.id)?.activeTurn;
      rows.set(row.id, {
        ...row,
        active_turn:
          turn && ["running", "waiting"].includes(turn.status)
            ? { turn_id: turn.turnId ?? row.id, started_at: row.last_message_at }
            : null,
      });
    }
  }
  if (nyxagentEnabled) {
    for (const row of nyx.conversations) rows.set(row.id, row);
  }
  for (const row of props.conversations) rows.set(row.id, row);

  return (
    <AssistantSidebar
      {...props}
      conversations={[...rows.values()].sort((a, b) =>
        b.last_message_at.localeCompare(a.last_message_at),
      )}
      onRename={nyx.renameConversation}
      onDelete={async (id) => {
        if (isNyxAgentConversationId(id)) {
          await nyx.deleteConversation(id);
        } else if (isDirectConversationId(id)) {
          if (rows.get(id)?.active_turn) return;
          await directAssistantTransport.deleteConversation(id);
        } else if (engine !== "actor") {
          await chatHistoryApi.deleteConversation(id);
          await queryClient.invalidateQueries({ queryKey: actorKey });
        } else {
          await props.onDelete(id);
          return;
        }
        // Let the owning page perform its route cleanup after successful deletion.
        if (id === props.activeConversationId) props.onNewChat();
      }}
    />
  );
}
