import type { ReactNode } from "react";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Conversation } from "@/types/assistant";
import type { User } from "@/types/api";
import { useAssistantContextStore } from "@/stores/assistant-context-store";
import { useAssistantDraftStore } from "@/stores/assistant-draft-store";
import { useAuthStore } from "@/stores/auth-store";

const { mockNavigate, state } = vi.hoisted(() => ({
  mockNavigate: vi.fn(),
  state: {
    search: {} as { c?: string },
    conversations: undefined as Conversation[] | undefined,
    conversationsResolved: false,
  },
}));

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
  useRouterState: ({
    select,
  }: {
    readonly select: (routerState: {
      location: { search: { c?: string } };
    }) => string | undefined;
  }) => select({ location: { search: state.search } }),
}));

vi.mock("@/components/assistant/assistant-shell", () => ({
  AssistantShell: ({ children }: { readonly children: ReactNode }) => children,
}));
vi.mock("@/components/assistant/assistant-sidebar", () => ({
  AssistantSidebar: () => <div data-testid="assistant-sidebar" />,
}));
vi.mock("@/components/assistant/chat-thread", () => ({
  ChatThread: () => <div data-testid="chat-thread" />,
}));
vi.mock("@/components/assistant/approvals-view", () => ({
  ApprovalsView: () => <div />,
}));
vi.mock("@/components/assistant/plugins-view", () => ({
  PluginsView: () => <div />,
}));

vi.mock("@/hooks/use-assistant", () => ({
  describeSendFailure: () => ({ message: "Failed", description: "Failed" }),
  useConversations: () => ({
    data: state.conversations,
    isSuccess: state.conversationsResolved,
  }),
  useConversation: (conversationId: string | undefined) => ({
    data: conversationId
      ? {
          conversation: state.conversations?.find(
            (conversation) => conversation.id === conversationId,
          ) ?? {
            id: conversationId,
            title: "Loading",
            created_at: "2026-07-29T00:00:00.000Z",
            last_message_at: "2026-07-29T00:00:00.000Z",
          },
          messages: [],
          has_more: false,
        }
      : undefined,
    isLoading: false,
    isError: false,
  }),
  useAssistantTurn: () => ({ data: null }),
  useCreateConversation: () => ({
    isPending: false,
    mutateAsync: vi.fn(),
  }),
  useSendMessage: () => ({ isPending: false, mutateAsync: vi.fn() }),
  useCancelTurn: () => ({ mutateAsync: vi.fn() }),
  useDecideApproval: () => ({ mutateAsync: vi.fn() }),
  useDeleteConversation: () => ({
    isPending: false,
    variables: undefined,
    mutateAsync: vi.fn(),
  }),
}));

import { AssistantPage } from "./assistant";

const user: User = {
  id: "user-1",
  email: "user@example.com",
  display_name: "User",
  avatar_url: null,
  email_verified: true,
  mfa_enabled: false,
  is_admin: false,
  is_active: true,
  created_at: "2026-07-29T00:00:00.000Z",
};

const conversation: Conversation = {
  id: "conversation-keys",
  title: "Keys",
  created_at: "2026-07-29T00:00:00.000Z",
  last_message_at: "2026-07-29T00:00:00.000Z",
};

describe("AssistantPage conversation resolution", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
    state.search = {};
    state.conversations = undefined;
    state.conversationsResolved = false;
    useAuthStore.setState({
      user,
      isAuthenticated: true,
      isLoading: false,
      mfaRequired: false,
    });
    useAssistantContextStore.setState({
      ownerUserId: user.id,
      lastScreen: "/keys",
      bindings: {
        "/keys": {
          conversationId: conversation.id,
          updatedAt: 1,
        },
      },
    });
    useAssistantDraftStore.setState({ ownerUserId: user.id, drafts: {} });
  });

  it("migrates the live screen draft when a cold list resolves its binding", async () => {
    const { rerender } = render(<AssistantPage />);
    const input = screen.getByRole("textbox");
    fireEvent.change(input, {
      target: { value: "Question typed while conversations load" },
    });

    state.conversations = [conversation];
    state.conversationsResolved = true;
    rerender(<AssistantPage />);

    await waitFor(() => {
      expect(screen.getByRole("textbox")).toHaveValue(
        "Question typed while conversations load",
      );
    });
    expect(
      useAssistantDraftStore.getState().getDraft("screen:/keys"),
    ).toBe("");
    expect(
      useAssistantDraftStore
        .getState()
        .getDraft(`conv:${conversation.id}`),
    ).toBe("Question typed while conversations load");
  });

  it("removes a stale explicit conversation after the list resolves", async () => {
    state.search = { c: "deleted-conversation" };
    useAssistantContextStore.setState({
      ownerUserId: user.id,
      lastScreen: "/nodes",
      bindings: {},
    });
    const { rerender } = render(<AssistantPage />);

    state.conversations = [conversation];
    state.conversationsResolved = true;
    rerender(<AssistantPage />);

    await waitFor(() => {
      expect(mockNavigate).toHaveBeenCalledWith({
        to: "/assistant",
        search: {},
        replace: true,
      });
    });
  });
});
