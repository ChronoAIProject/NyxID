import { ConversationMachineDesktops } from "./machine-desktop-panel";
import { NyxAgentAcknowledgementCard } from "./nyxagent-acknowledgement-card";
import { NyxBotEventNotice, NyxBotOrchestratorMessage } from "./nyxbot-messages";
import { NyxBotSettingsButton } from "./nyxbot-settings-dialog";
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
import { LazyVoicePanel as VoicePanel } from "./lazy-voice-panel";
import { useFeature } from "@/hooks/use-feature-flag";
import { useNyxBotSettings } from "@/hooks/use-nyxbot-agents";
import { NyxBotHome } from "./nyxbot-home";
import { nyxBotOf, selectedAgentOf, useNyxBotAgents } from "@/hooks/use-nyxbot-agents";
import { useNyxBotGroups } from "@/hooks/use-nyxbot-groups";
import {
  agentHandle,
  agentTitle,
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
import { UploadComposer } from "@/components/assistant/upload-composer";
import { ChatComposer } from "@/components/assistant/chat-composer";
import { ChatMessageBubble, ChatMessageList } from "@/components/assistant/chat-message";
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
        to: "/assistant" as never,
        search: {
          c: conversationId,
          ...(fixtureMode ? { mock: 1 } : {}),
        } as never,
        replace: true,
      });
    },
    [fixtureMode, navigate],
  );
  const repairMissingConversation = useCallback(() => {
    void navigate({
      to: "/assistant" as never,
      search: (fixtureMode ? { mock: 1 } : {}) as never,
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
    void navigate({
      to: "/assistant" as never,
      search: {
        c: conversationId,
        ...(fixtureMode ? { mock: 1 } : {}),
      } as never,
    });
  }

  function createNewChat() {
    setComposerFocusRequest((value) => value + 1);
    chat.newChat();
    void navigate({
      to: "/assistant" as never,
      search: {
        draft: true,
        ...(fixtureMode ? { mock: 1 } : {}),
      } as never,
    });
  }

  async function deleteConversation(conversationId: string) {
    if (chat.isConversationStreaming(conversationId)) return;
    try {
      await chat.deleteConversation(conversationId);
      if (selectedId === conversationId) {
        void navigate({
          to: "/assistant" as never,
          search: (fixtureMode ? { mock: 1 } : {}) as never,
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
        to: "/assistant" as never,
        search: { c: conversationId } as never,
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
      to: "/assistant" as never,
      search: { draft: true } as never,
      replace: true,
    });
  }, [chat.isMissing, navigate]);

  function selectConversation(conversationId: string) {
    setComposerFocusRequest((value) => value + 1);
    void navigate({
      to: "/assistant" as never,
      search: { c: conversationId } as never,
    });
  }

  function createNewChat() {
    setComposerFocusRequest((value) => value + 1);
    void navigate({
      to: "/assistant" as never,
      search: { draft: true } as never,
    });
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
        <ChatMessageList
          session={chat.session}
          bottomInset={composerHeight}
          emptyDescription={DIRECT_MODE_COPY}
        />
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
  const voiceSettings = useNyxBotSettings(voiceEnabled);
  const [voiceThread, setVoiceThread] = useState<string>();
  const navigate = useNavigate();
  const user = useAuthStore((state) => state.user);
  const search = useRouterState({
    select: (state) => parseAssistantSearch(state.location.search as Record<string, unknown>),
  });
  const selectedId = search.draft ? undefined : search.c;
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
  const adopt = useCallback((id: string) => {
    if (selection.current !== selectedId) return;
    void navigate({
      to: "/assistant" as never,
      search: { c: id, ...(search.mock ? { mock: 1 } : {}) } as never,
      replace: true,
    });
  }, [navigate, search.mock, selectedId]);
  const agents = useNyxBotAgents();
  const groups = useNyxBotGroups();
  // The agent whose details sheet is open (the thread's agent, or NyxBot from home).
  const [detailsAgentId, setDetailsAgentId] = useState<string>();
  const [creating, setCreating] = useState<"agent" | "group">();
  // The open thread's agent, else the agent in the URL, else NyxBot.
  const threadAgent = nyxAgentTransport.getConversation(selectedId)?.agent ?? undefined;
  const selectedAgent = selectedAgentOf(agents.data?.agents, threadAgent?.id, search.agent);
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
    onConversationAdopted: adopt,
    threadsAgentId: selectedAgent?.id,
    draftAgent,
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
      to: "/assistant" as never,
      search: { c: latestThreadId, ...(search.mock ? { mock: 1 } : {}) } as never,
      replace: true,
    });
  }, [
    chat.threadsLoaded,
    latestThreadId,
    navigate,
    onChatRoute,
    search.agent,
    search.c,
    search.draft,
    search.mock,
  ]);

  function navigateTo(next: { c?: string; draft?: true; agent?: string; g?: string }) {
    void navigate({
      to: "/assistant" as never,
      search: { ...next, ...(search.mock ? { mock: 1 } : {}) } as never,
    });
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
  // The home stays until the first message of its new NyxBot thread shows.
  const showHome = home && !chat.isStreaming && !chat.session.messages.length;
  const threadPanels = headerAgent && !showHome ? (
    <div className="shrink-0 px-4 pt-3 sm:px-6">
      <div className="mx-auto w-full max-w-[758px] space-y-2">
        <ThreadHeader
          name={agentName}
          handle={headerNamed ? agentHandle(headerNamed) : undefined}
          kind={headerAgent.kind}
          agentId={headerAgent.id}
          destroyed={destroyed}
          channelPlatform={channelPlatform}
          onOpenDetails={() => setDetailsAgentId(headerAgent.id)}
        />
        {headerAgent.kind === "nyxbot" ? (
          <TeamStrip agents={agents.data?.agents ?? []} onOpenConversation={go} />
        ) : null}
        {conversation && !chat.isStreaming ? (
          <PendingEventsNote count={conversation.pending_events} agentName={agentName} />
        ) : null}
        {selectedId && !chat.isStreaming ? (
          <WaitingNote items={chat.waiting} agentName={agentName} />
        ) : null}
      </div>
    </div>
  ) : null;

  return (
    <AssistantShell
      title={showHome ? "Home" : chat.session.title}
      titleKey={selectedId}
      onRenameTitle={selectedId && !destroyed
        ? (title) => chat.renameConversation(selectedId, title)
        : undefined}
      headerActions={<NyxBotSettingsButton />}
      sidebar={
        <AssistantEngineSidebar
          engine="nyxagent"
          conversations={[]}
          activeConversationId={selectedId}
          onNewChat={() => go()}
          onSelect={go}
          onDelete={chat.deleteConversation}
          notice={chat.error}
        />
      }
    >
      <div className="relative flex h-full min-h-0 flex-col bg-background">
        {threadPanels}
        <AgentDetailsSheet
          agentId={detailsAgentId}
          agents={agents.data?.agents ?? []}
          open={detailsAgentId !== undefined}
          onOpenChange={(open) => {
            if (!open) setDetailsAgentId(undefined);
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
            {chat.isStreaming && chat.continuations > 0 ? <p role="status" className="mx-auto w-full max-w-[758px] px-4 pb-2 text-11 text-muted-foreground">Continuing task…</p> : null}
            {chat.session.conversationId ? <ConversationMachineDesktops conversationId={chat.session.conversationId} turnActive={chat.isStreaming} /> : null}
            <ChatMessageList
              session={chat.session}
              renderMessage={(message) => {
                if (message.role === "event") return <NyxBotEventNotice message={message} />;
                if (message.role === "orchestrator") {
                  return <NyxBotOrchestratorMessage message={message} />;
                }
                const via = message.role === "user" ? (message.via ?? channelPlatform) : null;
                if (via) {
                  // The user wrote this in a chat app, not here.
                  return (
                    <div className="flex flex-col items-end gap-1">
                      <ChannelBadge platform={via} />
                      <ChatMessageBubble message={message} />
                    </div>
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
                      onDecide={(approved) => chat.decideApproval(approval.id, approved)}
                    />
                  );
                }
                const acknowledgement = chat.acknowledgements.find((row) =>
                  message.id === `nyxagent-acknowledgement:${row.id}`,
                );
                if (!acknowledgement) return undefined;
                return (
                  <NyxAgentAcknowledgementCard
                    acknowledgement={acknowledgement}
                    deciding={Boolean(chat.decidingAcknowledgement)}
                    onDecision={async (choice) => {
                      await chat.decideAcknowledgement({ id: acknowledgement.id, choice });
                      setFocusRequest((value) => value + 1);
                    }}
                  />
                );
              }}
              projectionVersion={[
                ...chat.acknowledgements.map((row) => `${row.id}:${row.status}`),
                ...chat.approvals.map((row) => `approval:${row.id}`),
              ].join(",")}
              bottomInset={composerHeight}
              notice={chat.error}
              emptyDescription={
                headerAgent?.kind === "specialist"
                  ? `Talk to ${agentName} directly. It remembers across its threads and asks NyxBot for anything outside its grants.`
                  : "See your connected services, connect a new one, " +
                    "set up a channel bot, or check approvals."
              }
            />
          </AssistantLinkModalHost>
        )}
        {voiceEnabled && voiceThread && voiceThread === selectedId && !destroyed && !channelPlatform && (
          <div className="absolute inset-x-0 top-3 z-20 mx-auto w-full max-w-[758px] px-4">
            <VoicePanel key={`${user?.id}:${voiceThread}`} threadId={voiceThread} savedPreferences={voiceSettings.data?.voice}
              onClose={() => { setVoiceThread(undefined); setFocusRequest((n) => n + 1); }} />
          </div>
        )}
        <div ref={composerRef} className="absolute inset-x-0 bottom-0 z-10">
          <UploadComposer
            key={`${user?.id}:${selectedId ?? headerAgent?.id ?? "draft"}`}
            onVoice={voiceEnabled && !destroyed && !channelPlatform ? async (id) => {
              if (selection.current !== selectedId) return;
              setVoiceThread(id);
              adopt(id);
            } : undefined}
            scope={{ kind: "conversations", id: selectedId, agentId: headerAgent?.id }}
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
            // Model routing is server-side: there is no profile selector here.
            placeholder={`Message ${agentName}`}
            onSend={async (text, uploads) => {
              try {
                await chat.send(text, uploads);
              } catch (error) {
                toast.error(
                  error instanceof Error ? error.message : "The assistant is unavailable.",
                );
                throw error;
              }
            }}
            onStop={async () => {
              try {
                await chat.stop();
              } catch {
                toast.error("Could not stop the assistant. Try again.");
              }
            }}
          />
        </div>
      </div>
    </AssistantShell>
  );
}
