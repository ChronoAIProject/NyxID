import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useNavigate, useRouterState } from "@tanstack/react-router";
import { toast } from "sonner";
import { AssistantShell } from "@/components/assistant/assistant-shell";
import { AssistantSidebar } from "@/components/assistant/assistant-sidebar";
import { ChatComposer } from "@/components/assistant/chat-composer";
import { ChatThread } from "@/components/assistant/chat-thread";
import { ApprovalsView } from "@/components/assistant/approvals-view";
import { PluginsView } from "@/components/assistant/plugins-view";
import {
  describeHistoryError,
  describeSendFailure,
  useAssistantTurn,
  useCancelTurn,
  useConversation,
  useConversations,
  useDecideApproval,
  useDeleteConversation,
  useSendMessage,
  useTurnEpisode,
} from "@/hooks/use-assistant";
import { resolveAssistantConversationId } from "@/lib/assistant/conversation-resolution";
import { parseAssistantSearch } from "@/lib/assistant/search";
import { useAssistantContextStore } from "@/stores/assistant-context-store";
import { useAssistantDraftStore } from "@/stores/assistant-draft-store";
import { useAuthStore } from "@/stores/auth-store";
import { isTurnActive, type AssistantMessage } from "@/types/assistant";

/** Text of a user message's first text block, for the optimistic-echo check. */
function userMessageText(
  message: AssistantMessage | undefined,
): string | undefined {
  if (message?.role !== "user") return undefined;
  for (const block of message.blocks as unknown[]) {
    if (
      typeof block === "object" &&
      block !== null &&
      "type" in block &&
      block.type === "text" &&
      "text" in block &&
      typeof block.text === "string"
    ) {
      return block.text;
    }
  }
  return undefined;
}

function optimisticUserMessage(text: string): AssistantMessage {
  return {
    id: "optimistic-user-message",
    role: "user",
    schema_version: 1,
    blocks: [{ type: "text", block_id: "optimistic-user-block", text }],
    created_at: new Date().toISOString(),
  };
}

interface PendingSendEcho {
  readonly targetId: string | undefined;
  readonly content: string;
  readonly matchingMessagesBeforeSend: number;
}

export function AssistantPage({
  view = "chat",
}: {
  readonly view?: "chat" | "plugins" | "approvals";
}) {
  const navigate = useNavigate();
  const user = useAuthStore((state) => state.user);
  const [entryScreen] = useState(() => {
    const context = useAssistantContextStore.getState();
    const currentUser = useAuthStore.getState().user;
    return currentUser && context.ownerUserId === currentUser.id
      ? context.lastScreen
      : null;
  });
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
  // Which conversation the in-flight send was aimed at, so its optimistic echo
  // cannot follow the reader into a different thread. State rather than a ref:
  // it is read during render to build the message list.
  const [pendingSendEcho, setPendingSendEcho] = useState<PendingSendEcho>();

  useLayoutEffect(() => {
    const element = composerRef.current;
    if (!element || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver((entries) => {
      setComposerHeight(entries[0]?.contentRect.height ?? 0);
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, [view]);

  const boundConversationId = useAssistantContextStore((state) => {
    if (!user || !entryScreen || state.ownerUserId !== user.id) {
      return undefined;
    }
    return state.bindings[entryScreen]?.conversationId;
  });
  const selectedId = useMemo(() => {
    if (drafting) {
      const explicitConversationIsUsable = Boolean(
        selectedFromSearch &&
        (!conversations.isSuccess ||
          (conversations.data ?? []).some(
            (conversation) => conversation.id === selectedFromSearch,
          )),
      );
      return explicitConversationIsUsable ? selectedFromSearch : undefined;
    }
    return resolveAssistantConversationId({
      explicitConversationId: selectedFromSearch,
      boundConversationId,
      entryScreen,
      conversationsResolved: conversations.isSuccess,
      conversations: conversations.data ?? [],
    });
  }, [
    boundConversationId,
    conversations.data,
    conversations.isSuccess,
    drafting,
    entryScreen,
    selectedFromSearch,
  ]);
  const history = useConversation(selectedId);
  const turn = useAssistantTurn(selectedId);
  const episode = useTurnEpisode(selectedId);
  const sendMessage = useSendMessage(selectedId);
  const cancelTurn = useCancelTurn(selectedId);
  const decideApproval = useDecideApproval(selectedId);
  const deleteConversation = useDeleteConversation();
  const selectedConversationExists = (conversations.data ?? []).some(
    (conversation) => conversation.id === selectedId,
  );
  const explicitConversationIsStale = Boolean(
    selectedFromSearch &&
    conversations.isSuccess &&
    !(conversations.data ?? []).some(
      (conversation) => conversation.id === selectedFromSearch,
    ),
  );

  useEffect(() => {
    if (!conversations.isSuccess) return;
    const existingIds = (conversations.data ?? []).map(
      (conversation) => conversation.id,
    );
    useAssistantContextStore.getState().pruneBindings(existingIds);
    useAssistantDraftStore.getState().pruneConversationDrafts(existingIds);
  }, [conversations.data, conversations.isSuccess]);

  useEffect(() => {
    if (view !== "chat" || !conversations.isSuccess) {
      return;
    }
    if (selectedFromSearch && !explicitConversationIsStale) return;
    if (!selectedId && !explicitConversationIsStale) return;
    void navigate({
      to: "/assistant" as never,
      search: selectedId
        ? ({ c: selectedId } as never)
        : drafting
          ? ({ draft: true } as never)
          : ({} as never),
      replace: true,
    });
  }, [
    conversations.isSuccess,
    drafting,
    explicitConversationIsStale,
    navigate,
    selectedFromSearch,
    selectedId,
    view,
  ]);

  useEffect(() => {
    if (
      view !== "chat" ||
      !user ||
      !entryScreen ||
      !selectedId ||
      !(conversations.data ?? []).some(
        (conversation) => conversation.id === selectedId,
      )
    ) {
      return;
    }
    useAssistantContextStore
      .getState()
      .bindConversation(user.id, entryScreen, selectedId);
  }, [conversations.data, entryScreen, selectedId, user, view]);

  function selectConversation(conversationId: string, replace = false) {
    void navigate({
      to: "/assistant" as never,
      search: { c: conversationId } as never,
      replace,
    });
  }

  /**
   * "New chat" is navigation only — it issues NO requests.
   *
   * An empty conversation has no server-side representation on any upstream
   * surface: the `chat-history` transcript 404s, the `nyxid-chat` registry
   * lists ids without titles, and `/state` 404s until the controller commits
   * something. So there is nothing to fetch and nothing to show that the
   * client cannot paint itself. Aevatar's own reference client
   * (`apps/aevatar-console-web`) never calls create either — it allocates the
   * conversation with the first message.
   *
   * Provisioning eagerly here is what broke new chat: the create landed, the
   * swap to `?c=<id>` re-enabled the history query, and the transcript read
   * 404'd because no turn had ever materialized a history row. Allocation now
   * happens lazily on first send via `useSendMessage`'s `createConversationOnce`
   * single-flight, and `handleSend` navigates to the id it returns — so the
   * first history read only ever runs against a conversation that has a turn.
   */
  function createNewChat() {
    void navigate({
      to: "/assistant" as never,
      search: { draft: true } as never,
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
    // The mutation outlives a conversation switch, and its `variables` say
    // nothing about where the text was going — without this the reader can
    // switch chats mid-send and watch the previous chat's message appear in
    // this one's transcript. `undefined` is the draft thread, which matches.
    const normalizedContent = content.trim();
    const matchingMessagesBeforeSend = (history.data?.messages ?? []).filter(
      (message) => userMessageText(message)?.trim() === normalizedContent,
    ).length;
    setPendingSendEcho({
      targetId: selectedId,
      content,
      matchingMessagesBeforeSend,
    });
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
  const turnStatus = turn.data?.status;
  const active = isTurnActive(turnStatus);

  /**
   * Paint the turn from the in-flight send until the transcript catches up.
   *
   * "New chat" allocates nothing, so the first send has to create the
   * conversation before it has an id — and until that round-trip returns there
   * is no id, no enabled history query, and nothing on screen. The composer has
   * already cleared the textarea by then (it clears, THEN awaits), so the reader
   * watches their message vanish into the empty state for the whole create.
   *
   * Also covers the shorter gap on an existing conversation, between the send
   * starting and the projection landing.
   */
  const pendingEcho = useMemo(
    () =>
      sendMessage.isPending &&
      pendingSendEcho !== undefined &&
      pendingSendEcho.targetId === selectedId
        ? pendingSendEcho
        : sendMessage.isPending &&
            pendingSendEcho === undefined &&
            typeof sendMessage.variables === "string"
          ? {
              targetId: selectedId,
              content: sendMessage.variables,
              matchingMessagesBeforeSend: 0,
            }
          : undefined,
    [pendingSendEcho, selectedId, sendMessage.isPending, sendMessage.variables],
  );
  const messages = useMemo(() => {
    const transcript = history.data?.messages ?? [];
    if (pendingEcho === undefined) return transcript;
    const normalizedContent = pendingEcho.content.trim();
    const matchingMessages = transcript.filter(
      (message) => userMessageText(message)?.trim() === normalizedContent,
    ).length;
    // The transport's projected copy is present only when the count advanced
    // beyond the send-time snapshot. An older identical message is not the
    // identity of this send and must not suppress its optimistic echo.
    const projected = matchingMessages > pendingEcho.matchingMessagesBeforeSend;
    return projected
      ? transcript
      : [...transcript, optimisticUserMessage(pendingEcho.content)];
  }, [history.data?.messages, pendingEcho]);
  const episodeState = episode.data ?? undefined;
  const awaitingFirstTurn =
    active || sendMessage.isPending || episodeState?.open === true;

  /**
   * Whether THIS episode has closed, per the pump that served it.
   *
   * Turn status cannot answer this. It still holds the previous turn's terminal
   * value until the new episode's first event lands, and an approval
   * continuation reuses the original turn id outright — so a closed status paired
   * with a content-free tail is not evidence that the current stream said
   * nothing. Cancelled stays excluded whichever way it arrives: the reader
   * pressed Stop, and calling their own decision an error would be a lie.
   */
  const turnEnded =
    episodeState !== undefined &&
    !episodeState.open &&
    turnStatus !== "cancelled";
  const draftKey =
    selectedId && (!conversations.isSuccess || selectedConversationExists)
      ? `conv:${selectedId}`
      : !selectedFromSearch && entryScreen
        ? `screen:${entryScreen}`
        : null;
  const sidebar = (
    <AssistantSidebar
      conversations={conversations.data ?? []}
      activeConversationId={view === "chat" ? selectedId : undefined}
      activeView={view}
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
        {/* A failed transcript read is REPORTED, never blocking. It used to
            replace the whole thread, which also took the composer's context
            away and made the chat look dead — even though sending still
            works: the turn streams into the conversation actor, which is a
            different upstream surface from the chat-history transcript.
            The notice sits above a still-usable thread and composer. */}
        {history.isError ? (
          <div
            role="status"
            className="border-b border-border/60 bg-destructive/5 px-6 py-2 text-center text-[12px] text-destructive"
          >
            {describeHistoryError(history.error)} You can keep chatting — new
            messages are unaffected.
          </div>
        ) : null}
        {history.isLoading && messages.length === 0 ? (
          <div className="flex flex-1 items-center justify-center text-[12px] text-text-tertiary">
            Loading conversation...
          </div>
        ) : (
          <ChatThread
            messages={messages}
            bottomInset={composerHeight}
            thinking={
              awaitingFirstTurn &&
              (!active || messages.at(-1)?.role !== "assistant")
            }
            streaming={active && messages.at(-1)?.role === "assistant"}
            turnEnded={turnEnded}
            turnPrinted={episodeState?.printed}
            transcriptSettling={
              episodeState?.projecting === true || sendMessage.isPending
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
            ownerUserId={user?.id ?? null}
            draftKey={draftKey}
            onSend={handleSend}
            onStop={() => cancelTurn.mutateAsync()}
          />
        </div>
      </div>
    </AssistantShell>
  );
}
