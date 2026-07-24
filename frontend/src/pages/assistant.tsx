import { useLayoutEffect, useMemo, useRef, useState } from "react";
import { useNavigate, useRouter, useRouterState } from "@tanstack/react-router";
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
  useCreateConversation,
  useDecideApproval,
  useDeleteConversation,
  useSendMessage,
} from "@/hooks/use-assistant";
import { parseAssistantSearch } from "@/lib/assistant/search";
import { isTurnActive } from "@/types/assistant";

export function AssistantPage({
  view = "chat",
}: {
  readonly view?: "chat" | "plugins" | "approvals";
}) {
  const navigate = useNavigate();
  const router = useRouter();
  // Two selectors returning primitives rather than one returning an object:
  // a fresh object identity would re-render the page on every router tick.
  const selectedFromSearch = useRouterState({
    select: (state) =>
      parseAssistantSearch(state.location.search as Record<string, unknown>).c,
  });
  const drafting = useRouterState({
    select: (state) =>
      parseAssistantSearch(state.location.search as Record<string, unknown>)
        .draft === true,
  });
  const conversations = useConversations();
  // The composer floats over the thread, so the thread has to know how tall it
  // currently is — it grows with the draft — to reserve the matching tail room
  // and place its fade.
  const composerRef = useRef<HTMLDivElement>(null);
  const [composerHeight, setComposerHeight] = useState(0);

  useLayoutEffect(() => {
    const element = composerRef.current;
    if (!element || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver((entries) => {
      setComposerHeight(entries[0]?.contentRect.height ?? 0);
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, [view]);

  const selectedId = useMemo(() => {
    const items = conversations.data ?? [];
    if (
      selectedFromSearch &&
      items.some((item) => item.id === selectedFromSearch)
    ) {
      return selectedFromSearch;
    }
    // A draft is a deliberately empty thread painted before its actor
    // exists, so it must NOT fall through to the newest chat — that
    // fallback is what makes a plain `/assistant` land on your last
    // conversation, and it would make "New chat" look like it did nothing.
    if (drafting) return undefined;
    return items[0]?.id;
  }, [conversations.data, drafting, selectedFromSearch]);
  const history = useConversation(selectedId);
  const turn = useAssistantTurn(selectedId);
  const createConversation = useCreateConversation();
  const sendMessage = useSendMessage(selectedId);
  const cancelTurn = useCancelTurn(selectedId);
  const decideApproval = useDecideApproval(selectedId);
  const deleteConversation = useDeleteConversation();

  function selectConversation(conversationId: string, replace = false) {
    void navigate({
      to: "/assistant" as never,
      search: { c: conversationId } as never,
      replace,
    });
  }

  /**
   * The click paints the empty thread immediately (`?draft`), then
   * provisions the actor in the background and swaps in `?c=<id>` once it
   * lands. Awaiting the create first made the button spin through three
   * sequential round trips — create, history, list — before anything moved,
   * and a failure resolved to nothing at all: no navigation, no message,
   * just a button that stopped spinning.
   *
   * `replace` on the swap keeps Back going where the user came from rather
   * than to the draft URL of a chat that now exists. The live router read
   * (not the render-time `drafting`) is deliberate: if they navigated on —
   * Plugins, another chat, or a send that already claimed this actor — the
   * late resolve must not yank them back.
   */
  async function createNewChat() {
    void navigate({
      to: "/assistant" as never,
      search: { draft: true } as never,
    });
    try {
      const conversation = await createConversation.mutateAsync();
      const stillDrafting =
        router.state.location.pathname === "/assistant" &&
        parseAssistantSearch(
          router.state.location.search as Record<string, unknown>,
        ).draft === true;
      if (stillDrafting) selectConversation(conversation.id, true);
    } catch (error) {
      // The draft stays put and the composer's auto-create retries the
      // provision on first send, so this is a recoverable failure — but an
      // unsaid one reads as the button being broken.
      toast.error("Could not start a new chat", {
        description:
          error instanceof Error && error.message
            ? error.message
            : "The assistant backend did not respond. Send a message to try again.",
      });
    }
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
      creating={createConversation.isPending}
      deletingId={
        deleteConversation.isPending ? deleteConversation.variables : undefined
      }
      onNewChat={() => void createNewChat()}
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
      <div className="relative flex h-full min-h-0 flex-col bg-background">
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
            bottomInset={composerHeight}
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
        <div ref={composerRef} className="absolute inset-x-0 bottom-0 z-10">
          <ChatComposer
            active={active}
            sending={sendMessage.isPending}
            onSend={handleSend}
            onStop={() => cancelTurn.mutateAsync()}
          />
        </div>
      </div>
    </AssistantShell>
  );
}
