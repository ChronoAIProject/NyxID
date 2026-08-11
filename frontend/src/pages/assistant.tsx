import {
  lazy,
  Suspense,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ComponentType,
  type LazyExoticComponent,
} from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useNavigate, useRouterState } from "@tanstack/react-router";
import { toast } from "sonner";
import { AssistantShell } from "@/components/assistant/assistant-shell";
import { AssistantSidebar } from "@/components/assistant/assistant-sidebar";
import { ChatComposer } from "@/components/assistant/chat-composer";
import { ChatThread } from "@/components/assistant/chat-thread";
import {
  DirectChatControls,
  DirectModeBanner,
  DIRECT_MODE_COPY,
} from "@/components/assistant/direct-chat-controls";
import { ApprovalsView } from "@/components/assistant/approvals-view";
import { PluginsView } from "@/components/assistant/plugins-view";
import { AssistantWireLogAction } from "@/components/assistant/assistant-wire-log-panel";
import {
  assistantKeysFor,
  describeSendFailure,
  useAssistantEngine,
  useAssistantTurn,
  useActionCardActions,
  useCancelTurn,
  useConversation,
  useConversations,
  useDecideApproval,
  useDeleteConversation,
  useSendMessage,
  useTurnEpisode,
} from "@/hooks/use-assistant";
import { useFeature } from "@/hooks/use-feature-flag";
import { ApiError } from "@/lib/api-client";
import {
  AssistantConversationNotFoundError,
  AssistantTurnActiveError,
  AssistantTurnCancelledError,
} from "@/lib/assistant/errors";
import { markChatActivity } from "@/lib/assistant/connect-watch";
import { assistantTransport } from "@/lib/assistant/transport";
import { parseAssistantSearch } from "@/lib/assistant/search";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import type { ActionReport } from "@/schemas/assistant-actions";
import { useAssistantContextStore } from "@/stores/assistant-context-store";
import { useAssistantDraftStore } from "@/stores/assistant-draft-store";
import { useAuthStore } from "@/stores/auth-store";
import { isTurnActive, type AssistantMessage } from "@/types/assistant";

const MockScenariosAction = import.meta.env.DEV
  ? lazy(() =>
      import("@/components/assistant/mock-scenarios-action").then((module) => ({
        default: module.MockScenariosAction,
      })),
    )
  : null;

type ScenarioActionComponent =
  | ComponentType
  | LazyExoticComponent<ComponentType>;

export function AssistantHeaderActions({
  scenarioAction = MockScenariosAction,
}: {
  readonly scenarioAction?: ScenarioActionComponent | null;
} = {}) {
  const ScenarioAction = scenarioAction;
  return (
    <>
      {ScenarioAction ? (
        <Suspense fallback={null}>
          <ScenarioAction />
        </Suspense>
      ) : null}
      <AssistantWireLogAction />
    </>
  );
}

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

function belongsToOtherEngine(
  conversationId: string | undefined,
  engine: "aevatar" | "direct",
): boolean {
  if (!conversationId) return false;
  if (engine === "direct") {
    return /^(?:nyxid-chat-|chatc-|workflow-pending-|nyxid-pending-)/.test(
      conversationId,
    );
  }
  return conversationId.startsWith("direct-");
}

export function AssistantPage({
  view = "chat",
}: {
  readonly view?: "chat" | "plugins" | "approvals";
}) {
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const user = useAuthStore((state) => state.user);
  const directChatEnabled = useFeature(FEATURE_FLAG.DIRECT_CHAT_ENGINE);
  const engine = directChatEnabled ? "direct" : "aevatar";
  const keys = assistantKeysFor(engine);
  useAssistantEngine(engine);
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
  const currentPathname = useRouterState({
    select: (state) => state.location.pathname,
  });
  const conversations = useConversations(engine);

  // The composer floats over the thread, so the thread has to know how tall it
  // currently is — it grows with the draft — to reserve the matching tail room
  // and place its fade.
  const composerRef = useRef<HTMLDivElement>(null);
  const [composerHeight, setComposerHeight] = useState(0);
  // Which conversation the in-flight send was aimed at, so its optimistic echo
  // cannot follow the reader into a different thread. State rather than a ref:
  // it is read during render to build the message list.
  const [pendingSendEcho, setPendingSendEcho] = useState<PendingSendEcho>();
  const previousEngineRef = useRef(engine);
  /**
   * Bumped only by an explicit "put me in this chat" action — a sidebar row or
   * New Chat. The composer takes focus on every bump.
   *
   * A `draftKey` change cannot stand in for this. The URL also moves on its
   * own: canonical-id repair swaps `?c=` under a conversation the reader is
   * already in, confirmed-stale repair drops it entirely, and the first send
   * migrates a draft thread onto a real id. None of those are anyone asking to
   * type, and focusing the composer for them would pull a reader out of
   * whatever they were actually doing.
   */
  const [composerFocusRequest, setComposerFocusRequest] = useState(0);
  function requestComposerFocus() {
    setComposerFocusRequest((request) => request + 1);
  }

  useLayoutEffect(() => {
    const element = composerRef.current;
    if (!element || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver((entries) => {
      setComposerHeight(entries[0]?.contentRect.height ?? 0);
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, [view]);

  // A conversation is selected only when the URL explicitly addresses it.
  // Keep that id through list/history failures so the history query can prove
  // a real 404; `?draft=true` is the same empty state as bare `/assistant`.
  const otherEngineSelected = belongsToOtherEngine(selectedFromSearch, engine);
  const selectedId =
    !drafting && selectedFromSearch && !otherEngineSelected
      ? selectedFromSearch
      : undefined;
  const history = useConversation(selectedId, engine);
  const turn = useAssistantTurn(selectedId, engine);
  const episode = useTurnEpisode(selectedId, engine);
  const sendMessage = useSendMessage(selectedId, engine);
  const cancelTurn = useCancelTurn(selectedId, engine);
  const decideApproval = useDecideApproval(selectedId, engine);
  const actionCards = useActionCardActions(selectedId, engine);
  const deleteConversation = useDeleteConversation(engine);
  const turnStatus = turn.data?.status;
  const active = isTurnActive(turnStatus);
  const episodeState = episode.data ?? undefined;
  const reactiveTurnLive =
    active ||
    sendMessage.isPending ||
    cancelTurn.isPending ||
    decideApproval.isPending ||
    episodeState?.open === true;

  useEffect(() => {
    const engineChanged = previousEngineRef.current !== engine;
    previousEngineRef.current = engine;
    if (!engineChanged && !otherEngineSelected) return;

    if (selectedFromSearch && otherEngineSelected) {
      assistantTransport.cancelActiveTurn(selectedFromSearch);
    }
    if (engineChanged) {
      useAssistantDraftStore.getState().clear();
    } else if (user && selectedFromSearch) {
      useAssistantDraftStore
        .getState()
        .clearDraft(user.id, `conv:${selectedFromSearch}`);
    }
    setPendingSendEcho(undefined);
    void navigate({
      to: "/assistant" as never,
      search: {} as never,
      replace: true,
    });
  }, [engine, navigate, otherEngineSelected, selectedFromSearch, user]);

  // Effects queued by an earlier quiet render must observe a continuation
  // that starts before they fire. The counter covers the synchronous window
  // before React Query publishes pending/episode state.
  const synchronousContinuationsRef = useRef(0);
  const reactiveTurnLiveRef = useRef(reactiveTurnLive);
  const turnLiveRef = useRef(reactiveTurnLive);
  reactiveTurnLiveRef.current = reactiveTurnLive;
  turnLiveRef.current =
    reactiveTurnLive || synchronousContinuationsRef.current > 0;

  function beginContinuation() {
    // Every user-initiated turn funnels through here — send, approval
    // decision, action report. That makes it the natural liveness signal for
    // background work the chat owns: a card waiting on an out-of-band
    // authorization treats "still using the chat" as "still here", so moving
    // on to something else extends its window instead of ending it.
    markChatActivity();
    synchronousContinuationsRef.current += 1;
    turnLiveRef.current = true;
  }

  function endContinuation() {
    synchronousContinuationsRef.current = Math.max(
      0,
      synchronousContinuationsRef.current - 1,
    );
    turnLiveRef.current =
      reactiveTurnLiveRef.current || synchronousContinuationsRef.current > 0;
  }

  const currentConversationRef = useRef(selectedFromSearch);
  const currentPathnameRef = useRef(currentPathname);
  currentConversationRef.current = selectedFromSearch;
  currentPathnameRef.current = currentPathname;

  const explicitConversationIsConfirmedStale = Boolean(
    selectedFromSearch &&
    conversations.isSuccess &&
    !(conversations.data ?? []).some(
      (conversation) => conversation.id === selectedFromSearch,
    ) &&
    history.isError &&
    (history.error instanceof AssistantConversationNotFoundError ||
      (history.error instanceof ApiError && history.error.status === 404)) &&
    !reactiveTurnLive,
  );

  useEffect(() => {
    if (!conversations.isSuccess) return;
    const existingIds = (conversations.data ?? []).map(
      (conversation) => conversation.id,
    );
    useAssistantDraftStore.getState().pruneConversationDrafts(existingIds);
  }, [conversations.data, conversations.isSuccess]);

  useEffect(() => {
    if (view !== "chat" || currentPathname !== "/assistant") return;
    const sourceConversationId = selectedFromSearch;
    if (!sourceConversationId) return;

    const canonicalConversationId = history.data?.conversation.id;
    const shouldRepairStaleId = explicitConversationIsConfirmedStale;
    const shouldSwapToCanonicalId = Boolean(
      !shouldRepairStaleId &&
      canonicalConversationId &&
      canonicalConversationId !== sourceConversationId &&
      !reactiveTurnLive,
    );
    if (!shouldRepairStaleId && !shouldSwapToCanonicalId) return;

    // Fire-time guards: router state and turn state can both advance after
    // this effect was queued. A stale effect may never override either one.
    if (currentPathnameRef.current !== "/assistant") return;
    if (currentConversationRef.current !== sourceConversationId) return;
    if (turnLiveRef.current) return;

    if (shouldRepairStaleId) {
      void navigate({
        to: "/assistant" as never,
        search: {} as never,
        replace: true,
      });
      return;
    }
    if (!canonicalConversationId || !history.data) return;

    // The event pump owns the placeholder slots. Transfer its settled
    // projection before changing the query key so the canonical render has a
    // complete transcript and terminal turn state on its very first frame.
    queryClient.setQueryData(
      keys.history(canonicalConversationId),
      history.data,
    );
    queryClient.setQueryData(
      keys.episode(canonicalConversationId),
      episode.data ?? null,
    );
    queryClient.setQueryData(
      keys.turn(canonicalConversationId),
      turn.data ?? null,
    );
    void navigate({
      to: "/assistant" as never,
      search: { c: canonicalConversationId } as never,
      replace: true,
    });
  }, [
    currentPathname,
    episode.data,
    explicitConversationIsConfirmedStale,
    history.data,
    keys,
    navigate,
    queryClient,
    reactiveTurnLive,
    selectedFromSearch,
    turn.data,
    view,
  ]);

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
  // stale id lingers in the address bar and re-selects nothing on reload.
  // Failures are said out loud here and re-thrown so the sidebar keeps its
  // confirm dialog open and the delete stays retryable.
  async function handleDelete(conversationId: string) {
    try {
      await deleteConversation.mutateAsync(conversationId);
      if (
        conversationId === selectedFromSearch ||
        conversationId === history.data?.conversation.id
      ) {
        // Deleting the chat you were in lands you on the draft screen, and the
        // confirm dialog's trigger went with it — so focus has nowhere to
        // return to. This one IS a reader-initiated arrival at an empty
        // composer, unlike the repair navigations, so it asks for focus.
        requestComposerFocus();
        void navigate({ to: "/assistant" as never, search: {} as never });
      }
    } catch (error) {
      toast.error("Could not delete the chat", {
        description:
          error instanceof Error && error.message
            ? error.message
            : "The assistant backend did not respond. Try again.",
      });
      throw error;
    }
  }

  // First send from the "New chat" empty state has no conversation yet; the
  // mutation auto-creates one and this follows the navigation to it. A send
  // that fails before any turn exists must be said out loud — the composer
  // restores the text, and without the toast the button just looks dead.
  async function handleSend(content: string) {
    beginContinuation();
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
      const { message, description } = describeSendFailure(error, engine);
      toast.error(message, { description });
      throw error;
    } finally {
      endContinuation();
    }
  }

  async function handleDecideApproval(blockId: string, approved: boolean) {
    beginContinuation();
    try {
      await decideApproval.mutateAsync({ blockId, approved });
    } finally {
      endContinuation();
    }
  }

  async function handleResolveAction(report: ActionReport) {
    beginContinuation();
    try {
      await actionCards.continueAction(report);
    } catch (error) {
      // A continuation that dies before any stream event is otherwise
      // invisible: the card's own catch only unlocks dismissal. Same
      // contract as approval delivery — toast, stable id, and a user Stop
      // is an expected outcome, not a delivery failure. Rethrown so the
      // card rolls itself out of its busy state.
      if (!(error instanceof AssistantTurnCancelledError)) {
        toast.error("The action response was not delivered", {
          id: "assistant-action-failed",
          description:
            error instanceof AssistantTurnActiveError
              ? "Wait for the current reply to finish, then try again."
              : error instanceof Error && error.message
                ? error.message
                : "The assistant backend did not respond. Try again.",
        });
      }
      throw error;
    } finally {
      endContinuation();
    }
  }

  const title =
    view === "plugins"
      ? "Plugins"
      : view === "approvals"
        ? "Approvals"
        : (history.data?.conversation.title ?? "New chat");
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
  const draftKey = selectedId
    ? `conv:${selectedId}`
    : entryScreen
      ? engine === "direct"
        ? `screen:direct:${entryScreen}`
        : `screen:${entryScreen}`
      : null;
  const sidebar = (
    <AssistantSidebar
      conversations={conversations.data ?? []}
      activeConversationId={
        view === "chat"
          ? (history.data?.conversation.id ?? selectedId)
          : undefined
      }
      activeView={view}
      deletingId={
        deleteConversation.isPending ? deleteConversation.variables : undefined
      }
      onNewChat={() => {
        requestComposerFocus();
        createNewChat();
      }}
      onSelect={(conversationId) => {
        requestComposerFocus();
        selectConversation(conversationId);
      }}
      onDelete={handleDelete}
    />
  );

  if (view === "plugins" || view === "approvals") {
    return (
      <AssistantShell
        title={title}
        sidebar={sidebar}
        headerActions={<AssistantHeaderActions />}
      >
        {view === "plugins" ? <PluginsView /> : <ApprovalsView />}
      </AssistantShell>
    );
  }

  return (
    <AssistantShell
      title={title}
      sidebar={sidebar}
      headerActions={<AssistantHeaderActions />}
    >
      <div className="relative flex h-full min-h-0 flex-col bg-background">
        {directChatEnabled ? <DirectModeBanner /> : null}
        {/* Projection provenance (`awaitingProjection` / `projectionStalled`)
            deliberately renders NOTHING here. The transcript demonstrably
            materializes on its own — the reconciler projects it into the
            thread below with no reload — and a status strip narrating that
            plumbing claimed more than it knew (it stood beside a false
            "didn't reply" error for turns whose answer was already committed
            upstream). The provenance still drives the background reconciler
            via `useConversation`; only the furniture is gone. Refined
            surfaces are deferred W4/W5 in docs/plans/newchat-followup-fix.md. */}
        {/* A failed transcript read is toast-only (`useHistoryErrorToast`,
            fired from `useConversation`): every stream/turn error on this
            surface goes through toasts, and none of them blocks the thread
            or the composer. */}
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
            emptyDescription={directChatEnabled ? DIRECT_MODE_COPY : undefined}
            onDecideApproval={handleDecideApproval}
            onActionProgress={actionCards.setInProgress}
            onBlockAction={actionCards.blockAction}
            onResolveAction={handleResolveAction}
          />
        )}
        <div ref={composerRef} className="absolute inset-x-0 bottom-0 z-10">
          <ChatComposer
            active={active}
            sending={sendMessage.isPending}
            ownerUserId={user?.id ?? null}
            draftKey={draftKey}
            focusRequest={composerFocusRequest}
            controls={
              directChatEnabled ? (
                <DirectChatControls
                  conversationId={selectedId}
                  disabled={active || sendMessage.isPending}
                />
              ) : undefined
            }
            onSend={handleSend}
            onStop={() => cancelTurn.mutateAsync()}
          />
        </div>
      </div>
    </AssistantShell>
  );
}
