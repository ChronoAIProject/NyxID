import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactNode } from "react";
import { AssistantPage } from "./assistant";

const testState = vi.hoisted(() => ({
  search: {} as Record<string, unknown>,
  navigate: vi.fn(),
  useConversation: vi.fn(),
  conversation: {
    id: "conversation-1",
    title: "Existing chat",
    created_at: "2026-07-24T00:00:00.000Z",
    last_message_at: "2026-07-24T00:05:00.000Z",
  },
}));

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => testState.navigate,
  useRouterState: ({
    select,
  }: {
    readonly select: (state: {
      readonly location: { readonly search: Record<string, unknown> };
    }) => unknown;
  }) => select({ location: { search: testState.search } }),
}));

vi.mock("@/components/assistant/assistant-shell", () => ({
  AssistantShell: ({
    title,
    sidebar,
    children,
  }: {
    readonly title: string;
    readonly sidebar: ReactNode;
    readonly children: ReactNode;
  }) => (
    <div>
      <h1>{title}</h1>
      {sidebar}
      {children}
    </div>
  ),
}));

vi.mock("@/components/assistant/assistant-sidebar", () => ({
  AssistantSidebar: ({
    activeConversationId,
    onNewChat,
  }: {
    readonly activeConversationId?: string;
    readonly onNewChat: () => void;
  }) => (
    <button
      type="button"
      data-active-conversation={activeConversationId ?? ""}
      onClick={onNewChat}
    >
      New chat
    </button>
  ),
}));

vi.mock("@/components/assistant/chat-composer", () => ({
  ChatComposer: () => null,
}));

vi.mock("@/components/assistant/chat-thread", () => ({
  ChatThread: () => <div>Chat thread</div>,
}));

vi.mock("@/components/assistant/approvals-view", () => ({
  ApprovalsView: () => null,
}));

vi.mock("@/components/assistant/plugins-view", () => ({
  PluginsView: () => null,
}));

vi.mock("@/hooks/use-assistant", () => ({
  describeSendFailure: () => ({ message: "failed", description: "failed" }),
  useConversations: () => ({ data: [testState.conversation] }),
  useConversation: (conversationId: string | undefined) => {
    testState.useConversation(conversationId);
    return {
      data: conversationId
        ? { conversation: testState.conversation, messages: [] }
        : undefined,
      isLoading: false,
      isError: false,
    };
  },
  useAssistantTurn: () => ({ data: null }),
  useSendMessage: () => ({ isPending: false, mutateAsync: vi.fn() }),
  useCancelTurn: () => ({ mutateAsync: vi.fn() }),
  useDecideApproval: () => ({ mutateAsync: vi.fn() }),
  useDeleteConversation: () => ({
    isPending: false,
    variables: undefined,
    mutateAsync: vi.fn(),
  }),
}));

describe("AssistantPage new chat", () => {
  beforeEach(() => {
    testState.search = {};
    testState.navigate.mockReset();
    testState.useConversation.mockReset();
  });

  it("navigates to the local new-chat state without waiting for a backend create", async () => {
    const user = userEvent.setup();
    render(<AssistantPage />);

    await user.click(screen.getByRole("button", { name: "New chat" }));

    expect(testState.navigate).toHaveBeenCalledWith({
      to: "/assistant",
      search: { c: "new" },
    });
  });

  it("does not fall back to an existing conversation in the new-chat state", () => {
    testState.search = { c: "new" };

    render(<AssistantPage />);

    expect(testState.useConversation).toHaveBeenCalledWith(undefined);
    expect(screen.getByRole("heading", { name: "New chat" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "New chat" })).toHaveAttribute(
      "data-active-conversation",
      "",
    );
  });
});
