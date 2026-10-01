import { AutomationsPage } from "@/pages/automations";
import { MachinesPage } from "@/pages/machines";
import { MachineSetupPage, MachinePairPage } from "@/pages/machine-setup";
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
  NyxAgentAssistantChatPage,
} from "@/components/assistant/assistant-chat-page";
import { AssistantShell } from "@/components/assistant/assistant-shell";
import { AssistantEngineSidebar } from "@/components/assistant/assistant-engine-sidebar";
import { AssistantWireLogAction } from "@/components/assistant/assistant-wire-log-panel";
import { NyxBotSettingsButton } from "@/components/assistant/nyxbot-settings-dialog";
import { PluginsView } from "@/components/assistant/plugins-view";
import { useAssistantChat } from "@/hooks/use-assistant-chat";
import { useDirectAssistantChat } from "@/hooks/use-assistant-direct";
import { useFeature } from "@/hooks/use-feature-flag";
import {
  assistantChatSurface,
  isDirectConversationId,
} from "@/lib/assistant/conversation-ids";
import { directAssistantTransport } from "@/lib/assistant/direct-transport";
import { nyxAgentTransport } from "@/lib/assistant/nyxagent-transport";
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

type WorkspaceView =
  | "plugins"
  | "approvals"
  | "automations"
  | "machines"
  | "machine-setup"
  | "machine-pair";

const workspaceTitles: Record<WorkspaceView, string> = {
  plugins: "Plugins",
  approvals: "Approvals",
  automations: "Automations",
  machines: "Machines",
  "machine-setup": "Add a machine",
  "machine-pair": "Pair a machine",
};

function workspaceActiveView(view: WorkspaceView) {
  return view === "machine-setup" || view === "machine-pair"
    ? "machines"
    : view;
}

function WorkspaceContent({ view }: { readonly view: WorkspaceView }) {
  if (view === "plugins") return <PluginsView />;
  if (view === "approvals") return <ApprovalsView />;
  return (
    <div
      className="h-full overflow-y-auto px-4 py-6 sm:px-6 lg:px-10"
      style={{ paddingBottom: "max(2rem, var(--sab))" }}
    >
      {view === "automations" ? (
        <AutomationsPage />
      ) : view === "machines" ? (
        <MachinesPage />
      ) : view === "machine-setup" ? (
        <MachineSetupPage />
      ) : (
        <MachinePairPage />
      )}
    </div>
  );
}

function AssistantWorkspacePage({
  view,
  directEnabled,
}: {
  readonly view: WorkspaceView;
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

  const title = workspaceTitles[view];
  const sidebar = (
    <AssistantEngineSidebar engine="actor"
      conversations={conversations}
      activeConversationId={undefined}
      activeView={workspaceActiveView(view)}
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
      <WorkspaceContent view={view} />
    </AssistantShell>
  );
}

/**
 * Workspace views for NyxAgent users: the NyxAgent sidebar (agents and
 * groups), and none of the earlier engines' chat lists are fetched.
 */
function NyxAgentWorkspacePage({ view }: { readonly view: WorkspaceView }) {
  const navigate = useNavigate();
  function openAssistant(search: { c?: string } = {}) {
    void navigate({ to: "/assistant" as never, search: search as never });
  }
  return (
    <AssistantShell
      title={workspaceTitles[view]}
      headerActions={<NyxBotSettingsButton />}
      sidebar={
        <AssistantEngineSidebar
          engine="nyxagent"
          conversations={[]}
          activeConversationId={undefined}
          activeView={workspaceActiveView(view)}
          onNewChat={() => openAssistant()}
          onSelect={(id) => openAssistant({ c: id })}
          onDelete={(id) => nyxAgentTransport.delete(id)}
        />
      }
    >
      <WorkspaceContent view={view} />
    </AssistantShell>
  );
}

export function AssistantPage({
  view = "chat",
}: {
  readonly view?: "chat" | WorkspaceView;
}) {
  const directEnabled = useFeature(FEATURE_FLAG.DIRECT_CHAT_ENGINE);
  const nyxagentEnabled = useFeature(FEATURE_FLAG.NYXAGENT_ENGINE);
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
    const workspace = nyxagentEnabled ? (
      <NyxAgentWorkspacePage view={view} />
    ) : (
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
  const surface = assistantChatSurface({
      nyxagentEnabled,
      directEnabled,
      drafting,
      selectedConversationId,
    });
  if (surface === "nyxagent") return <NyxAgentAssistantChatPage />;
  if (surface === "direct") {
    return <DirectAssistantChatPage />;
  }
  return <AssistantChatPage />;
}
