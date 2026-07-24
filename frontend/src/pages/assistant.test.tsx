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
    isError: false,
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
});

describe("AssistantPage new chat", () => {
  it("navigates to the draft thread before the actor is provisioned", async () => {
    const user = userEvent.setup();
    // Never resolves: the point is that the UI moves without waiting.
    mockCreateMutateAsync.mockReturnValue(new Promise(() => {}));
    renderPage();

    await user.click(screen.getByRole("button", { name: /New chat/ }));

    expect(mockNavigate).toHaveBeenCalledWith(
      expect.objectContaining({ search: { draft: true } }),
    );
  });

  it("swaps the draft for the conversation id once the create lands", async () => {
    const user = userEvent.setup();
    mockCreateMutateAsync.mockResolvedValue({ ...EXISTING, id: "conv-new" });
    renderPage();

    // The optimistic navigation is what puts the app in draft; mirror it,
    // since the router is stubbed.
    state.search = { draft: true };
    await user.click(screen.getByRole("button", { name: /New chat/ }));

    await waitFor(() => {
      expect(mockNavigate).toHaveBeenCalledWith(
        expect.objectContaining({ search: { c: "conv-new" }, replace: true }),
      );
    });
  });

  it("does not yank the user back when they navigated away mid-create", async () => {
    const user = userEvent.setup();
    let resolveCreate: (conversation: Conversation) => void = () => {};
    mockCreateMutateAsync.mockReturnValue(
      new Promise<Conversation>((resolve) => {
        resolveCreate = resolve;
      }),
    );
    renderPage();

    state.search = { draft: true };
    await user.click(screen.getByRole("button", { name: /New chat/ }));
    mockNavigate.mockClear();

    // They opened another chat while the actor was still provisioning.
    state.search = { c: EXISTING.id };
    resolveCreate({ ...EXISTING, id: "conv-new" });

    await waitFor(() => {
      expect(mockCreateMutateAsync).toHaveBeenCalled();
    });
    expect(mockNavigate).not.toHaveBeenCalled();
  });

  it("says a failed create out loud instead of stopping silently", async () => {
    const user = userEvent.setup();
    mockCreateMutateAsync.mockRejectedValue(new Error("aevatar is down"));
    renderPage();

    state.search = { draft: true };
    await user.click(screen.getByRole("button", { name: /New chat/ }));

    await waitFor(() => {
      expect(mockToastError).toHaveBeenCalledWith(
        "Could not start a new chat",
        expect.objectContaining({ description: "aevatar is down" }),
      );
    });
  });

  it("renders the empty draft thread rather than falling back to the newest chat", () => {
    state.search = { draft: true };
    renderPage();

    expect(screen.getByRole("heading", { name: "New chat" })).toBeInTheDocument();
  });

  it("still falls back to the newest chat when there is no draft", () => {
    renderPage();

    expect(
      screen.getByRole("heading", { name: EXISTING.title }),
    ).toBeInTheDocument();
  });
});
