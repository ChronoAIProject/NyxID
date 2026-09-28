import {
  cleanup,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AnchorHTMLAttributes, ReactNode } from "react";
import { TooltipProvider } from "@/components/ui/tooltip";
import { useAssistantDraftStore } from "@/stores/assistant-draft-store";
import { useAuthStore } from "@/stores/auth-store";
import type { Conversation } from "@/types/assistant";
import type { AssistantAgent } from "@/schemas/assistant-nyxagent";
import type { User } from "@/types/api";
import { AssistantSidebar, type SidebarAgents } from "./assistant-sidebar";

vi.mock("@/hooks/use-assistant-workspace", () => ({
  useAssistantWorkspaceCounts: () => ({
    data: { artifacts: 0, pendingApprovals: 0 },
  }),
}));

vi.mock("@tanstack/react-router", () => ({
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

const CONVERSATION: Conversation = {
  id: "conv-1",
  title: "Quarterly digest",
  created_at: "2026-07-20T00:00:00.000Z",
  last_message_at: "2026-07-20T00:05:00.000Z",
};

const SECOND_CONVERSATION: Conversation = {
  id: "conv-2",
  title: "Rotate GitHub token",
  created_at: "2026-07-19T00:00:00.000Z",
  last_message_at: "2026-07-19T00:05:00.000Z",
};

const USER: User = {
  id: "user-1",
  email: "reader@example.com",
  display_name: "Reader",
  avatar_url: null,
  email_verified: true,
  mfa_enabled: false,
  is_admin: false,
  is_active: true,
  created_at: "2026-07-20T00:00:00.000Z",
};

function renderSidebar(
  onDelete: (id: string) => void | Promise<void> = vi.fn(),
  conversations: readonly Conversation[] = [CONVERSATION],
  activeConversationId: string | undefined = CONVERSATION.id,
  notice?: string,
) {
  const onSelect = vi.fn();
  const view = render(
    <TooltipProvider>
      <AssistantSidebar
        conversations={conversations}
        activeConversationId={activeConversationId}
        onNewChat={vi.fn()}
        onSelect={onSelect}
        onDelete={onDelete}
        notice={notice}
      />
    </TooltipProvider>,
  );
  return { ...view, onSelect, onDelete };
}

function seedDraft(ownerUserId: string, text: string) {
  useAssistantDraftStore.setState({
    ownerUserId,
    drafts: {
      [`conv:${CONVERSATION.id}`]: { text, updatedAt: 1 },
    },
  });
}

beforeEach(() => {
  localStorage.clear();
  useAssistantDraftStore.setState({ ownerUserId: null, drafts: {} });
  useAuthStore.setState({
    user: USER,
    isAuthenticated: true,
    isLoading: false,
    mfaRequired: false,
    mfaToken: null,
  });
});

afterEach(() => {
  cleanup();
  useAssistantDraftStore.getState().clear();
  useAuthStore.setState({
    user: null,
    isAuthenticated: false,
    isLoading: true,
    mfaRequired: false,
    mfaToken: null,
  });
  localStorage.clear();
});

describe("AssistantSidebar conversation rows", () => {
  it("surfaces a conversation-list failure without hiding cached rows", () => {
    renderSidebar(
      vi.fn(),
      [CONVERSATION],
      CONVERSATION.id,
      "Could not load chats. Network unavailable",
    );

    expect(
      screen.getByText("Could not load chats. Network unavailable"),
    ).toHaveAttribute("role", "status");
    expect(screen.getByText("Quarterly digest")).toBeVisible();
  });

  it("opens a menu -- not a delete prompt -- and only deletes after confirm", async () => {
    const user = userEvent.setup();
    const onDelete = vi.fn();
    renderSidebar(onDelete);

    await user.click(screen.getByLabelText("Options for Quarterly digest"));
    expect(
      await screen.findByRole("menuitem", { name: /delete/i }),
    ).toBeInTheDocument();
    expect(screen.queryByText("Delete chat?")).not.toBeInTheDocument();
    expect(onDelete).not.toHaveBeenCalled();

    await user.click(screen.getByRole("menuitem", { name: /delete/i }));
    expect(await screen.findByText("Delete chat?")).toBeInTheDocument();
    expect(onDelete).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Delete" }));
    expect(onDelete).toHaveBeenCalledTimes(1);
    expect(onDelete).toHaveBeenCalledWith(CONVERSATION.id);
  });

  it("cancel closes the confirm without deleting", async () => {
    const user = userEvent.setup();
    const onDelete = vi.fn();
    renderSidebar(onDelete);

    await user.click(screen.getByLabelText("Options for Quarterly digest"));
    await user.click(screen.getByRole("menuitem", { name: /delete/i }));
    await user.click(screen.getByRole("button", { name: "Cancel" }));

    await waitFor(() => {
      expect(screen.queryByText("Delete chat?")).not.toBeInTheDocument();
    });
    expect(onDelete).not.toHaveBeenCalled();
  });

  it("keeps the confirm open when the delete fails, so it stays retryable", async () => {
    const user = userEvent.setup();
    const onDelete = vi.fn().mockRejectedValue(new Error("offline"));
    renderSidebar(onDelete);

    await user.click(screen.getByLabelText("Options for Quarterly digest"));
    await user.click(screen.getByRole("menuitem", { name: /delete/i }));
    await user.click(screen.getByRole("button", { name: "Delete" }));

    expect(onDelete).toHaveBeenCalledTimes(1);
    expect(screen.getByText("Delete chat?")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Delete" }));
    expect(onDelete).toHaveBeenCalledTimes(2);
  });

  it("does not fire a second delete while the first is in flight", async () => {
    const user = userEvent.setup();
    let settle: () => void = () => undefined;
    const onDelete = vi.fn(
      () =>
        new Promise<void>((resolve) => {
          settle = resolve;
        }),
    );
    renderSidebar(onDelete);

    await user.click(screen.getByLabelText("Options for Quarterly digest"));
    await user.click(screen.getByRole("menuitem", { name: /delete/i }));
    await user.click(screen.getByRole("button", { name: "Delete" }));
    await user.click(screen.getByRole("button", { name: "Delete" }));

    expect(onDelete).toHaveBeenCalledTimes(1);

    settle();
    await waitFor(() => {
      expect(screen.queryByText("Delete chat?")).not.toBeInTheDocument();
    });
  });

  // Dismissal stays available mid-flight, so a request can outlive its own
  // dialog; landing late it must not close the confirmation for a different
  // chat that has been opened since.
  it("a late-landing delete does not close another chat's confirmation", async () => {
    const user = userEvent.setup();
    let settleFirst: () => void = () => undefined;
    const onDelete = vi.fn(
      () =>
        new Promise<void>((resolve) => {
          settleFirst = resolve;
        }),
    );
    renderSidebar(onDelete, [CONVERSATION, SECOND_CONVERSATION]);

    await user.click(screen.getByLabelText("Options for Quarterly digest"));
    await user.click(screen.getByRole("menuitem", { name: /delete/i }));
    await user.click(screen.getByRole("button", { name: "Delete" }));
    await user.click(screen.getByRole("button", { name: "Cancel" }));

    await user.click(screen.getByLabelText("Options for Rotate GitHub token"));
    await user.click(screen.getByRole("menuitem", { name: /delete/i }));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText(/Rotate GitHub token/)).toBeInTheDocument();

    settleFirst();
    await waitFor(() => {
      expect(onDelete).toHaveBeenCalledTimes(1);
    });
    expect(within(dialog).getByText("Delete chat?")).toBeInTheDocument();
    expect(within(dialog).getByText(/Rotate GitHub token/)).toBeInTheDocument();
  });

  // A single pending marker would forget chat A the moment B was submitted,
  // letting A's still-hung request be fired a second time.
  it("tracks every in-flight target, not just the most recent one", async () => {
    const user = userEvent.setup();
    const settles = new Map<string, () => void>();
    const onDelete = vi.fn(
      (id: string) =>
        new Promise<void>((resolve) => {
          settles.set(id, resolve);
        }),
    );
    renderSidebar(onDelete, [CONVERSATION, SECOND_CONVERSATION]);

    async function submitDelete(title: string) {
      await user.click(screen.getByLabelText(`Options for ${title}`));
      await user.click(screen.getByRole("menuitem", { name: /delete/i }));
      await user.click(screen.getByRole("button", { name: "Delete" }));
    }

    await submitDelete("Quarterly digest");
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    await submitDelete("Rotate GitHub token");
    await user.click(screen.getByRole("button", { name: "Cancel" }));

    settles.get(SECOND_CONVERSATION.id)?.();
    await waitFor(() => {
      expect(onDelete).toHaveBeenCalledTimes(2);
    });

    // The first request is still hung; re-confirming it must not fire again.
    await submitDelete("Quarterly digest");
    expect(onDelete).toHaveBeenCalledTimes(2);

    settles.get(CONVERSATION.id)?.();
    await waitFor(() => {
      expect(screen.queryByText("Delete chat?")).not.toBeInTheDocument();
    });
  });

  it("clicking the row body selects the conversation instead of deleting", async () => {
    const user = userEvent.setup();
    const { onSelect, onDelete } = renderSidebar();

    await user.click(screen.getByText("Quarterly digest"));

    expect(onSelect).toHaveBeenCalledWith(CONVERSATION.id);
    expect(onDelete).not.toHaveBeenCalled();
  });

  it("shows an inactive conversation draft when the owner matches", () => {
    seedDraft(USER.id, "Finish the quarterly summary");

    const { container } = renderSidebar(
      undefined,
      [CONVERSATION],
      SECOND_CONVERSATION.id,
    );

    expect(
      screen.getByText("Finish the quarterly summary"),
    ).toBeInTheDocument();
    expect(container.querySelector(".lucide-pencil-line")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Quarterly digest" }),
    ).toHaveAccessibleDescription("Draft: Finish the quarterly summary");
  });

  it("does not show a draft owned by another user", () => {
    seedDraft("previous-user", "Private draft from another account");

    const { container } = renderSidebar(
      undefined,
      [CONVERSATION],
      SECOND_CONVERSATION.id,
    );

    expect(
      screen.queryByText("Private draft from another account"),
    ).not.toBeInTheDocument();
    expect(
      container.querySelector(".lucide-pencil-line"),
    ).not.toBeInTheDocument();
  });

  it("does not show the draft preview on the active conversation", () => {
    seedDraft(USER.id, "Visible in the active composer");

    const { container } = renderSidebar();

    expect(
      screen.queryByText("Visible in the active composer"),
    ).not.toBeInTheDocument();
    expect(
      container.querySelector(".lucide-pencil-line"),
    ).not.toBeInTheDocument();
  });

  it("collapses draft whitespace into a single preview line", () => {
    seedDraft(USER.id, "  First line\n\n second\tline   and final words  ");

    renderSidebar(undefined, [CONVERSATION], SECOND_CONVERSATION.id);

    expect(
      screen.getByText("First line second line and final words"),
    ).toBeInTheDocument();
  });

  it("shows no draft line and still selects when no draft exists", async () => {
    const user = userEvent.setup();
    const { container, onSelect, onDelete } = renderSidebar(
      undefined,
      [CONVERSATION],
      SECOND_CONVERSATION.id,
    );

    expect(
      container.querySelector(".lucide-pencil-line"),
    ).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Quarterly digest" }));

    expect(onSelect).toHaveBeenCalledWith(CONVERSATION.id);
    expect(onDelete).not.toHaveBeenCalled();
  });
});

describe("NyxAgent row controls", () => {
  const row: Conversation = { ...CONVERSATION, id: `nyxa-${"a".repeat(32)}` };
  function nyxSidebar(conversation: Conversation, onRename = vi.fn()) {
    render(
      <TooltipProvider>
        <AssistantSidebar
          conversations={[conversation]}
          activeConversationId={row.id}
          onNewChat={vi.fn()}
          onSelect={vi.fn()}
          onDelete={vi.fn()}
          onRename={onRename}
        />
      </TooltipProvider>,
    );
    return onRename;
  }
  it("renames through a dirty-gated validated form", async () => {
    const user = userEvent.setup();
    const rename = nyxSidebar(row);
    await user.click(screen.getByRole("button", { name: `Options for ${row.title}` }));
    await user.click(screen.getByRole("menuitem", { name: "Rename" }));
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    const input = within(screen.getByRole("dialog")).getByRole("textbox");
    await user.clear(input);
    await user.type(input, "New title");
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(rename).toHaveBeenCalledExactlyOnceWith(row.id, "New title"));
  });
  it("disables both mutations while a turn is active", async () => {
    const user = userEvent.setup();
    nyxSidebar({ ...row, active_turn: { turn_id: "running", started_at: row.created_at } });
    await user.click(screen.getByRole("button", { name: `Options for ${row.title}` }));
    expect(screen.getByRole("menuitem", { name: "Rename" })).toHaveAttribute(
      "aria-disabled", "true",
    );
    expect(screen.getByRole("menuitem", { name: "Delete" })).toHaveAttribute(
      "aria-disabled", "true",
    );
  });
});

describe("NyxBot agents in the sidebar", () => {
  const at = "2026-09-28T00:00:00.000Z";
  function agent(fields: Partial<AssistantAgent>): AssistantAgent {
    return {
      id: "agent-nyxbot",
      kind: "nyxbot",
      name: "NyxBot",
      description: "",
      specialty: null,
      created_by: "user",
      status: "idle",
      services: [],
      account_read: true,
      pending_requests: [],
      last_reply: null,
      home_conversation_id: null,
      memory_count: 0,
      created_at: at,
      last_active_at: at,
      destroyed_at: null,
      pending_acknowledgements: 0,
      channels: [],
      ...fields,
    };
  }
  const agents = [
    agent({ channels: [{ id: "c1", platform: "telegram", bot_label: "Home", status: "active" }] }),
    agent({
      id: "agent-researcher",
      kind: "specialist",
      name: "researcher",
      status: "running",
      pending_acknowledgements: 2,
    }),
    agent({ id: "agent-mailer", kind: "specialist", name: "mailer", status: "destroyed", destroyed_at: at }),
  ];
  const thread: Conversation = {
    id: `nyxa-${"c".repeat(32)}`,
    title: "Morning briefing",
    created_at: at,
    last_message_at: at,
    channel: { platform: "telegram" },
  };

  function renderAgents(model: Partial<SidebarAgents> = {}) {
    const handlers = {
      onSelectAgent: vi.fn(),
      onNewThread: vi.fn(),
      onNewAgent: vi.fn(),
    };
    const onSelect = vi.fn();
    render(
      <TooltipProvider>
        <AssistantSidebar
          conversations={[]}
          activeConversationId={thread.id}
          onNewChat={vi.fn()}
          onSelect={onSelect}
          onDelete={vi.fn()}
          agents={{
            agents,
            selectedAgentId: "agent-nyxbot",
            threads: [thread],
            ...handlers,
            ...model,
          }}
        />
      </TooltipProvider>,
    );
    return { ...handlers, onSelect };
  }

  it("pins NyxBot first and nests the selected agent's threads under it", async () => {
    const user = userEvent.setup();
    const { onSelect, onNewThread } = renderAgents();
    const nav = screen.getByRole("navigation");
    // Agent rows are the expandable buttons without a menu; NyxBot is pinned first.
    const agentRows = within(nav)
      .getAllByRole("button")
      .filter(
        (button) =>
          button.hasAttribute("aria-expanded") && !button.hasAttribute("aria-haspopup"),
      );
    expect(agentRows.map((button) => button.getAttribute("aria-label"))).toEqual([
      "NyxBot — your personal agent",
      "researcher, specialist, Running, 2 pending requests",
    ]);
    const nyxbot = within(nav).getByRole("button", { name: "NyxBot — your personal agent" });
    expect(nyxbot).toHaveAttribute("aria-expanded", "true");
    expect(nyxbot).toHaveTextContent("Telegram");
    const threads = within(nav).getByRole("group", { name: "Threads with NyxBot" });
    const row = within(threads).getByRole("button", { name: "Morning briefing" });
    expect(row).toHaveTextContent("Telegram");
    await user.click(row);
    expect(onSelect).toHaveBeenCalledWith(thread.id);
    await user.click(within(threads).getByRole("button", { name: "New chat with NyxBot" }));
    expect(onNewThread).toHaveBeenCalledWith("agent-nyxbot");
    // Only the chats of earlier engines are listed under "Chats"; none here.
    expect(screen.queryByText("Chats")).not.toBeInTheDocument();
  });

  it("shows specialist status and pending requests, and selects an agent", async () => {
    const user = userEvent.setup();
    const { onSelectAgent, onNewAgent } = renderAgents();
    const researcher = screen.getByRole("button", {
      name: "researcher, specialist, Running, 2 pending requests",
    });
    expect(researcher).toHaveAttribute("aria-expanded", "false");
    expect(researcher).toHaveTextContent("2");
    await user.click(researcher);
    expect(onSelectAgent).toHaveBeenCalledWith("agent-researcher");
    await user.click(screen.getByRole("button", { name: "New agent" }));
    expect(onNewAgent).toHaveBeenCalledOnce();
  });

  it("keeps destroyed specialists behind a toggle, dimmed", async () => {
    const user = userEvent.setup();
    renderAgents();
    expect(screen.queryByRole("button", { name: /^mailer/ })).not.toBeInTheDocument();
    const toggle = screen.getByRole("button", { name: "Show destroyed (1)" });
    await user.click(toggle);
    const mailer = screen.getByRole("button", { name: "mailer, specialist, Destroyed" });
    expect(mailer.className).toContain("opacity-50");
    expect(screen.getByRole("button", { name: "Hide destroyed" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
  });

  it("offers no new thread for a destroyed agent", () => {
    renderAgents({ selectedAgentId: "agent-mailer", threads: [] });
    // Destroyed agents are hidden until shown, so nothing is expanded.
    expect(screen.queryByRole("button", { name: /New chat with/ })).not.toBeInTheDocument();
  });
});
