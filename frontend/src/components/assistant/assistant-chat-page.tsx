import type { PreparedMicrophone } from "@/lib/microphone-capture";
import { navigateWithBuildUpdate } from "@/lib/build-update-navigation";
import { AssistantErrorNotice } from "./assistant-error-notice";
import { assistantErrorSummary, withAssistantTrace } from "@/lib/assistant/request-error";
import { isAdmittedSendError } from "@/lib/assistant/send-admission";
import { parseAccountPanelSearch } from "@/lib/assistant/account-panel-search";
import { ConversationMachineDesktops } from "./machine-desktop-panel";
import { ReviewCardTarget } from "./review-card-target";
import { NyxAgentAcknowledgementCard } from "./nyxagent-acknowledgement-card";
import { NyxBotEventNotice, NyxBotOrchestratorMessage } from "./nyxbot-messages";
import { NyxBotSettingsButton } from "./nyxbot-settings-button";
import {
  ChannelBadge,
  PendingEventsNote,
  TeamStrip,
  ThreadHeader,
  WaitingNote,
} from "./nyxbot-agent-panels";
import { AgentDetailsSheet } from "./nyxbot-agent-details";
import { NewAgentDialog } from "./nyxbot-agent-forms";
import { NewGroupDialog } from "./nyxbot-group-forms";
import { NyxAgentGroupPage } from "./nyxbot-group-view";
import { VoiceCaptionText } from "./voice-captions";
import {
  mergeVoiceCaptions,
  voiceDeliveryRevision,
  type VoiceView,
} from "./voice-caption-state";
import { LazyVoicePanel as VoicePanel } from "./lazy-voice-panel";
import { useFeature } from "@/hooks/use-feature-flag";
import { useNyxBotSettings } from "@/hooks/use-nyxbot-agents";
import { NyxBotHome } from "./nyxbot-home";
import { nyxBotOf, selectedAgentOf, useNyxBotAgents } from "@/hooks/use-nyxbot-agents";
import { useNyxBotGroups } from "@/hooks/use-nyxbot-groups";
import {
  agentHandle,
  agentTitle,
  effectiveAccessMode,
  groupWithDisplayNames,
  withDisplayName,
} from "@/lib/assistant/nyxbot-labels";
import { nyxAgentTransport } from "@/lib/assistant/nyxagent-transport";
import type { NyxAgentConversationAgent } from "@/schemas/assistant-nyxagent";
import { ApprovalCard } from "@/components/assistant/blocks/approval-card";
import { AssistantLinkModalHost } from "@/components/assistant/assistant-link-modals";
import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useNavigate, useRouterState } from "@tanstack/react-router";
import { toast } from "sonner";
import { AssistantShell } from "@/components/assistant/assistant-shell";
import { AssistantEngineSidebar } from "@/components/assistant/assistant-engine-sidebar";
import { useNyxAgentAssistantChat } from "@/hooks/use-assistant-nyxagent";
import { Button } from "@/components/ui/button";
import { AssistantWireLogAction } from "@/components/assistant/assistant-wire-log-panel";
import { ChatActorControls } from "@/components/assistant/chat-actor-controls";
import { NyxAgentComposer } from "@/components/assistant/nyxagent-composer";
import { ChatComposer } from "@/components/assistant/chat-composer";
import { ChatMessageBubble, ChatMessageEntry, ChatMessageList } from "@/components/assistant/chat-message";
import { WorkingContextPanel } from "@/components/assistant/working-context-panel";
import type { ChatSessionState } from "@/lib/assistant/chat-types";
import {
  DirectChatControls,
  DirectModeBanner,
  DIRECT_MODE_COPY,
} from "@/components/assistant/direct-chat-controls";
import { useAssistantChat } from "@/hooks/use-assistant-chat";
import { useDirectAssistantChat } from "@/hooks/use-assistant-direct";
import { isLegacyConversationId } from "@/lib/assistant/conversation-ids";
import { markChatActivity } from "@/lib/assistant/connect-watch";
import { directAssistantTransport } from "@/lib/assistant/direct-transport";
import { parseAssistantSearch } from "@/lib/assistant/search";
import { useAuthStore } from "@/stores/auth-store";
import type { Conversation } from "@/types/assistant";

const MockScenariosAction = import.meta.env.DEV
  ? lazy(() =>
      import("@/components/assistant/mock-scenarios-action").then((module) => ({
        default: module.MockScenariosAction,
      })),
    )
  : null;

function sidebarConversation(
  conversation: ReturnType<
    typeof useAssistantChat
  >["visibleConversations"][number],
): Conversation {
  return {
    id: conversation.id,
    title: conversation.title,
    created_at: conversation.createdAt,
    last_message_at: conversation.updatedAt,
    message_count: conversation.messageCount,
    llm_route: conversation.llmRoute,
    llm_model: conversation.llmModel,
  };
}

export function AevatarAssistantMessageList({ session, bottomInset, footer, notice, projectionVersion, emptyDescription }: { readonly session: ChatSessionState | null; readonly bottomInset: number; readonly footer?: React.ReactNode; readonly notice?: React.ReactNode; readonly projectionVersion?: string; readonly emptyDescription?: React.ReactNode }) { return <ChatMessageList session={session} bottomInset={bottomInset} footer={footer} notice={notice} projectionVersion={projectionVersion} emptyDescription={emptyDescription} />; }
/** How the NyxAgent thread page renders an assistant reply: the entry with
 * author header and feedback controls, and Retry for a failed reply. */
export function NyxAgentAssistantEntry({
  message,
  conversationId,
  retry,
  checkRecovery,
}: {
  readonly message: import("@/lib/assistant/chat-types").ChatMessage;
  readonly conversationId: string | undefined;
  readonly retry: (messageId: string) => Promise<void>;
  readonly checkRecovery: (messageId: string) => Promise<unknown>;
}) {
  return (
    <ChatMessageEntry
      message={message}
      conversationId={conversationId}
      feedbackEnabled
      onRetry={message.status === "error" && message.recovery?.status === "available"
        ? () => retry(message.id) : undefined}
      onCheckRecovery={message.status === "error" &&
        (!message.recovery || ["turn_active", "status_unavailable"].includes(message.recovery.reason))
        ? () => checkRecovery(message.id) : undefined}
    />
  );
}

/** A user message the owner wrote in a chat app, not here. */
export function ChannelUserEntry({
  message,
  platform,
}: {
  readonly message: import("@/lib/assistant/chat-types").ChatMessage;
  readonly platform: string;
}) {
  return (
    <div className="flex flex-col items-end gap-1">
      <ChannelBadge platform={platform} />
      <ChatMessageBubble message={message} />
    </div>
  );
}

export function NyxAgentThreadMessageList({ session, bottomInset, renderMessage, projectionVersion, notice, emptyDescription, focusMessageId }: { readonly session: ChatSessionState | null; readonly bottomInset: number; readonly renderMessage?: (message: import("@/lib/assistant/chat-types").ChatMessage) => React.ReactNode; readonly projectionVersion?: string; readonly notice?: React.ReactNode; readonly emptyDescription?: React.ReactNode; readonly focusMessageId?: string }) { return <ChatMessageList session={session} bottomInset={bottomInset} feedbackEnabled renderMessage={renderMessage} projectionVersion={projectionVersion} notice={notice} emptyDescription={emptyDescription} focusMessageId={focusMessageId} />; }

export function AssistantChatPage() {
  const navigate = useNavigate();
  const user = useAuthStore((state) => state.user);
  const selectedConversationId = useRouterState({
    select: (state) =>
      parseAssistantSearch(state.location.search as Record<string, unknown>).c,
  });
  const drafting = useRouterState({
    select: (state) =>
      parseAssistantSearch(state.location.search as Record<string, unknown>)
        .draft === true,
  });
  const fixtureMode = Boolean(
    import.meta.env.DEV &&
      typeof window !== "undefined" &&
      new URLSearchParams(window.location.search).get("mock") === "1",
  );
  const selectedId = drafting ? undefined : selectedConversationId;
  const composerRef = useRef<HTMLDivElement>(null);
  const [composerHeight, setComposerHeight] = useState(0);
  const [composerFocusRequest, setComposerFocusRequest] = useState(0);

  const adoptConversation = useCallback(
    (conversationId: string) => {
      void navigate({
        to: "/assistant",
        search: (previous) => ({ ...parseAccountPanelSearch(previous), c: conversationId, ...(fixtureMode ? { mock: 1 as const } : {}) }),
        replace: true,
      });
    },
    [fixtureMode, navigate],
  );
  const repairMissingConversation = useCallback(() => {
    void navigate({
      to: "/assistant",
      search: (previous) => ({ ...parseAccountPanelSearch(previous), ...(fixtureMode ? { mock: 1 as const } : {}) }),
      replace: true,
    });
  }, [fixtureMode, navigate]);
  const chat = useAssistantChat({
    selectedConversationId: selectedId,
    onConversationAdopted: adoptConversation,
    onConversationMissing: repairMissingConversation,
  });

  useLayoutEffect(() => {
    const element = composerRef.current;
    if (!element || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver((entries) => {
      setComposerHeight(entries[0]?.contentRect.height ?? 0);
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  const conversations = useMemo(
    () => chat.visibleConversations.map(sidebarConversation),
    [chat.visibleConversations],
  );
  const actorActive = Boolean(
    chat.projection?.activeTurn?.status === "active" ||
    chat.projection?.task?.status === "active",
  );
  const title = chat.session?.title ?? "New chat";
  const readOnly = Boolean(
    chat.session?.conversationId &&
    isLegacyConversationId(chat.session.conversationId),
  );
  const draftKey = chat.session?.conversationId
    ? `conv:${chat.session.conversationId}`
    : null;

  function selectConversation(conversationId: string) {
    setComposerFocusRequest((value) => value + 1);
    void navigateWithBuildUpdate(() => navigate({
      to: "/assistant",
      search: {
        c: conversationId,
        ...(fixtureMode ? { mock: 1 } : {}),
      },
    }));
  }

  function createNewChat() {
    setComposerFocusRequest((value) => value + 1);
    chat.newChat();
    void navigateWithBuildUpdate(() => navigate({
      to: "/assistant",
      search: {
        draft: true,
        ...(fixtureMode ? { mock: 1 } : {}),
      },
    }));
  }

  async function deleteConversation(conversationId: string) {
    if (chat.isConversationStreaming(conversationId)) return;
    try {
      await chat.deleteConversation(conversationId);
      if (selectedId === conversationId) {
        void navigate({
          to: "/assistant",
          search: (fixtureMode ? { mock: 1 } : {}),
        });
      }
    } catch (error) {
      toast.error("Could not delete the chat", {
        description:
          error instanceof Error
            ? error.message
            : "The assistant backend did not respond. Try again.",
      });
      throw error;
    }
  }

  async function send(content: string) {
    markChatActivity();
    try {
      if (actorActive) await chat.steer(content);
      else await chat.send(content);
    } catch (error) {
      toast.error("The message was not delivered", {
        description:
          error instanceof Error
            ? error.message
            : "The assistant backend did not respond. Try again.",
      });
      throw error;
    }
  }

  const actorControls = readOnly ? null : (
    <ChatActorControls
      projection={chat.projection}
      disabled={chat.controlBusy || !chat.controlReady}
      actionOverrides={chat.actionOverrides}
      onResolveInput={chat.resolveInput}
      onResolveApproval={chat.resolveApproval}
      onResolvePlan={chat.resolvePlan}
      onStop={chat.stop}
      onControlStep={chat.controlStep}
      onActionProgress={(requestId, active) =>
        chat.setActionOverride(requestId, active ? "in_progress" : "pending")
      }
      onBlockAction={(requestId, note) =>
        chat.setActionOverride(requestId, "blocked", note)
      }
      onResolveAction={chat.reportAction}
    />
  );
  const sidebar = (
    <AssistantEngineSidebar engine="actor"
      conversations={conversations}
      activeConversationId={chat.session?.conversationId}
      onNewChat={createNewChat}
      onSelect={selectConversation}
      onDelete={deleteConversation}
      notice={
        chat.listError ? `Could not load chats. ${chat.listError}` : undefined
      }
    />
  );
  const threadNotice = readOnly
    ? "This legacy conversation is read-only. You can view or delete it, but it cannot be continued."
    : chat.detailState.status === "missing"
      ? "This chat has no saved transcript yet. You can keep chatting."
      : chat.detailState.status === "error"
        ? `Could not load earlier messages. ${chat.detailState.message}`
        : undefined;

  return (
    <AssistantShell
      title={title}
      sidebar={sidebar}
      headerActions={
        <>
          {MockScenariosAction ? (
            <Suspense fallback={null}>
              <MockScenariosAction />
            </Suspense>
          ) : null}
          <AssistantWireLogAction
            activeConversationId={chat.session?.conversationId ?? null}
          />
        </>
      }
    >
      <div className="relative flex h-full min-h-0 flex-col bg-background">
        {chat.detailState.status === "loading" &&
        !(chat.session?.messages.length ?? 0) ? (
          <div className="flex flex-1 items-center justify-center text-12 text-text-tertiary">
            Loading conversation...
          </div>
        ) : (
          <AssistantLinkModalHost>
            <ChatMessageList
              session={chat.session}
              bottomInset={composerHeight}
              footer={actorControls}
              notice={threadNotice}
              projectionVersion={`${String(chat.projection?.stateVersion ?? 0)}:${String(chat.projection?.progressSequence ?? 0)}`}
            />
          </AssistantLinkModalHost>
        )}
        <div ref={composerRef} className="absolute inset-x-0 bottom-0 z-10">
          <ChatComposer
            active={chat.isStreaming || actorActive}
            allowActiveInput={actorActive && chat.controlReady}
            sending={chat.isStreaming || chat.controlBusy}
            disabled={readOnly}
            ownerUserId={user?.id ?? null}
            draftKey={draftKey}
            focusRequest={composerFocusRequest}
            onSend={send}
            onStop={chat.stop}
          />
        </div>
      </div>
    </AssistantShell>
  );
}

export function DirectAssistantChatPage() {
  const navigate = useNavigate();
  const user = useAuthStore((state) => state.user);
  const selectedConversationId = useRouterState({
    select: (state) =>
      parseAssistantSearch(state.location.search as Record<string, unknown>).c,
  });
  const drafting = useRouterState({
    select: (state) =>
      parseAssistantSearch(state.location.search as Record<string, unknown>)
        .draft === true,
  });
  const selectedId = drafting ? undefined : selectedConversationId;
  const composerRef = useRef<HTMLDivElement>(null);
  const [composerHeight, setComposerHeight] = useState(0);
  const [composerFocusRequest, setComposerFocusRequest] = useState(0);

  const adoptConversation = useCallback(
    (conversationId: string) => {
      void navigate({
        to: "/assistant",
        search: (previous) => ({ ...parseAccountPanelSearch(previous), c: conversationId }),
        replace: true,
      });
    },
    [navigate],
  );
  const chat = useDirectAssistantChat({
    selectedConversationId: selectedId,
    onConversationAdopted: adoptConversation,
  });
  useLayoutEffect(() => {
    const element = composerRef.current;
    if (!element || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver((entries) => {
      setComposerHeight(entries[0]?.contentRect.height ?? 0);
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    if (!chat.isMissing) return;
    void navigate({
      to: "/assistant",
      search: (previous) => ({ ...parseAccountPanelSearch(previous), draft: true }),
      replace: true,
    });
  }, [chat.isMissing, navigate]);

  function selectConversation(conversationId: string) {
    setComposerFocusRequest((value) => value + 1);
    void navigateWithBuildUpdate(() => navigate({
      to: "/assistant",
      search: { c: conversationId },
    }));
  }

  function createNewChat() {
    setComposerFocusRequest((value) => value + 1);
    void navigateWithBuildUpdate(() => navigate({
      to: "/assistant",
      search: { draft: true },
    }));
  }

  async function deleteConversation(conversationId: string) {
    if (
      directAssistantTransport.getHistorySnapshot(conversationId)?.activeTurn
        ?.status === "running"
    ) {
      return;
    }
    try {
      await chat.deleteConversation(conversationId);
      if (selectedId === conversationId) createNewChat();
    } catch (error) {
      toast.error("Could not delete the chat", {
        description:
          error instanceof Error
            ? error.message
            : "The direct chat store did not respond. Try again.",
      });
      throw error;
    }
  }

  async function send(content: string) {
    markChatActivity();
    try {
      await chat.send(content);
    } catch (error) {
      toast.error("The message was not delivered", {
        description:
          error instanceof Error
            ? error.message
            : "The direct model did not respond. Try again.",
      });
      throw error;
    }
  }

  const draftKey = chat.session.conversationId
    ? `conv:${chat.session.conversationId}`
    : "screen:direct:assistant";
  const sidebar = (
    <AssistantEngineSidebar engine="direct"
      conversations={chat.conversations}
      activeConversationId={chat.session.conversationId}
      onNewChat={createNewChat}
      onSelect={selectConversation}
      onDelete={deleteConversation}
    />
  );

  return (
    <AssistantShell
      title={chat.session.title}
      sidebar={sidebar}
      headerActions={
        <AssistantWireLogAction
          activeConversationId={chat.session.conversationId ?? null}
        />
      }
    >
      <div className="relative flex h-full min-h-0 flex-col bg-background">
        <DirectModeBanner />
        <AssistantLinkModalHost>
        <AevatarAssistantMessageList
          session={chat.session}
          bottomInset={composerHeight}
          emptyDescription={DIRECT_MODE_COPY}
        />
        </AssistantLinkModalHost>
        <div ref={composerRef} className="absolute inset-x-0 bottom-0 z-10">
          <ChatComposer
            active={chat.isStreaming}
            sending={chat.isStreaming}
            ownerUserId={user?.id ?? null}
            draftKey={draftKey}
            focusRequest={composerFocusRequest}
            controls={
              <DirectChatControls
                conversationId={chat.session.conversationId}
                disabled={chat.isStreaming}
              />
            }
            onSend={send}
            onStop={chat.stop}
          />
        </div>
      </div>
    </AssistantShell>
  );
}

/**
 * The NyxAgent engine: a group chat (`?g=`), a thread (`?c=`, `?draft`,
 * `?agent=`), or the NyxBot home when none of those are set.
 */
export function NyxAgentAssistantChatPage() {
  const search = useRouterState({
    select: (state) => parseAssistantSearch(state.location.search as Record<string, unknown>),
  });
  if (search.g) {
    return <NyxAgentGroupPage key={search.g} groupId={search.g} mock={Boolean(search.mock)} />;
  }
  return <NyxAgentThreadPage />;
}

function NyxAgentThreadPage() {
  const voiceEnabled = useFeature("assistant:voice");
  const nyxBotSettings = useNyxBotSettings();
  const [voiceBinding, setVoiceBinding] = useState<{
    threadId: string;
    actorId?: string;
    token: string;
    preparation?: PreparedMicrophone;
    settingsRequest?: number;
  } | null>(null);
  const voiceBindingRef = useRef(voiceBinding);
  const voicePageMounted = useRef(false);
  useEffect(() => {
    voicePageMounted.current = true;
    return () => {
      voicePageMounted.current = false;
      queueMicrotask(() => {
        if (!voicePageMounted.current)
          voiceBindingRef.current?.preparation?.close();
      });
    };
  }, []);
  const voicePanelRef = useRef<HTMLDivElement>(null);
  function setVoiceThread(id?: string, preparation?: PreparedMicrophone) {
    const existing = voiceBindingRef.current;
    if (id && existing?.threadId === id && existing.actorId === user?.id) {
      if (preparation) {
        existing.preparation?.close();
        preparation.handoff();
        const binding = { ...existing, preparation };
        voiceBindingRef.current = binding;
        setVoiceBinding(binding);
      } else if (!voiceView?.active) {
        const binding = {
          ...existing,
          settingsRequest: (existing.settingsRequest ?? 0) + 1,
        };
        voiceBindingRef.current = binding;
        setVoiceBinding(binding);
      }
      voicePanelRef.current
        ?.querySelector<HTMLButtonElement>("button")
        ?.focus();
      return;
    }
    preparation?.handoff();
    existing?.preparation?.close();
    const binding = id
      ? {
          threadId: id,
          actorId: user?.id,
          token: crypto.randomUUID(),
          preparation,
          settingsRequest: preparation ? 0 : 1,
        }
      : null;
    voiceBindingRef.current = binding;
    setVoiceBinding(binding);
    if (!binding)
      setVoiceView((view) =>
        view
          ? {
              ...view,
              active: false,
              activity: {
                ...view.activity,
                input: 0,
                output: 0,
                bands: Array<number>(24).fill(0),
              },
            }
          : null,
      );
  }
  const [voiceView, setVoiceView] = useState<VoiceView | null>(null);
  const navigate = useNavigate();
  const user = useAuthStore((state) => state.user);
  const search = useRouterState({
    select: (state) => parseAssistantSearch(state.location.search as Record<string, unknown>),
  });
  const selectedId = search.draft ? undefined : search.c;
  const errorScope = `${user?.id}:${selectedId ?? `draft:${search.agent ?? "nyxbot"}`}`;
  const [requestError, setRequestError] = useState<{ scope: string; message: string }>();
  const pendingSend = useRef<{ scope: string } | undefined>(undefined);
  // No thread, draft or agent in the URL: the NyxBot home.
  const home = !search.c && !search.draft && !search.agent;
  // Router state is global: while the sidebar navigates to another page this
  // component is still mounted and sees that page's (thread-less) URL.
  const onChatRoute = useRouterState({
    select: (state) => state.location.pathname.replace(/\/+$/, "") === "/assistant",
  });
  const composerRef = useRef<HTMLDivElement>(null);
  const [composerHeight, setComposerHeight] = useState(0);
  const [focusRequest, setFocusRequest] = useState(0);
  // Identity adoption must not undo navigation performed while the POST waited.
  const selection = useRef(selectedId);
  useLayoutEffect(() => {
    selection.current = selectedId;
  }, [selectedId]);
  const receiveVoiceView = useCallback((view: VoiceView) => {
    if (voiceBindingRef.current?.token !== view.panelId) return;
    setVoiceView((previous) => {
      // End clears the client, while durable history may still be catching up.
      if (
        !view.active &&
        !view.snapshot &&
        previous &&
        previous.panelId === view.panelId
      )
        return {
          ...view,
          snapshot: previous.snapshot,
          activity: {
            ...view.activity,
            playbackMs: Math.max(
              view.activity.playbackMs,
              previous.activity.playbackMs,
            ),
          },
        };
      return view;
    });
  }, []);
  const voiceThread =
    voiceBinding?.actorId === user?.id ? voiceBinding?.threadId : undefined;
  const previousVoiceThread = useRef(selectedId);
  useEffect(() => {
    setVoiceView((view) =>
      view && (view.threadId !== selectedId || view.actorId !== user?.id)
        ? null
        : view,
    );
    const binding = voiceBindingRef.current;
    if (
      binding &&
      (binding.actorId !== user?.id ||
        (previousVoiceThread.current === binding.threadId &&
          selectedId !== binding.threadId) ||
        !onChatRoute)
    ) {
      voiceBindingRef.current?.preparation?.close();
      voiceBindingRef.current = null;
      setVoiceBinding(null);
    }
    previousVoiceThread.current = selectedId;
  }, [selectedId, user?.id, onChatRoute]);
  const adopt = useCallback((id: string) => {
    if (selection.current !== selectedId) return;
    if (pendingSend.current) pendingSend.current.scope = `${user?.id}:${id}`;
    void navigate({
      to: "/assistant",
      search: (previous) => ({ ...parseAccountPanelSearch(previous), c: id, ...(search.mock ? { mock: 1 as const } : {}) }),
      replace: true,
    });
  }, [navigate, search.mock, selectedId, user?.id]);
  const agents = useNyxBotAgents();
  const groups = useNyxBotGroups();
  // The agent whose details sheet is open (the thread's agent, or NyxBot from home).
  const [detailsAgentId, setDetailsAgentId] = useState<string>();
  const [creating, setCreating] = useState<"agent" | "group">();
  // The open thread's agent, else the agent in the URL, else NyxBot.
  const threadAgent = nyxAgentTransport.getConversation(selectedId)?.agent ?? undefined;
  const selectedAgent = selectedAgentOf(agents.data?.agents, threadAgent?.id, search.agent);
  // Names the exact linked agent even when it is not in the list.
  const linkedDetails = Boolean(search.details || search.proposal);
  const openDetailsAgentId =
    detailsAgentId ??
    (linkedDetails ? (search.agent ?? threadAgent?.id ?? selectedAgent?.id) : undefined);

  const draftAgent: NyxAgentConversationAgent | undefined = selectedAgent
    ? {
        id: selectedAgent.id,
        kind: selectedAgent.kind,
        name: selectedAgent.name,
        destroyed: selectedAgent.status === "destroyed",
      }
    : undefined;
  const headerAgent = selectedId ? threadAgent : draftAgent;
  const chat = useNyxAgentAssistantChat({
    selectedConversationId: selectedId,
    reviewCardId: search.card,
    onConversationAdopted: adopt,
    threadsAgentId: selectedAgent?.id,
    draftAgent,
  });
  // Keep rejected retries with the other request failures above the chat.
  const retryFailedMessage = useCallback(async (messageId: string) => {
    if (!selectedId) throw new Error("Select the conversation to retry.");
    setRequestError(undefined);
    try {
      await chat.retry(selectedId, messageId);
    } catch (error) {
      const message = error instanceof Error ? error.message : "Could not retry your message. Try again shortly.";
      setRequestError({ scope: errorScope, message });
      toast.error(isAdmittedSendError(error) ? "Live reply interrupted" : "Could not retry your message", {
        description: assistantErrorSummary(message),
      });
    }
  }, [chat, selectedId, errorScope]);
  const checkFailedMessage = useCallback(async (messageId: string) => {
    if (!selectedId) return;
    await chat.checkRecovery(selectedId, messageId);
  }, [chat, selectedId]);
  useLayoutEffect(() => {
    const element = composerRef.current;
    if (!element || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver((entries) => {
      setComposerHeight(entries[0]?.contentRect.height ?? 0);
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  // `?agent=` alone (the sidebar's agent rows): open that agent's latest
  // thread. The home never redirects.
  const latestThreadId = chat.conversations[0]?.id;
  useEffect(() => {
    if (
      !onChatRoute ||
      !search.agent ||
      search.c ||
      search.draft ||
      !chat.threadsLoaded ||
      !latestThreadId
    ) {
      return;
    }
    void navigate({
      to: "/assistant",
      search: (previous) => ({
        ...parseAccountPanelSearch(previous),
        c: latestThreadId,
        ...(linkedDetails ? { agent: search.agent } : {}),
        ...(search.details ? { details: search.details } : {}),
        ...(search.proposal ? { proposal: search.proposal } : {}),
        ...(search.mock ? { mock: 1 as const } : {}),
      }),
      replace: true,
    });
  }, [
    chat.threadsLoaded,
    latestThreadId,
    linkedDetails,
    navigate,
    onChatRoute,
    search.agent,
    search.c,
    search.details,
    search.draft,
    search.mock,
    search.proposal,
  ]);

  function navigateTo(next: { c?: string; draft?: true; agent?: string; g?: string }) {
    void navigateWithBuildUpdate(() => navigate({
      to: "/assistant",
      search: { ...next, ...(search.mock ? { mock: 1 } : {}) },
    }));
  }

  /**
   * Open a thread. Without one, a specialist starts a new thread and NyxBot
   * returns to its home (where the composer starts a new NyxBot thread).
   */
  function go(id?: string) {
    setVoiceThread(undefined);
    setFocusRequest((value) => value + 1);
    if (id) navigateTo({ c: id });
    else if (selectedAgent?.kind === "specialist") {
      navigateTo({ draft: true, agent: selectedAgent.id });
    } else navigateTo({});
  }

  const conversation = chat.conversation;
  // Threads carry the agent's handle; its display name lives on the agent list.
  const headerNamed = headerAgent ? withDisplayName(headerAgent, agents.data?.agents) : undefined;
  const agentName = headerNamed ? agentTitle(headerNamed) : "NyxBot";
  const destroyed = Boolean(headerAgent?.destroyed) || selectedAgent?.can_use === false;
  const channelPlatform = selectedId ? (conversation?.channel?.platform ?? null) : null;
  useEffect(() => {
    if (!voiceEnabled || destroyed || channelPlatform) {
      voiceBindingRef.current?.preparation?.close();
      voiceBindingRef.current = null;
      setVoiceBinding(null);
      setVoiceView(null);
    }
  }, [voiceEnabled, destroyed, channelPlatform]);
  // The home stays until the first message of its new NyxBot thread shows.
  const showHome = home && !chat.isStreaming && !chat.session.messages.length;
  const agentOverride = agents.data?.agents.find(
    (row) => row.id === headerAgent?.id,
  )?.access_mode_override;
  const accessMode = effectiveAccessMode(
    nyxBotSettings.data?.access_mode,
    agentOverride,
  );
  const currentVoice =
    voiceView &&
    voiceView.threadId === selectedId &&
    voiceView.actorId === user?.id
      ? voiceView
      : null;
  const voiceMounted = Boolean(
    voiceEnabled &&
    voiceThread === selectedId &&
    voiceBinding?.token === currentVoice?.panelId &&
    !destroyed &&
    !channelPlatform,
  );
  const liveCaptions = currentVoice?.snapshot?.captions ?? [];
  const voiceSession = {
    ...chat.session,
    messages: mergeVoiceCaptions(chat.session.messages, liveCaptions),
  };
  const lastReply = chat.isStreaming
    ? undefined
    : [...chat.session.messages]
        .reverse()
        .find(
          (message) => message.role === "assistant" || message.role === "user",
        );
  const turnError =
    lastReply?.status === "error"
      ? withAssistantTrace(
          lastReply.error ??
            "NyxBot could not complete this turn. Check which actions completed before trying again.",
          lastReply.turnId ?? null,
        )
      : undefined;
  const threadPanels =
    headerAgent && !showHome ? (
      <div className="shrink-0 px-4 pt-3 sm:px-6">
        <div className="mx-auto w-full max-w-[758px] space-y-2">
          <ThreadHeader
            name={agentName}
            handle={headerNamed ? agentHandle(headerNamed) : undefined}
            kind={headerAgent.kind}
            agentId={headerAgent.id}
            destroyed={destroyed}
            channelPlatform={conversation?.platform_thread?.platform ?? channelPlatform}
            onOpenDetails={() => setDetailsAgentId(headerAgent.id)}
          />
          {headerAgent.kind === "nyxbot" ? (
            <TeamStrip
              agents={agents.data?.agents ?? []}
              onOpenConversation={go}
            />
          ) : null}
          {conversation && !chat.isStreaming ? (
            <PendingEventsNote
              count={conversation.pending_events}
              agentName={agentName}
            />
          ) : null}
          {selectedId && !chat.isStreaming ? (
            <WaitingNote items={chat.waiting} agentName={agentName} />
          ) : null}
          {selectedId ? (
            <WorkingContextPanel
              conversationId={selectedId}
              onManageMemory={setDetailsAgentId}
              onReset={() => void chat.refreshHistory()}
            />
          ) : null}
        </div>
      </div>
    ) : null;

  return (
    <AssistantShell
      title={showHome ? "Home" : chat.session.title}
      titleKey={selectedId}
      onRenameTitle={
        selectedId && !destroyed
          ? (title) => chat.renameConversation(selectedId, title)
          : undefined
      }
      headerActions={<NyxBotSettingsButton />}
      sidebar={
        <AssistantEngineSidebar
          engine="nyxagent"
          conversations={[]}
          activeConversationId={selectedId}
          onNewChat={() => go()}
          onSelect={go}
          onDelete={chat.deleteConversation}
        />
      }
    >
      <div className="relative flex h-full min-h-0 flex-col bg-background">
        <AssistantErrorNotice
          message={
            chat.error ??
            (requestError?.scope === errorScope
              ? requestError.message
              : undefined) ??
            turnError
          }
        />
        {threadPanels}
        <AgentDetailsSheet
          agentId={openDetailsAgentId}
          agents={agents.data?.agents ?? []}
          proposalId={search.proposal}
          tab={search.details}
          open={openDetailsAgentId !== undefined}
          onOpenChange={(open) => {
            if (!open) {
              setDetailsAgentId(undefined);
              // Closing a linked sheet drops its URL state (push, like the
              // account panels, so Back after an explicit close reopens it).
              if (linkedDetails)
                navigateTo({
                  c: search.c,
                  agent: search.agent,
                  draft: search.draft ? true : undefined,
                });
            }
          }}
          onDeleted={() => {
            setDetailsAgentId(undefined);
            navigateTo({});
          }}
        />
        {creating === "agent" ? (
          <NewAgentDialog
            onClose={() => setCreating(undefined)}
            onCreated={(created) => {
              setCreating(undefined);
              navigateTo({ c: created.home_conversation_id });
            }}
          />
        ) : null}
        {creating === "group" ? (
          <NewGroupDialog
            agents={agents.data?.agents ?? []}
            onClose={() => setCreating(undefined)}
            onCreated={(group) => {
              setCreating(undefined);
              navigateTo({ g: group.id });
            }}
          />
        ) : null}
        {chat.beforeSeq && !showHome ? (
          <Button variant="ghost" onClick={() => void chat.loadOlder()}>
            Load earlier messages
          </Button>
        ) : null}
        {showHome ? (
          <NyxBotHome
            userName={user?.display_name ?? undefined}
            agents={agents.data?.agents ?? []}
            agentsLoading={agents.isPending}
            groups={(groups.data ?? []).map((group) =>
              groupWithDisplayNames(group, agents.data?.agents),
            )}
            bottomInset={composerHeight}
            onChat={(agent) => navigateTo({ agent: agent.id })}
            onOpenConversation={(id) => go(id)}
            onOpenGroup={(id) => navigateTo({ g: id })}
            onNewGroup={() => setCreating("group")}
            onNewAgent={() => setCreating("agent")}
            onManageMemory={() => {
              const nyxbot = nyxBotOf(agents.data?.agents);
              if (nyxbot) setDetailsAgentId(nyxbot.id);
            }}
          />
        ) : chat.isLoading && !chat.session.messages.length ? (
          <div className="flex flex-1 items-center justify-center text-12 text-text-tertiary">
            Loading conversation...
          </div>
        ) : (
          <AssistantLinkModalHost>
            {chat.session.conversationId ? (
              <ConversationMachineDesktops
                conversationId={chat.session.conversationId}
                turnActive={chat.isStreaming}
              />
            ) : null}
            <NyxAgentThreadMessageList
              session={voiceSession}
              renderMessage={(message) => {
                const caption = liveCaptions.find(
                  (item) => item.id === message.id,
                );
                if (caption)
                  return (
                    <VoiceCaptionText
                      caption={caption}
                      playbackMs={currentVoice?.activity?.playbackMs ?? 0}
                    />
                  );
                if (message.continuation_of)
                  return (
                    <p
                      role="status"
                      className="ml-[30px] px-[7px] text-11 text-muted-foreground"
                    >
                      Continuing after your decision
                    </p>
                  );
                if (message.role === "event")
                  return <NyxBotEventNotice message={message} />;
                if (message.role === "orchestrator") {
                  return <NyxBotOrchestratorMessage message={message} />;
                }
                const via =
                  message.role === "user"
                    ? (message.via ?? channelPlatform)
                    : null;
                if (via) {
                  return <ChannelUserEntry message={message} platform={via} />;
                }
                if (message.role === "assistant") {
                  return (
                    <NyxAgentAssistantEntry
                      message={
                        turnError && message.id === lastReply?.id
                          ? {
                              ...message,
                              error: undefined,
                              errorDetail: undefined,
                            }
                          : message
                      }
                      conversationId={chat.session.conversationId}
                      retry={retryFailedMessage}
                      checkRecovery={checkFailedMessage}
                    />
                  );
                }
                const approval = chat.approvals.find(
                  (row) => message.id === `nyxagent-approval:${row.id}`,
                );
                if (approval) {
                  return (
                    <ApprovalCard
                      block={{
                        type: "approval_card",
                        block_id: message.id,
                        approval_request_id: approval.id,
                        body: `${approval.service_name}: ${approval.summary}`,
                        service_slug: approval.service_slug,
                        agent_key_prefix: approval.agent_key_prefix,
                        approval_mode: approval.approval_mode,
                        grant_duration_sec: null,
                        expires_at: approval.expires_at,
                        decision: null,
                        decision_channel: null,
                      }}
                      onDecide={(approved) =>
                        chat.decideApproval(approval.id, approved)
                      }
                    />
                  );
                }
                const acknowledgement = chat.acknowledgements.find(
                  (row) => message.id === `nyxagent-acknowledgement:${row.id}`,
                );
                if (!acknowledgement) return undefined;
                return (
                  <ReviewCardTarget
                    selected={search.card === acknowledgement.id}
                  >
                    <NyxAgentAcknowledgementCard
                      acknowledgement={acknowledgement}
                      accessMode={accessMode}
                      deciding={Boolean(chat.decidingAcknowledgement)}
                      onDecision={async (choice, acceptReducedScope) => {
                        await chat.decideAcknowledgement({
                          id: acknowledgement.id,
                          choice,
                          acceptReducedScope,
                        });
                        setFocusRequest((value) => value + 1);
                      }}
                    />
                  </ReviewCardTarget>
                );
              }}
              // Everything renderMessage reads besides the message itself.
              projectionVersion={[
                `top-error:${turnError ? lastReply?.id : ""}`,
                `card:${search.card ?? ""}`,
                `deciding:${chat.decidingAcknowledgement ?? ""}`,
                `channel:${channelPlatform ?? ""}`,
                `mode:${accessMode}`,
                `voice:${voiceDeliveryRevision(liveCaptions, currentVoice?.activity.playbackMs ?? 0)}`,
                ...chat.acknowledgements.map(
                  (row) => `${row.id}:${row.status}`,
                ),
                ...chat.approvals.map((row) => `approval:${row.id}`),
              ].join(",")}
              bottomInset={composerHeight}
              // A linked review card may sit outside the mounted window.
              focusMessageId={
                search.card
                  ? `nyxagent-acknowledgement:${search.card}`
                  : undefined
              }
              emptyDescription={
                headerAgent?.kind === "specialist"
                  ? `Talk to ${agentName} directly. It handles your request itself, remembers across its threads, and tells you if it needs access you have not granted.`
                  : "See your connected services, connect a new one, " +
                    "set up a channel bot, or check approvals."
              }
            />
          </AssistantLinkModalHost>
        )}
        <div ref={composerRef} className="absolute inset-x-0 bottom-0 z-10">
          <NyxAgentComposer
            voiceControls={
              voiceEnabled &&
              voiceThread &&
              voiceThread === selectedId &&
              !destroyed &&
              !channelPlatform && (
                <div ref={voicePanelRef} className="pb-2">
                  <VoicePanel
                    key={voiceBinding!.token}
                    panelId={voiceBinding!.token}
                    preparation={voiceBinding!.preparation}
                    settingsRequest={voiceBinding!.settingsRequest}
                    preferencesReady={
                      !nyxBotSettings.isPending && !nyxBotSettings.isFetching
                    }
                    preferencesError={nyxBotSettings.isError}
                    onRetryPreferences={() => nyxBotSettings.refetch()}
                    threadId={voiceThread}
                    savedPreferences={nyxBotSettings.data?.voice}
                    onViewChange={receiveVoiceView}
                    onClose={() => {
                      setVoiceThread(undefined);
                      setFocusRequest((n) => n + 1);
                    }}
                  />
                </div>
              )
            }
            conversation={chat.conversation}
            agentModel={selectedAgent?.model}
            voiceActive={voiceMounted && currentVoice?.active}
            voiceActivity={voiceMounted ? currentVoice?.activity : undefined}
            key={`${user?.id}:${selectedId ?? headerAgent?.id ?? "draft"}`}
            onVoice={
              voiceEnabled && !destroyed && !channelPlatform
                ? async (id, preparation) => {
                    if (
                      preparation?.isClosed ||
                      selection.current !== selectedId ||
                      useAuthStore.getState().user?.id !== user?.id
                    ) {
                      preparation?.close();
                      return;
                    }
                    setVoiceThread(id, preparation);
                    adopt(id);
                  }
                : undefined
            }
            scope={{
              kind: "conversations",
              id: selectedId,
              agentId: headerAgent?.id,
            }}
            active={chat.isStreaming}
            sending={chat.isStreaming}
            disabled={Boolean(selectedId && chat.error) || destroyed}
            ownerUserId={user?.id ?? null}
            draftKey={
              selectedId
                ? `conv:${selectedId}`
                : headerAgent?.kind === "specialist"
                  ? `screen:nyxagent:agent:${headerAgent.id}`
                  : "screen:nyxagent:assistant"
            }
            focusRequest={focusRequest}
            agentOverride={agentOverride}
            placeholder={`Message ${agentName}`}
            onSend={async (text, uploads, model) => {
              const sending = { scope: errorScope };
              pendingSend.current = sending;
              setRequestError(undefined);
              try {
                await chat.send(text, uploads, model);
              } catch (error) {
                const message =
                  error instanceof Error
                    ? error.message
                    : "NyxBot is temporarily unavailable. Try again shortly.";
                setRequestError({ scope: sending.scope, message });
                toast.error(
                  isAdmittedSendError(error)
                    ? "Live reply interrupted"
                    : "Could not send your message",
                  {
                    description: assistantErrorSummary(message),
                  },
                );
                throw error;
              } finally {
                if (pendingSend.current === sending)
                  pendingSend.current = undefined;
              }
            }}
            onStop={async () => {
              setRequestError(undefined);
              try {
                await chat.stop();
              } catch (error) {
                const message =
                  error instanceof Error
                    ? error.message
                    : "Could not stop NyxBot. Try again shortly.";
                setRequestError({ scope: errorScope, message });
                toast.error("Could not stop NyxBot", {
                  description: assistantErrorSummary(message),
                });
              }
            }}
          />
        </div>
      </div>
    </AssistantShell>
  );
}
