import type { AnchorHTMLAttributes, ReactNode } from "react";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { TooltipProvider } from "@/components/ui/tooltip";
import { useAssistantContextStore } from "@/stores/assistant-context-store";
import { useAssistantDraftStore } from "@/stores/assistant-draft-store";
import { useAuthStore } from "@/stores/auth-store";
import type { Conversation } from "@/types/assistant";
import type { User } from "@/types/api";

const {
  mockNavigate,
  mockCreateMutateAsync,
  mockSendMutateAsync,
  mockToastError,
  state,
} = vi.hoisted(() => ({
  mockNavigate: vi.fn(),
  mockCreateMutateAsync: vi.fn(),
  mockSendMutateAsync: vi.fn(),
  mockToastError: vi.fn(),
  state: {
    pathname: "/assistant",
    search: {} as Record<string, unknown>,
    conversations: [] as Conversation[] | undefined,
    conversationsResolved: true,
    historyError: false,
  },
}));

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
  useRouterState: ({
    select,
  }: {
    select: (routerState: {
      location: { search: Record<string, unknown> };
    }) => unknown;
  }) => select({ location: { search: state.search } }),
  Link: ({
    to,
    children,
    ...rest
  }: AnchorHTMLAttributes<HTMLAnchorElement> & {
    readonly to?: string;
    readonly children?: ReactNode;
  }) => (
    <a data-to={to} {...rest}>
      {children}
    </a>
  ),
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

vi.mock("@/hooks/use-assistant", () => ({
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
    isError: state.historyError,
    error: undefined,
  }),
  useAssistantTurn: () => ({ data: null }),
  useCreateConversation: () => ({
    mutateAsync: mockCreateMutateAsync,
    isPending: false,
  }),
  useSendMessage: () => ({
    mutateAsync: mockSendMutateAsync,
    isPending: false,
  }),
  useCancelTurn: () => ({ mutateAsync: vi.fn() }),
  useDecideApproval: () => ({ mutateAsync: vi.fn() }),
  useActionCardActions: () => ({
    setInProgress: vi.fn(),
    continueAction: vi.fn(),
  }),
  useDeleteConversation: () => ({
    mutateAsync: vi.fn(),
    isPending: false,
    variables: undefined,
  }),
  useWorkspaceCounts: () => ({ data: { artifacts: 0, pendingApprovals: 0 } }),
  describeSendFailure: () => ({
    message: "Message not sent",
    description: "",
  }),
  describeHistoryError: () => "This conversation has no saved transcript yet.",
}));

vi.mock("sonner", () => ({
  toast: { error: mockToastError },
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

const existingConversation: Conversation = {
  id: "conv-1",
  title: "Quarterly digest",
  created_at: "2026-07-20T00:00:00.000Z",
  last_message_at: "2026-07-20T00:05:00.000Z",
};

const boundConversation: Conversation = {
  id: "conversation-keys",
  title: "Keys",
  created_at: "2026-07-29T00:00:00.000Z",
  last_message_at: "2026-07-29T00:00:00.000Z",
};

function renderPage() {
  return render(
    <TooltipProvider>
      <AssistantPage />
    </TooltipProvider>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  state.pathname = "/assistant";
  state.search = {};
  state.conversations = [existingConversation];
  state.conversationsResolved = true;
  state.historyError = false;
  useAuthStore.setState({
    user,
    isAuthenticated: true,
    isLoading: false,
    mfaRequired: false,
  });
  useAssistantContextStore.setState({
    ownerUserId: user.id,
    lastScreen: null,
    bindings: {},
  });
  useAssistantDraftStore.setState({ ownerUserId: user.id, drafts: {} });
});

describe("AssistantPage new chat", () => {
  it("navigates to the draft thread", async () => {
    const event = userEvent.setup();
    renderPage();

    await event.click(screen.getByRole("button", { name: /New chat/ }));

    expect(mockNavigate).toHaveBeenCalledWith(
      expect.objectContaining({ search: { draft: true } }),
    );
  });

  it("provisions nothing on click because allocation belongs to first send", async () => {
    const event = userEvent.setup();
    renderPage();
    mockNavigate.mockClear();

    await event.click(screen.getByRole("button", { name: /New chat/ }));

    expect(mockCreateMutateAsync).not.toHaveBeenCalled();
    expect(mockSendMutateAsync).not.toHaveBeenCalled();
    expect(mockNavigate).toHaveBeenCalledTimes(1);
  });

  it("allocates on first send and follows the returned conversation", async () => {
    const event = userEvent.setup();
    mockSendMutateAsync.mockResolvedValue({ conversationId: "conv-new" });
    state.search = { draft: true };
    renderPage();

    await event.type(screen.getByRole("textbox"), "hello");
    await event.click(screen.getByRole("button", { name: /send/i }));

    await waitFor(() => {
      expect(mockSendMutateAsync).toHaveBeenCalledWith("hello");
    });
    expect(mockNavigate).toHaveBeenCalledWith(
      expect.objectContaining({ search: { c: "conv-new" } }),
    );
  });

  it("renders the empty draft thread rather than a binding or newest chat", () => {
    state.search = { draft: true };
    useAssistantContextStore.setState({
      ownerUserId: user.id,
      lastScreen: "/keys",
      bindings: {
        "/keys": {
          conversationId: existingConversation.id,
          updatedAt: 1,
        },
      },
    });
    renderPage();

    expect(
      screen.getByRole("heading", { name: "New chat" }),
    ).toBeInTheDocument();
  });

  it("reports a failed transcript read without taking the chat away", () => {
    state.historyError = true;
    state.search = { c: existingConversation.id };
    renderPage();

    expect(screen.getByRole("status")).toHaveTextContent(
      /no saved transcript yet/i,
    );
    expect(screen.getByRole("textbox")).toBeInTheDocument();
  });

  it("still falls back to the newest chat with no recorded entry screen", () => {
    renderPage();

    expect(
      screen.getByRole("heading", { name: existingConversation.title }),
    ).toBeInTheDocument();
  });
});

describe("AssistantPage conversation resolution", () => {
  it("migrates the live screen draft when a cold list resolves its binding", async () => {
    state.conversations = undefined;
    state.conversationsResolved = false;
    useAssistantContextStore.setState({
      ownerUserId: user.id,
      lastScreen: "/keys",
      bindings: {
        "/keys": {
          conversationId: boundConversation.id,
          updatedAt: 1,
        },
      },
    });
    const { rerender } = renderPage();
    fireEvent.change(screen.getByRole("textbox"), {
      target: { value: "Question typed while conversations load" },
    });

    state.conversations = [boundConversation];
    state.conversationsResolved = true;
    rerender(
      <TooltipProvider>
        <AssistantPage />
      </TooltipProvider>,
    );

    await waitFor(() => {
      expect(screen.getByRole("textbox")).toHaveValue(
        "Question typed while conversations load",
      );
    });
    expect(useAssistantDraftStore.getState().getDraft("screen:/keys")).toBe("");
    expect(
      useAssistantDraftStore
        .getState()
        .getDraft(`conv:${boundConversation.id}`),
    ).toBe("Question typed while conversations load");
  });

  it("removes a stale explicit conversation and falls through to blank for its screen", async () => {
    state.search = { c: "deleted-conversation" };
    state.conversations = undefined;
    state.conversationsResolved = false;
    useAssistantContextStore.setState({
      ownerUserId: user.id,
      lastScreen: "/nodes",
      bindings: {},
    });
    const { rerender } = renderPage();

    state.conversations = [boundConversation];
    state.conversationsResolved = true;
    rerender(
      <TooltipProvider>
        <AssistantPage />
      </TooltipProvider>,
    );

    await waitFor(() => {
      expect(mockNavigate).toHaveBeenCalledWith({
        to: "/assistant",
        search: {},
        replace: true,
      });
    });
    expect(
      screen.getByRole("heading", { name: "New chat" }),
    ).toBeInTheDocument();
  });
});
