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
    // Mirrors what TanStack exposes for an in-flight mutation.
    sendPending: undefined as string | undefined,
    historyMessages: [] as unknown[],
    episode: null as {
      open: boolean;
      printed: boolean;
      projecting: boolean;
    } | null,
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
          messages: state.historyMessages,
          has_more: false,
        }
      : undefined,
    isLoading: false,
    isFetching: false,
    isError: state.historyError,
    error: undefined,
  }),
  useAssistantTurn: () => ({ data: null }),
  useTurnEpisode: () => ({ data: state.episode }),
  useCreateConversation: () => ({
    mutateAsync: mockCreateMutateAsync,
    isPending: false,
  }),
  useSendMessage: () => ({
    mutateAsync: mockSendMutateAsync,
    isPending: state.sendPending !== undefined,
    variables: state.sendPending,
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

function userTranscriptMessage(id: string, text: string) {
  return {
    id,
    role: "user" as const,
    schema_version: 1,
    blocks: [{ type: "text", block_id: `${id}-block`, text }],
    created_at: "2026-07-29T00:00:00.000Z",
  };
}

function page() {
  return (
    <TooltipProvider>
      <AssistantPage />
    </TooltipProvider>
  );
}

function renderPage() {
  return render(page());
}

/** Type into the composer and submit, as the reader does. */
async function sendThrough(
  event: ReturnType<typeof userEvent.setup>,
  text: string,
) {
  await event.type(screen.getByRole("textbox"), text);
  await event.click(screen.getByRole("button", { name: /send/i }));
}

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  state.pathname = "/assistant";
  state.search = {};
  state.conversations = [existingConversation];
  state.conversationsResolved = true;
  state.historyError = false;
  state.sendPending = undefined;
  state.historyMessages = [];
  state.episode = null;
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

  it("shows the sent message and a thinking state while the draft allocates", () => {
    // The conversation does not exist yet, so there is no id, no history query
    // and nothing to render — and the composer has already cleared the text.
    // Without the optimistic echo the reader watches their message vanish into
    // the empty state for the length of the create round-trip.
    state.search = { draft: true };
    state.sendPending = "check my issues";
    renderPage();

    expect(screen.getByText("check my issues")).toBeInTheDocument();
    expect(
      screen.getByRole("status", { name: "Assistant is thinking" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByText("Start a new conversation"),
    ).not.toBeInTheDocument();
  });

  it("does not echo the sent message twice once the transport projects it", async () => {
    const event = userEvent.setup();
    state.search = { c: existingConversation.id };
    const { rerender } = renderPage();

    // Sent through the composer first so the page records the send's target
    // conversation, then the transcript and pending flag are advanced to the
    // state they reach mid-send. Setting `sendPending` up front would make the
    // composer refuse the submit as already-sending.
    await sendThrough(event, "already projected");
    state.historyMessages = [
      userTranscriptMessage("user-projected", "already projected"),
    ];
    state.sendPending = "already projected";
    rerender(page());

    expect(screen.getAllByText("already projected")).toHaveLength(1);
  });

  it("does not echo below the answer when the assistant projects first", async () => {
    // The assistant's first message can land while the send mutation is still
    // pending. A tail-only projection test appended a second copy of the
    // reader's message UNDER the answer, and flipped streaming back to thinking.
    const event = userEvent.setup();
    state.search = { c: existingConversation.id };
    const { rerender } = renderPage();

    await sendThrough(event, "racing send");
    state.historyMessages = [
      userTranscriptMessage("user-projected", "racing send"),
      {
        id: "assistant-first",
        role: "assistant",
        schema_version: 1,
        blocks: [{ type: "text", block_id: "b2", text: "Answering" }],
        created_at: "2026-07-29T00:00:01.000Z",
      },
    ];
    state.sendPending = "racing send";
    rerender(page());

    expect(screen.getAllByText("racing send")).toHaveLength(1);
    expect(screen.getByText("Answering")).toBeInTheDocument();
  });

  it("keeps a pending echo out of a conversation the reader switched to", async () => {
    const event = userEvent.setup();
    state.search = { c: existingConversation.id };
    state.conversations = [existingConversation, boundConversation];
    const { rerender } = renderPage();

    await sendThrough(event, "meant for the first chat");
    state.sendPending = "meant for the first chat";
    rerender(page());
    expect(screen.getByText("meant for the first chat")).toBeInTheDocument();

    // Same mutation still pending, different conversation on screen.
    state.search = { c: boundConversation.id };
    rerender(page());

    expect(
      screen.queryByText("meant for the first chat"),
    ).not.toBeInTheDocument();
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
