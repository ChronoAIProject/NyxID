import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { AnchorHTMLAttributes, ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { TooltipProvider } from "@/components/ui/tooltip";
import type { Conversation } from "@/types/assistant";
import { AssistantPage } from "./assistant";

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
    conversations: [] as Conversation[],
    historyError: false,
  },
}));

// Getters, not a snapshot: the page reads the router AFTER awaiting the
// create, precisely to detect that the user navigated away meanwhile.
const routerStub = {
  state: {
    location: {
      get pathname() {
        return state.pathname;
      },
      get search() {
        return state.search;
      },
    },
  },
};

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
  useRouter: () => routerStub,
  useRouterState: ({
    select,
  }: {
    select: (s: { location: { search: Record<string, unknown> } }) => unknown;
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

// The shell is chrome (theme, auth menu, mobile drawer); this page's
// behaviour is the sidebar + thread it wraps.
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
  useConversations: () => ({ data: state.conversations }),
  useConversation: (conversationId: string | undefined) => ({
    data: conversationId
      ? {
          conversation: state.conversations.find(
            (item) => item.id === conversationId,
          ),
          messages: [],
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
  useDeleteConversation: () => ({
    mutateAsync: vi.fn(),
    isPending: false,
    variables: undefined,
  }),
  useWorkspaceCounts: () => ({ data: { artifacts: 0, pendingApprovals: 0 } }),
  describeSendFailure: () => ({ message: "Message not sent", description: "" }),
  describeHistoryError: () => "This conversation has no saved transcript yet.",
}));

vi.mock("sonner", () => ({
  toast: { error: mockToastError },
}));

const EXISTING: Conversation = {
  id: "conv-1",
  title: "Quarterly digest",
  created_at: "2026-07-20T00:00:00.000Z",
  last_message_at: "2026-07-20T00:05:00.000Z",
};

function renderPage() {
  render(
    <TooltipProvider>
      <AssistantPage />
    </TooltipProvider>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  state.pathname = "/assistant";
  state.search = {};
  state.conversations = [EXISTING];
  state.historyError = false;
});

describe("AssistantPage new chat", () => {
  it("navigates to the draft thread", async () => {
    const user = userEvent.setup();
    renderPage();

    await user.click(screen.getByRole("button", { name: /New chat/ }));

    expect(mockNavigate).toHaveBeenCalledWith(
      expect.objectContaining({ search: { draft: true } }),
    );
  });

  it("provisions nothing on click — the click is navigation only", async () => {
    // An empty conversation has no server-side representation: the
    // chat-history transcript 404s until a turn materializes a row. Creating
    // the actor here and swapping to `?c=<id>` is what made every new chat
    // fail its first history read. Allocation belongs to the first send.
    const user = userEvent.setup();
    renderPage();

    await user.click(screen.getByRole("button", { name: /New chat/ }));

    expect(mockCreateMutateAsync).not.toHaveBeenCalled();
    expect(mockNavigate).toHaveBeenCalledTimes(1);
  });

  it("allocates on first send and follows the returned conversation", async () => {
    const user = userEvent.setup();
    mockSendMutateAsync.mockResolvedValue({ conversationId: "conv-new" });
    state.search = { draft: true };
    renderPage();

    await user.type(screen.getByRole("textbox"), "hello");
    await user.click(screen.getByRole("button", { name: /send/i }));

    await waitFor(() => {
      expect(mockSendMutateAsync).toHaveBeenCalledWith("hello");
    });
    expect(mockNavigate).toHaveBeenCalledWith(
      expect.objectContaining({ search: { c: "conv-new" } }),
    );
  });

  it("renders the empty draft thread rather than falling back to the newest chat", () => {
    state.search = { draft: true };
    renderPage();

    expect(screen.getByRole("heading", { name: "New chat" })).toBeInTheDocument();
  });

  it("reports a failed transcript read without taking the chat away", () => {
    // Replacing the whole thread with an error hid the composer too, so the
    // chat looked dead even though sending still works — the turn streams
    // into the conversation actor, a different surface from the transcript.
    state.historyError = true;
    state.search = { c: EXISTING.id };
    renderPage();

    expect(screen.getByRole("status")).toHaveTextContent(
      /no saved transcript yet/i,
    );
    // Still usable: the composer is present and the thread still rendered.
    expect(screen.getByRole("textbox")).toBeInTheDocument();
  });

  it("still falls back to the newest chat when there is no draft", () => {
    renderPage();

    expect(
      screen.getByRole("heading", { name: EXISTING.title }),
    ).toBeInTheDocument();
  });
});
