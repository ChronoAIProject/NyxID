import {
  lazy,
  Suspense,
  useCallback,
  type ComponentType,
  type LazyExoticComponent,
} from "react";
import { useNavigate, useRouterState } from "@tanstack/react-router";
import { toast } from "sonner";
import { ApprovalsView } from "@/components/assistant/approvals-view";
import {
  AssistantChatPage,
  DirectAssistantChatPage,
} from "@/components/assistant/assistant-chat-page";
import { AssistantShell } from "@/components/assistant/assistant-shell";
import { AssistantSidebar } from "@/components/assistant/assistant-sidebar";
import { AssistantWireLogAction } from "@/components/assistant/assistant-wire-log-panel";
import { PluginsView } from "@/components/assistant/plugins-view";
import { useAssistantChat } from "@/hooks/use-assistant-chat";
import { useDirectAssistantChat } from "@/hooks/use-assistant-direct";
import { useFeature } from "@/hooks/use-feature-flag";
import {
  assistantChatSurface,
  isDirectConversationId,
} from "@/lib/assistant/conversation-ids";
import { directAssistantTransport } from "@/lib/assistant/direct-transport";
import { parseAssistantSearch } from "@/lib/assistant/search";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import type { Conversation } from "@/types/assistant";

const MockScenariosAction = import.meta.env.DEV
  ? lazy(() =>
      import("@/components/assistant/mock-scenarios-action").then((module) => ({
        default: module.MockScenariosAction,
      })),
    )
  : null;

const AssistantHttpFixturePage = import.meta.env.DEV
  ? lazy(() =>
      import("@/components/assistant/assistant-http-fixture-page").then(
        (module) => ({ default: module.AssistantHttpFixturePage }),
      ),
    )
  : null;

const AssistantHttpFixtureBoundary = import.meta.env.DEV
  ? lazy(() =>
      import("@/components/assistant/assistant-http-fixture-page").then(
        (module) => ({ default: module.AssistantHttpFixtureBoundary }),
      ),
    )
  : null;

type ScenarioActionComponent =
  | ComponentType
  | LazyExoticComponent<ComponentType>;

export function AssistantHeaderActions({
  scenarioAction = MockScenariosAction,
  activeConversationId = null,
}: {
  readonly scenarioAction?: ScenarioActionComponent | null;
  readonly activeConversationId?: string | null;
} = {}) {
  const ScenarioAction = scenarioAction;
  return (
    <>
      {ScenarioAction ? (
        <Suspense fallback={null}>
          <ScenarioAction />
        </Suspense>
      ) : null}
      <AssistantWireLogAction activeConversationId={activeConversationId} />
    </>
  );
}

function fixtureMode(): boolean {
  if (import.meta.env.MODE === "test") return true;
  return Boolean(
    import.meta.env.DEV &&
      typeof window !== "undefined" &&
      new URLSearchParams(window.location.search).get("mock") === "1",
  );
}

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

function AssistantWorkspacePage({
  view,
  directEnabled,
}: {
  readonly view: "plugins" | "approvals";
  readonly directEnabled: boolean;
}) {
  const navigate = useNavigate();
  const noopAdoption = useCallback(() => undefined, []);
  const actorChat = useAssistantChat({
    onConversationAdopted: noopAdoption,
    onConversationMissing: noopAdoption,
  });
  const directChat = useDirectAssistantChat({
    onConversationAdopted: noopAdoption,
  });
  const conversations: Conversation[] = [
    ...actorChat.visibleConversations.map(sidebarConversation),
    ...(directEnabled ? directChat.conversations : []),
  ].sort((left, right) =>
    right.last_message_at.localeCompare(left.last_message_at),
  );

  function createNewChat() {
    void navigate({
      to: "/assistant" as never,
      search: { draft: true } as never,
    });
  }

  function selectConversation(conversationId: string) {
    void navigate({
      to: "/assistant" as never,
      search: { c: conversationId } as never,
    });
  }

  async function deleteConversation(conversationId: string) {
    if (isDirectConversationId(conversationId)) {
      const turn = directAssistantTransport.getHistorySnapshot(conversationId)
        ?.activeTurn;
      if (turn?.status === "running" || turn?.status === "waiting") return;
    } else if (actorChat.isConversationStreaming(conversationId)) {
      return;
    }
    try {
      if (isDirectConversationId(conversationId)) {
        await directChat.deleteConversation(conversationId);
      } else {
        await actorChat.deleteConversation(conversationId);
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

  const title = view === "plugins" ? "Plugins" : "Approvals";
  const sidebar = (
    <AssistantSidebar
      conversations={conversations}
      activeConversationId={undefined}
      activeView={view}
      notice={
        actorChat.listError
          ? `Could not load chats. ${actorChat.listError}`
          : undefined
      }
      onNewChat={createNewChat}
      onSelect={selectConversation}
      onDelete={deleteConversation}
    />
  );

  return (
    <AssistantShell
      title={title}
      sidebar={sidebar}
      headerActions={<AssistantHeaderActions activeConversationId={null} />}
    >
      {view === "plugins" ? <PluginsView /> : <ApprovalsView />}
    </AssistantShell>
  );
}

export function AssistantPage({
  view = "chat",
}: {
  readonly view?: "chat" | "plugins" | "approvals";
}) {
  const directEnabled = useFeature(FEATURE_FLAG.DIRECT_CHAT_ENGINE);
  const selectedConversationId = useRouterState({
    select: (state) =>
      parseAssistantSearch(state.location.search as Record<string, unknown>).c,
  });
  const drafting = useRouterState({
    select: (state) =>
      parseAssistantSearch(state.location.search as Record<string, unknown>)
        .draft === true,
  });

  if (view !== "chat") {
    const workspace = (
      <AssistantWorkspacePage view={view} directEnabled={directEnabled} />
    );
    return AssistantHttpFixtureBoundary && fixtureMode() ? (
      <Suspense fallback={null}>
        <AssistantHttpFixtureBoundary>
          {workspace}
        </AssistantHttpFixtureBoundary>
      </Suspense>
    ) : (
      workspace
    );
  }
  if (AssistantHttpFixturePage && fixtureMode()) {
    return (
      <Suspense fallback={null}>
        <AssistantHttpFixturePage />
      </Suspense>
    );
  }
  if (
    assistantChatSurface({
      directEnabled,
      drafting,
      selectedConversationId,
    }) === "direct"
  ) {
    return <DirectAssistantChatPage />;
  }
  return <AssistantChatPage />;
}
