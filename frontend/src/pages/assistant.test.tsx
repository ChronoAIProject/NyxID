import type { AnchorHTMLAttributes, ReactNode } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { TooltipProvider } from "@/components/ui/tooltip";
import { assistantKeys } from "@/hooks/use-assistant";
import { ApiError } from "@/lib/api-client";
import { AssistantConversationNotFoundError } from "@/lib/assistant/errors";
import { useAssistantContextStore } from "@/stores/assistant-context-store";
import { useAssistantDraftStore } from "@/stores/assistant-draft-store";
import { useAuthStore } from "@/stores/auth-store";
import type { Conversation } from "@/types/assistant";
import type { User } from "@/types/api";

const {
  mockNavigate,
  mockCreateMutateAsync,
  mockCancelMutateAsync,
  mockDecideMutateAsync,
  mockResolveInputMutateAsync,
  mockContinueAction,
  mockDeleteMutateAsync,
  mockSendMutateAsync,
  mockToastError,
  state,
} = vi.hoisted(() => ({
  mockNavigate: vi.fn(),
  mockCreateMutateAsync: vi.fn(),
  mockCancelMutateAsync: vi.fn(),
  mockDecideMutateAsync: vi.fn(),
  mockResolveInputMutateAsync: vi.fn(),
  mockContinueAction: vi.fn(),
  mockDeleteMutateAsync: vi.fn(),
  mockSendMutateAsync: vi.fn(),
  mockToastError: vi.fn(),
  state: {
    pathname: "/assistant",
    search: {} as Record<string, unknown>,
    conversations: [] as Conversation[] | undefined,
    conversationsResolved: true,
    historyError: undefined as unknown,
    historyLoading: false,
    historyCanonicalId: undefined as string | undefined,
    historyAwaitingProjection: false,
    historyProjectionStalled: false,
    turnStatus: null as
      | "queued"
      | "running"
      | "waiting_approval"
      | "completed"
      | "failed"
      | "cancelled"
      | null,
    // Mirrors what TanStack exposes for an in-flight mutation.
    sendPending: undefined as string | undefined,
    cancelPending: false,
    decisionPending: false,
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
      location: { pathname: string; search: Record<string, unknown> };
    }) => unknown;
  }) =>
    select({
      location: { pathname: state.pathname, search: state.search },
    }),
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
  assistantKeys: {
    history: (conversationId: string) => [
      "assistant",
      "history",
      conversationId,
    ],
    turn: (conversationId: string) => ["assistant", "turn", conversationId],
    episode: (conversationId: string) => [
      "assistant",
      "episode",
      conversationId,
    ],
  },
  useConversations: () => ({
    data: state.conversations,
    isSuccess: state.conversationsResolved,
  }),
  useConversation: (conversationId: string | undefined) => ({
    data:
      conversationId && !state.historyLoading
        ? {
            conversation: {
              ...(state.conversations?.find(
                (conversation) => conversation.id === conversationId,
              ) ?? {
                id: conversationId,
                title: "Loading",
                created_at: "2026-07-29T00:00:00.000Z",
                last_message_at: "2026-07-29T00:00:00.000Z",
              }),
              id: state.historyCanonicalId ?? conversationId,
            },
            messages: state.historyMessages,
            has_more: false,
            awaitingProjection: state.historyAwaitingProjection,
            projectionStalled: state.historyProjectionStalled,
          }
        : undefined,
    isLoading: state.historyLoading,
    isFetching: state.historyLoading,
    isError: state.historyError !== undefined,
    error: state.historyError,
  }),
  useAssistantTurn: () => ({
    data:
      state.turnStatus === null
        ? null
        : { turnId: "turn-1", status: state.turnStatus, error: null },
  }),
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
  useCancelTurn: () => ({
    mutateAsync: mockCancelMutateAsync,
    isPending: state.cancelPending,
  }),
  useDecideApproval: () => ({
    mutateAsync: mockDecideMutateAsync,
    isPending: state.decisionPending,
  }),
  useResolveInput: () => ({
    mutateAsync: mockResolveInputMutateAsync,
    isPending: false,
  }),
  useActionCardActions: () => ({
    setInProgress: vi.fn(),
    blockAction: vi.fn(),
    continueAction: mockContinueAction,
  }),
  useDeleteConversation: () => ({
    mutateAsync: mockDeleteMutateAsync,
    isPending: false,
    variables: undefined,
  }),
  useWorkspaceCounts: () => ({ data: { artifacts: 0, pendingApprovals: 0 } }),
  describeSendFailure: () => ({
    message: "Message not sent",
    description: "",
  }),
}));

vi.mock("sonner", () => ({
  toast: { error: mockToastError },
}));

import { AssistantPage } from "./assistant";

const queryClient = new QueryClient({
  defaultOptions: {
    queries: { retry: false },
    mutations: { retry: false },
  },
});

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
    <QueryClientProvider client={queryClient}>
      <TooltipProvider>
        <AssistantPage />
      </TooltipProvider>
    </QueryClientProvider>
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
  queryClient.clear();
  localStorage.clear();
  state.pathname = "/assistant";
  state.search = {};
  state.conversations = [existingConversation];
  state.conversationsResolved = true;
  state.historyError = undefined;
  state.historyLoading = false;
  state.historyCanonicalId = undefined;
  state.historyAwaitingProjection = false;
  state.historyProjectionStalled = false;
  state.turnStatus = null;
  state.sendPending = undefined;
  state.cancelPending = false;
  state.decisionPending = false;
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
  });
  useAssistantDraftStore.setState({ ownerUserId: user.id, drafts: {} });
});

describe("AssistantPage projection status", () => {
  // Projection provenance is background reconciliation: the transcript
  // demonstrably materializes into the open thread on its own, so narrating
  // it as page chrome claimed more than the client knew. The provenance
  // still drives the reconciler; it must never render status furniture.
  it("renders no status strip for a transcript awaiting projection", () => {
    state.search = { c: existingConversation.id };
    state.historyAwaitingProjection = true;

    renderPage();

    expect(
      screen.queryByText("Syncing conversation history..."),
    ).not.toBeInTheDocument();
    expect(screen.queryByText(/You can keep chatting/)).not.toBeInTheDocument();
    expect(mockNavigate).not.toHaveBeenCalled();
  });

  it("renders no stalled notice or retry for a stalled projection", () => {
    state.search = { c: existingConversation.id };
    state.historyProjectionStalled = true;

    renderPage();

    expect(
      screen.queryByText("History is taking longer than expected."),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Retry" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByText("Syncing conversation history..."),
    ).not.toBeInTheDocument();
  });

  it("keeps a cold awaiting-projection conversation fully usable", () => {
    // Cold reload during the projection window: no episode this session, no
    // transcript yet. The never-block contract — nothing announced, nothing
    // disabled; the reconciler fills the thread in when the transcript lands.
    state.search = { c: existingConversation.id };
    state.historyAwaitingProjection = true;
    state.historyMessages = [];
    state.episode = null;

    renderPage();

    expect(
      screen.queryByText("Syncing conversation history..."),
    ).not.toBeInTheDocument();
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(screen.getByRole("textbox")).toBeEnabled();
  });

  it("keeps the composer usable while a cold transcript is loading [guard]", () => {
    // "Loading conversation..." replaces only the thread area, never the
    // composer or sidebar — a read that never resolves still leaves the page
    // fully operable.
    state.search = { c: existingConversation.id };
    state.historyLoading = true;
    state.historyMessages = [];

    renderPage();

    expect(screen.getByText("Loading conversation...")).toBeInTheDocument();
    expect(screen.getByRole("textbox")).toBeEnabled();
  });

  it("renders available content unannounced while projection is pending", () => {
    // Whatever content we legitimately have renders; the pending projection
    // is not narrated on top of it.
    state.search = { c: existingConversation.id };
    state.historyAwaitingProjection = true;
    state.historyMessages = [
      userTranscriptMessage("user-1", "hi"),
      {
        id: "assistant-1",
        role: "assistant",
        schema_version: 1,
        blocks: [{ type: "text", block_id: "b1", text: "Partial answer" }],
        created_at: "2026-07-29T00:00:01.000Z",
      },
    ];

    renderPage();

    expect(screen.getByText("Partial answer")).toBeInTheDocument();
    expect(
      screen.queryByText("Syncing conversation history..."),
    ).not.toBeInTheDocument();
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
    expect(screen.getByRole("textbox")).toBeEnabled();
  });
});

describe("AssistantPage action continuation", () => {
  it("toasts a failed action continuation and keeps the chat usable", async () => {
    const event = userEvent.setup();
    state.search = { c: existingConversation.id };
    state.historyMessages = [
      {
        id: "assistant-action",
        role: "assistant",
        schema_version: 1,
        blocks: [
          {
            type: "action_card",
            block_id: "action-card-1",
            action: "service.connect",
            action_request_id: "act-1",
            origin_turn_id: "turn-origin-1",
            task_id: "task-1",
            step_id: "step-1",
            params: {
              variant: "catalog",
              service_slug: "api-github",
              requested_scopes: ["repo"],
            },
            status: "pending",
            outcome_note: "",
          },
        ],
        created_at: "2026-07-29T00:00:01.000Z",
      },
    ];
    mockContinueAction.mockRejectedValueOnce(new Error("delivery failed"));

    renderPage();
    await event.click(screen.getByRole("button", { name: "Decline" }));

    await waitFor(() => {
      expect(mockToastError).toHaveBeenCalledWith(
        "The action response was not delivered",
        expect.objectContaining({
          id: "assistant-action-failed",
          description: "delivery failed",
        }),
      );
    });
    // Non-blocking: the card is still on screen and retryable, the composer
    // still takes input.
    expect(screen.getByRole("button", { name: "Decline" })).toBeEnabled();
    expect(screen.getByRole("textbox")).toBeEnabled();
  });
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

  it("renders the explicit draft state rather than the newest chat", () => {
    state.search = { draft: true };
    useAssistantContextStore.setState({
      ownerUserId: user.id,
      lastScreen: "/keys",
    });
    renderPage();

    expect(
      screen.getByRole("heading", { name: "New chat" }),
    ).toBeInTheDocument();
  });

  it("keeps the chat fully usable with no chrome when a transcript read fails", () => {
    // The failure itself is toast-only (`useHistoryErrorToast` inside
    // `useConversation`, covered in the hook suite). The page's contract is
    // silence: no strip, nothing disabled, existing content untouched.
    state.historyError = new Error("Network unavailable");
    state.search = { c: existingConversation.id };
    state.historyMessages = [userTranscriptMessage("user-kept", "still here")];
    renderPage();

    expect(
      screen.queryByText(/no saved transcript yet/i),
    ).not.toBeInTheDocument();
    expect(screen.queryByText(/You can keep chatting/)).not.toBeInTheDocument();
    expect(screen.getByText("still here")).toBeInTheDocument();
    expect(screen.getByRole("textbox")).toBeEnabled();
  });

  it("keeps bare /assistant as a new-chat draft when chats exist", () => {
    renderPage();

    expect(
      screen.getByRole("heading", { name: "New chat" }),
    ).toBeInTheDocument();
    expect(mockNavigate).not.toHaveBeenCalled();
  });

  it("does not infer a conversation when a cold list resolves", () => {
    state.conversations = undefined;
    state.conversationsResolved = false;
    const { rerender } = renderPage();

    state.conversations = [existingConversation];
    state.conversationsResolved = true;
    rerender(page());

    expect(
      screen.getByRole("heading", { name: "New chat" }),
    ).toBeInTheDocument();
    expect(mockNavigate).not.toHaveBeenCalled();
  });

  it("treats ?draft=true as bare even when an old c parameter is present", () => {
    state.search = { c: existingConversation.id, draft: true };
    renderPage();

    expect(
      screen.getByRole("heading", { name: "New chat" }),
    ).toBeInTheDocument();
    expect(mockNavigate).not.toHaveBeenCalled();
  });
});

describe("AssistantPage conversation resolution", () => {
  it("keeps lastScreen only for a draft that later follows an explicit selection", async () => {
    useAssistantContextStore.setState({
      ownerUserId: user.id,
      lastScreen: "/keys",
    });
    const { rerender } = renderPage();
    fireEvent.change(screen.getByRole("textbox"), {
      target: { value: "Question typed in the screen draft" },
    });

    state.search = { c: boundConversation.id };
    state.conversations = [boundConversation];
    rerender(page());

    await waitFor(() => {
      expect(screen.getByRole("textbox")).toHaveValue(
        "Question typed in the screen draft",
      );
    });
    expect(useAssistantDraftStore.getState().getDraft("screen:/keys")).toBe("");
    expect(
      useAssistantDraftStore
        .getState()
        .getDraft(`conv:${boundConversation.id}`),
    ).toBe("Question typed in the screen draft");
  });

  it("repairs a missing explicit id only after history confirms a 404", async () => {
    state.search = { c: "deleted-conversation" };
    state.conversations = undefined;
    state.conversationsResolved = false;
    useAssistantContextStore.setState({
      ownerUserId: user.id,
      lastScreen: "/nodes",
    });
    const { rerender } = renderPage();

    expect(mockNavigate).not.toHaveBeenCalled();

    state.conversations = [boundConversation];
    state.conversationsResolved = true;
    state.historyError = new ApiError(404, {
      error: "not_found",
      error_code: 404,
      message: "Conversation not found",
    });
    rerender(page());

    await waitFor(() => {
      expect(mockNavigate).toHaveBeenCalledWith({
        to: "/assistant",
        search: {},
        replace: true,
      });
    });
    state.search = {};
    state.historyError = undefined;
    rerender(page());
    expect(
      screen.getByRole("heading", { name: "New chat" }),
    ).toBeInTheDocument();
  });

  it("repairs a missing explicit id for the typed transport not-found", async () => {
    state.search = { c: "nyxid-pending-lost-after-reload" };
    state.conversations = [boundConversation];
    state.historyError = new AssistantConversationNotFoundError();
    renderPage();

    await waitFor(() => {
      expect(mockNavigate).toHaveBeenCalledWith({
        to: "/assistant",
        search: {},
        replace: true,
      });
    });
  });

  it.each([
    ["network", new Error("Network unavailable")],
    ["untyped not-found", new Error("Conversation was not found.")],
    [
      "forbidden",
      new ApiError(403, {
        error: "forbidden",
        error_code: 403,
        message: "Forbidden",
      }),
    ],
    [
      "server",
      new ApiError(503, {
        error: "unavailable",
        error_code: 503,
        message: "Unavailable",
      }),
    ],
  ])("retains an explicit id after a %s history failure", (_label, error) => {
    state.search = { c: "unlisted-conversation" };
    state.conversations = [boundConversation];
    state.historyError = error;
    renderPage();

    expect(mockNavigate).not.toHaveBeenCalled();
    // Failure reporting is toast-only; the page renders no error chrome and
    // stays usable on the same route.
    expect(
      screen.queryByText(/You can keep chatting/i),
    ).not.toBeInTheDocument();
    expect(screen.getByRole("textbox")).toBeEnabled();
  });

  it("retains an unlisted explicit id while history is still resolving", () => {
    state.search = { c: "unlisted-conversation" };
    state.conversations = [boundConversation];
    renderPage();

    expect(
      screen.getByRole("heading", { name: "Loading" }),
    ).toBeInTheDocument();
    expect(mockNavigate).not.toHaveBeenCalled();
  });
});

describe("AssistantPage canonical conversation resolution", () => {
  const placeholderId = "nyxid-pending-local";
  const canonicalConversation: Conversation = {
    id: "nyxid-chat-canonical",
    title: "Canonical chat",
    created_at: "2026-07-29T00:00:00.000Z",
    last_message_at: "2026-07-29T00:00:01.000Z",
  };

  function useAliasedConversation() {
    state.search = { c: placeholderId };
    state.conversations = [canonicalConversation];
    state.historyCanonicalId = canonicalConversation.id;
  }

  it("keeps the placeholder URL live and marks the canonical sidebar row active", () => {
    useAliasedConversation();
    state.sendPending = "hello";
    renderPage();

    expect(mockNavigate).not.toHaveBeenCalled();
    expect(
      screen.getByRole("button", { name: canonicalConversation.title }),
    ).toHaveClass("font-medium");
  });

  it.each([
    ["active turn", { turnStatus: "running" as const }],
    [
      "open episode",
      { episode: { open: true, printed: false, projecting: false } },
    ],
    ["pending cancellation", { cancelPending: true }],
    ["approval continuation", { decisionPending: true }],
  ])("does not swap during an %s", (_label, liveState) => {
    useAliasedConversation();
    Object.assign(state, liveState);
    renderPage();

    expect(mockNavigate).not.toHaveBeenCalled();
  });

  it("seeds the canonical transcript and turn slots before navigating", async () => {
    useAliasedConversation();
    state.historyMessages = [
      userTranscriptMessage("user-finished", "Question"),
      {
        id: "assistant-finished",
        role: "assistant",
        schema_version: 1,
        blocks: [
          { type: "text", block_id: "assistant-block", text: "Finished" },
        ],
        created_at: "2026-07-29T00:00:01.000Z",
      },
    ];
    state.turnStatus = "completed";
    state.episode = { open: true, printed: true, projecting: false };
    const { rerender } = renderPage();
    expect(mockNavigate).not.toHaveBeenCalled();
    expect(
      queryClient.getQueryData(assistantKeys.history(canonicalConversation.id)),
    ).toBeUndefined();

    state.episode = { open: false, printed: true, projecting: false };
    let cachesWereSeededBeforeNavigate = false;
    mockNavigate.mockImplementationOnce(() => {
      cachesWereSeededBeforeNavigate = Boolean(
        queryClient.getQueryData(
          assistantKeys.history(canonicalConversation.id),
        ) &&
        queryClient.getQueryData(
          assistantKeys.episode(canonicalConversation.id),
        ) &&
        queryClient.getQueryData(assistantKeys.turn(canonicalConversation.id)),
      );
    });
    rerender(page());

    await waitFor(() => {
      expect(mockNavigate).toHaveBeenCalledWith({
        to: "/assistant",
        search: { c: canonicalConversation.id },
        replace: true,
      });
    });
    expect(cachesWereSeededBeforeNavigate).toBe(true);
    expect(
      queryClient.getQueryData(assistantKeys.history(canonicalConversation.id)),
    ).toMatchObject({
      conversation: { id: canonicalConversation.id },
      messages: [
        expect.objectContaining({ id: "user-finished" }),
        expect.objectContaining({ id: "assistant-finished" }),
      ],
    });
    expect(
      queryClient.getQueryData(assistantKeys.episode(canonicalConversation.id)),
    ).toEqual({ open: false, printed: true, projecting: false });
    expect(
      queryClient.getQueryData(assistantKeys.turn(canonicalConversation.id)),
    ).toMatchObject({ status: "completed" });
  });

  it("drops the placeholder URL when its canonical sidebar row is deleted", async () => {
    const event = userEvent.setup();
    useAliasedConversation();
    state.episode = { open: true, printed: true, projecting: false };
    mockDeleteMutateAsync.mockResolvedValue(undefined);
    renderPage();

    await event.click(
      screen.getByLabelText(`Options for ${canonicalConversation.title}`),
    );
    await event.click(await screen.findByRole("menuitem", { name: /delete/i }));
    await event.click(screen.getByRole("button", { name: "Delete" }));

    await waitFor(() => {
      expect(mockDeleteMutateAsync).toHaveBeenCalledWith(
        canonicalConversation.id,
      );
      expect(mockNavigate).toHaveBeenCalledWith({
        to: "/assistant",
        search: {},
      });
    });
  });

  it("does not normalize after router state has moved off /assistant", () => {
    useAliasedConversation();
    state.pathname = "/assistant/plugins";
    renderPage();

    expect(mockNavigate).not.toHaveBeenCalled();
  });
});
