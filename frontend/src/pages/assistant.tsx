import { useMemo } from "react";
import { useNavigate, useRouterState } from "@tanstack/react-router";
import { AssistantShell } from "@/components/assistant/assistant-shell";
import { AssistantSidebar } from "@/components/assistant/assistant-sidebar";
import { ChatComposer } from "@/components/assistant/chat-composer";
import { ChatThread } from "@/components/assistant/chat-thread";
import { ApprovalsView } from "@/components/assistant/approvals-view";
import { PluginsView } from "@/components/assistant/plugins-view";
import {
  useAssistantTurn,
  useCancelTurn,
  useConversation,
  useConversations,
  useCreateConversation,
  useDecideApproval,
  useSendMessage,
} from "@/hooks/use-assistant";
import { isTurnActive } from "@/types/assistant";

export function AssistantPage({
  view = "chat",
}: {
  readonly view?: "chat" | "plugins" | "approvals";
}) {
  const navigate = useNavigate();
  const selectedFromSearch = useRouterState({
    select: (state) => {
      const search = state.location.search as Record<string, unknown>;
      return typeof search.c === "string" ? search.c : undefined;
    },
  });
  const conversations = useConversations();
  const selectedId = useMemo(() => {
    const items = conversations.data ?? [];
    if (
      selectedFromSearch &&
      items.some((item) => item.id === selectedFromSearch)
    ) {
      return selectedFromSearch;
    }
    return items[0]?.id;
  }, [conversations.data, selectedFromSearch]);
  const history = useConversation(selectedId);
  const turn = useAssistantTurn(selectedId);
  const createConversation = useCreateConversation();
  const sendMessage = useSendMessage(selectedId);
  const cancelTurn = useCancelTurn(selectedId);
  const decideApproval = useDecideApproval(selectedId);

  function selectConversation(conversationId: string) {
    void navigate({
      to: "/assistant" as never,
      search: { c: conversationId } as never,
    });
  }

  async function createNewChat() {
    const conversation = await createConversation.mutateAsync();
    selectConversation(conversation.id);
  }

  const title =
    view === "plugins"
      ? "Plugins"
      : view === "approvals"
        ? "Approvals"
        : (history.data?.conversation.title ?? "New chat");
  const active = isTurnActive(turn.data?.status);
  const sidebar = (
    <AssistantSidebar
      conversations={conversations.data ?? []}
      activeConversationId={view === "chat" ? selectedId : undefined}
      activeView={view}
      creating={createConversation.isPending}
      turnActive={active}
      onNewChat={() => void createNewChat()}
      onSelect={selectConversation}
    />
  );

  if (view === "plugins" || view === "approvals") {
    return (
      <AssistantShell title={title} sidebar={sidebar}>
        {view === "plugins" ? <PluginsView /> : <ApprovalsView />}
      </AssistantShell>
    );
  }

  return (
    <AssistantShell title={title} sidebar={sidebar}>
      <div className="flex h-full min-h-0 flex-col bg-background">
        {history.isLoading ? (
          <div className="flex flex-1 items-center justify-center text-[12px] text-text-tertiary">
            Loading conversation...
          </div>
        ) : history.isError ? (
          <div className="flex flex-1 items-center justify-center px-6 text-center text-[12px] text-destructive">
            Failed to load this conversation.
          </div>
        ) : (
          <ChatThread
            messages={history.data?.messages ?? []}
            thinking={
              active &&
              history.data?.messages.at(-1)?.role !== "assistant"
            }
            onDecideApproval={(blockId, approved) =>
              decideApproval.mutateAsync({ blockId, approved })
            }
          />
        )}
        <ChatComposer
          active={active}
          sending={sendMessage.isPending}
          onSend={(content) =>
            sendMessage.mutateAsync(content).then(() => undefined)
          }
          onStop={() => cancelTurn.mutateAsync()}
        />
      </div>
    </AssistantShell>
  );
}
