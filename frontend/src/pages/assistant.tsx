import { useMemo } from "react";
import { useNavigate, useRouterState } from "@tanstack/react-router";
import { toast } from "sonner";
import { AssistantShell } from "@/components/assistant/assistant-shell";
import { AssistantSidebar } from "@/components/assistant/assistant-sidebar";
import { ChatComposer } from "@/components/assistant/chat-composer";
import { ChatThread } from "@/components/assistant/chat-thread";
import { ApprovalsView } from "@/components/assistant/approvals-view";
import { PluginsView } from "@/components/assistant/plugins-view";
import {
  describeSendFailure,
  useAssistantTurn,
  useCancelTurn,
  useConversation,
  useConversations,
  useDecideApproval,
  useDeleteConversation,
  useSendMessage,
} from "@/hooks/use-assistant";
import { isTurnActive } from "@/types/assistant";

const NEW_CHAT_SEARCH_VALUE = "new";

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
    if (selectedFromSearch === NEW_CHAT_SEARCH_VALUE) return undefined;
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
  const sendMessage = useSendMessage(selectedId);
  const cancelTurn = useCancelTurn(selectedId);
  const decideApproval = useDecideApproval(selectedId);
  const deleteConversation = useDeleteConversation();

  function selectConversation(conversationId: string) {
    void navigate({
      to: "/assistant" as never,
      search: { c: conversationId } as never,
    });
  }

  function createNewChat() {
    void navigate({
      to: "/assistant" as never,
      search: { c: NEW_CHAT_SEARCH_VALUE } as never,
    });
  }

  // Deleting the URL-addressed conversation must also drop `?c=`, or the
  // stale id lingers in the address bar and re-selects nothing on reload;
  // selection then falls back to the newest remaining chat. Failures stay
  // in the row's popover (still open) with the reason said out loud.
  async function handleDelete(conversationId: string) {
    try {
      await deleteConversation.mutateAsync(conversationId);
      if (conversationId === selectedFromSearch) {
        void navigate({ to: "/assistant" as never, search: {} as never });
      }
    } catch (error) {
      toast.error("Could not delete the chat", {
        description:
          error instanceof Error && error.message
            ? error.message
            : "The assistant backend did not respond. Try again.",
      });
    }
  }

  // First send from the "New chat" empty state has no conversation yet; the
  // mutation auto-creates one and this follows the navigation to it. A send
  // that fails before any turn exists must be said out loud — the composer
  // restores the text, and without the toast the button just looks dead.
  async function handleSend(content: string) {
    try {
      const sent = await sendMessage.mutateAsync(content);
      if (sent.conversationId !== selectedId) {
        selectConversation(sent.conversationId);
      }
    } catch (error) {
      const { message, description } = describeSendFailure(error);
      toast.error(message, { description });
      throw error;
    }
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
      creating={false}
      deletingId={
        deleteConversation.isPending ? deleteConversation.variables : undefined
      }
      onNewChat={createNewChat}
      onSelect={selectConversation}
      onDelete={(conversationId) => void handleDelete(conversationId)}
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
            streaming={
              active &&
              history.data?.messages.at(-1)?.role === "assistant"
            }
            onDecideApproval={(blockId, approved) =>
              decideApproval.mutateAsync({ blockId, approved })
            }
          />
        )}
        <ChatComposer
          active={active}
          sending={sendMessage.isPending}
          onSend={handleSend}
          onStop={() => cancelTurn.mutateAsync()}
        />
      </div>
    </AssistantShell>
  );
}
