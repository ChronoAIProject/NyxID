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
import { AssistantSidebar } from "@/components/assistant/assistant-sidebar";
import { AssistantWireLogAction } from "@/components/assistant/assistant-wire-log-panel";
import { ChatActorControls } from "@/components/assistant/chat-actor-controls";
import { ChatComposer } from "@/components/assistant/chat-composer";
import { ChatMessageList } from "@/components/assistant/chat-message";
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
    <AssistantSidebar
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
          <div className="flex flex-1 items-center justify-center text-[12px] text-text-tertiary">
            Loading conversation...
          </div>
        ) : (
          <ChatMessageList
            session={chat.session}
            bottomInset={composerHeight}
            footer={actorControls}
            notice={threadNotice}
            projectionVersion={`${String(chat.projection?.stateVersion ?? 0)}:${String(chat.projection?.progressSequence ?? 0)}`}
          />
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
    <AssistantSidebar
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
