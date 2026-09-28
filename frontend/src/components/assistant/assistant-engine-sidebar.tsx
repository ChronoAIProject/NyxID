import { useState, useSyncExternalStore, type ComponentProps } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate, useRouterState } from "@tanstack/react-router";
import { AssistantSidebar, type SidebarAgents } from "@/components/assistant/assistant-sidebar";
import { NewAgentDialog } from "@/components/assistant/nyxbot-agent-forms";
import { useFeature } from "@/hooks/use-feature-flag";
import { useNyxAgentAssistantChat } from "@/hooks/use-assistant-nyxagent";
import { selectedAgentOf, useNyxBotAgents } from "@/hooks/use-nyxbot-agents";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { chatHistoryApi } from "@/lib/assistant/chat-history-api";
import { directAssistantTransport } from "@/lib/assistant/direct-transport";
import { isDirectConversationId, isNyxAgentConversationId } from "@/lib/assistant/conversation-ids";
import { nyxAgentTransport } from "@/lib/assistant/nyxagent-transport";
import { parseAssistantSearch } from "@/lib/assistant/search";
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
  const navigate = useNavigate();
  const search = useRouterState({
    select: (state) => parseAssistantSearch(state.location.search as Record<string, unknown>),
  });
  const directEnabled = useFeature(FEATURE_FLAG.DIRECT_CHAT_ENGINE);
  const nyxagentEnabled = useFeature(FEATURE_FLAG.NYXAGENT_ENGINE);
  const agents = useNyxBotAgents(nyxagentEnabled);
  const [creatingAgent, setCreatingAgent] = useState(false);
  const openThread = search.draft ? undefined : search.c;
  const selectedAgent = selectedAgentOf(
    agents.data?.agents,
    nyxAgentTransport.getConversation(openThread)?.agent?.id,
    search.agent,
  );
  const nyx = useNyxAgentAssistantChat({
    enabled: nyxagentEnabled,
    onConversationAdopted: noop,
    threadsAgentId: selectedAgent?.id,
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
  // Chats from the earlier engines; NyxBot threads are listed under their agent.
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
  for (const row of props.conversations) rows.set(row.id, row);

  function goTo(next: { c?: string; draft?: true; agent?: string }) {
    void navigate({
      to: "/assistant" as never,
      search: { ...next, ...(search.mock ? { mock: 1 } : {}) } as never,
    });
  }

  const agentsModel: SidebarAgents | undefined = nyxagentEnabled
    ? {
        agents: agents.data?.agents ?? [],
        selectedAgentId: selectedAgent?.id,
        threads: nyx.conversations,
        threadsLoading: Boolean(selectedAgent) && !nyx.threadsLoaded,
        // An agent lands on its latest thread (or a new one when it has none).
        onSelectAgent: (agentId) => goTo({ agent: agentId }),
        onNewThread: (agentId) => goTo({ draft: true, agent: agentId }),
        onNewAgent: () => setCreatingAgent(true),
      }
    : undefined;

  return (
    <>
      <AssistantSidebar
        {...props}
        agents={agentsModel}
        notice={props.notice ?? (agents.error ? `Could not load agents. ${agents.error.message}` : undefined)}
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
      {creatingAgent ? (
        <NewAgentDialog
          onClose={() => setCreatingAgent(false)}
          onCreated={(created) => {
            setCreatingAgent(false);
            goTo({ c: created.home_conversation_id });
          }}
        />
      ) : null}
    </>
  );
}
