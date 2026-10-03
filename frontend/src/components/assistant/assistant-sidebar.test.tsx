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
import type { AssistantAgent, AssistantGroup } from "@/schemas/assistant-nyxagent";
import type { User } from "@/types/api";
import {
  AssistantSidebar,
  type SidebarAgents,
  type SidebarGroups,
} from "./assistant-sidebar";

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
    const input = within(screen.getByRole("form", { name: "Rename chat" })).getByRole("textbox");
    await user.clear(input);
    await user.type(input, "New title");
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(rename).toHaveBeenCalledExactlyOnceWith(row.id, "New title"));
  });
  it("allows rename but disables delete while a turn is active", async () => {
    const user = userEvent.setup();
    nyxSidebar({ ...row, active_turn: { turn_id: "running", started_at: row.created_at } });
    await user.click(screen.getByRole("button", { name: `Options for ${row.title}` }));
    expect(screen.getByRole("menuitem", { name: "Rename" })).not.toHaveAttribute("aria-disabled", "true");
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
      display_name: null,
      persona: null,
      specialty: null,
      created_by: "user",
      status: "idle",
      services: [],
      account_read: true,
      guest_access: {},
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

  function renderAgents(
    model: Partial<SidebarAgents> = {},
    options: { conversations?: readonly Conversation[]; groups?: SidebarGroups } = {},
  ) {
    const handlers = {
      onSelectAgent: vi.fn(),
      onNewThread: vi.fn(),
      onNewAgent: vi.fn(),
    };
    const onSelect = vi.fn();
    render(
      <TooltipProvider>
        <AssistantSidebar
          conversations={options.conversations ?? []}
          groups={options.groups}
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
    // Agent rows are the expandable buttons without a menu (channel bots'
    // chat sections excepted); NyxBot is pinned first.
    const agentRows = within(nav)
      .getAllByRole("button")
      .filter(
        (button) =>
          button.hasAttribute("aria-expanded") &&
          !button.hasAttribute("aria-haspopup") &&
          !/ on Telegram, /.test(button.getAttribute("aria-label") ?? ""),
      );
    expect(agentRows.map((button) => button.getAttribute("aria-label"))).toEqual([
      "NyxBot — your personal agent",
      "researcher, specialist, Running, 2 pending requests",
    ]);
    const nyxbot = within(nav).getByRole("button", { name: "NyxBot — your personal agent" });
    expect(nyxbot).toHaveAttribute("aria-expanded", "true");
    expect(nyxbot).toHaveTextContent("Telegram");
    const threads = within(nav).getByRole("group", { name: "Threads with NyxBot" });
    // The channel thread sits in its bot's section, open since it is the
    // open thread.
    expect(
      within(threads).getByRole("button", { name: "Telegram bot on Telegram, 1 chat" }),
    ).toHaveAttribute("aria-expanded", "true");
    const row = within(threads).getByRole("button", { name: "Morning briefing" });
    await user.click(row);
    expect(onSelect).toHaveBeenCalledWith(thread.id);
    await user.click(within(threads).getByRole("button", { name: "New chat with NyxBot" }));
    expect(onNewThread).toHaveBeenCalledWith("agent-nyxbot");
    // Only the chats of earlier engines are listed under "Chats"; none here.
    expect(screen.queryByText("Chats")).not.toBeInTheDocument();
  });

  it("groups a bot's chats in a collapsed section with more on request", async () => {
    const user = userEvent.setup();
    const own: Conversation = { ...thread, id: `nyxa-${"a".repeat(32)}`, title: "Plans", channel: null };
    const chats: Conversation[] = Array.from({ length: 7 }, (_, index) => ({
      ...thread,
      id: `nyxa-${String(index).repeat(32)}`,
      title: `Chat ${String(index)}`,
      channel: {
        platform: "telegram",
        channel_agent_id: "c1",
        bot_label: "Support bot",
        chat_kind: index === 0 ? "group" : "private",
      },
    }));
    const renderWith = (active: string) =>
      render(
        <TooltipProvider>
          <AssistantSidebar
            conversations={[]}
            activeConversationId={active}
            onNewChat={vi.fn()}
            onSelect={vi.fn()}
            onDelete={vi.fn()}
            agents={{
              agents,
              selectedAgentId: "agent-nyxbot",
              threads: [own, ...chats],
              onSelectAgent: vi.fn(),
              onNewThread: vi.fn(),
              onNewAgent: vi.fn(),
            }}
          />
        </TooltipProvider>,
      );
    const { unmount } = renderWith(own.id);
    const threads = screen.getByRole("group", { name: "Threads with NyxBot" });
    expect(within(threads).getByRole("button", { name: "Plans" })).toBeInTheDocument();
    const section = within(threads).getByRole("button", {
      name: "Support bot on Telegram, 7 chats",
    });
    expect(section).toHaveAttribute("aria-expanded", "false");
    expect(within(threads).queryByRole("button", { name: "Chat 0" })).not.toBeInTheDocument();
    await user.click(section);
    expect(section).toHaveAttribute("aria-expanded", "true");
    expect(within(threads).getByRole("button", { name: "Chat 4" })).toBeInTheDocument();
    expect(within(threads).queryByRole("button", { name: "Chat 5" })).not.toBeInTheDocument();
    await user.click(within(threads).getByRole("button", { name: "Show 2 more" }));
    expect(within(threads).getByRole("button", { name: "Chat 6" })).toBeInTheDocument();
    unmount();
    // The open chat's section starts open and shows it even past the first
    // page; closed, it opens again for another of its chats.
    const view = renderWith(chats[6]!.id);
    const reopened = screen.getByRole("group", { name: "Threads with NyxBot" });
    expect(
      within(reopened).getByRole("button", { name: "Support bot on Telegram, 7 chats" }),
    ).toHaveAttribute("aria-expanded", "true");
    expect(within(reopened).getByRole("button", { name: "Chat 6" })).toBeInTheDocument();
    expect(within(reopened).queryByRole("button", { name: "Chat 5" })).not.toBeInTheDocument();
    const header = within(reopened).getByRole("button", {
      name: "Support bot on Telegram, 7 chats",
    });
    await user.click(header);
    expect(header).toHaveAttribute("aria-expanded", "false");
    view.rerender(
      <TooltipProvider>
        <AssistantSidebar
          conversations={[]}
          activeConversationId={chats[1]!.id}
          onNewChat={vi.fn()}
          onSelect={vi.fn()}
          onDelete={vi.fn()}
          agents={{
            agents,
            selectedAgentId: "agent-nyxbot",
            threads: [own, ...chats],
            onSelectAgent: vi.fn(),
            onNewThread: vi.fn(),
            onNewAgent: vi.fn(),
          }}
        />
      </TooltipProvider>,
    );
    expect(header).toHaveAttribute("aria-expanded", "true");
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

  it("names agents by their display name with the @handle beneath", () => {
    renderAgents({
      agents: [
        agent({ display_name: "Nyx" }),
        agent({ id: "agent-writer", kind: "specialist", name: "writer", display_name: "Luna" }),
      ],
    });
    const nyxbot = screen.getByRole("button", { name: "Nyx (@NyxBot) — your personal agent" });
    expect(nyxbot).toHaveTextContent("@NyxBot · personal agent");
    const writer = screen.getByRole("button", { name: "Luna (@writer), specialist, Idle" });
    expect(writer).toHaveTextContent("Luna");
    expect(writer).toHaveTextContent("@writer");
  });

  it("drops the generic New chat and the earlier engines' Chats list", () => {
    renderAgents({}, { conversations: [CONVERSATION] });
    expect(screen.queryByRole("button", { name: "New chat" })).not.toBeInTheDocument();
    expect(screen.queryByText("Chats")).not.toBeInTheDocument();
    expect(screen.queryByText("Quarterly digest")).not.toBeInTheDocument();
  });

  it("offers Home, marked current on the NyxBot home", async () => {
    const user = userEvent.setup();
    const onHome = vi.fn();
    renderAgents({ homeActive: true, onHome });
    const home = screen.getByRole("button", { name: "Home" });
    expect(home).toHaveAttribute("aria-current", "page");
    await user.click(home);
    expect(onHome).toHaveBeenCalledOnce();
  });

  it("lists groups with their members and who is working", async () => {
    const user = userEvent.setup();
    const group = (fields: Partial<AssistantGroup>): AssistantGroup => ({
      id: "nyxg-1",
      name: "Launch crew",
      members: [
        { id: "agent-nyxbot", name: "NyxBot", kind: "nyxbot", destroyed: false, working: false },
        { id: "agent-researcher", name: "researcher", kind: "specialist", destroyed: false, working: true },
      ],
      lead_agent_id: "agent-nyxbot",
      working_agent_ids: ["agent-researcher"],
      message_count: 3,
      last_message_at: at,
      created_at: at,
      ...fields,
    });
    const groups: SidebarGroups = {
      groups: [group({}), group({ id: "nyxg-2", name: "Quiet", working_agent_ids: [] })],
      selectedGroupId: "nyxg-2",
      onSelectGroup: vi.fn(),
      onNewGroup: vi.fn(),
    };
    renderAgents({}, { groups });
    const crew = screen.getByRole("button", {
      name: "Launch crew, group with NyxBot, researcher, 1 working",
    });
    expect(screen.getByRole("button", { name: "Quiet, group with NyxBot, researcher" })).toHaveAttribute(
      "aria-current",
      "page",
    );
    await user.click(crew);
    expect(groups.onSelectGroup).toHaveBeenCalledWith("nyxg-1");
    await user.click(screen.getByRole("button", { name: "New group" }));
    expect(groups.onNewGroup).toHaveBeenCalledOnce();
  });

  it("invites a first group when there are none", async () => {
    const user = userEvent.setup();
    const onNewGroup = vi.fn();
    renderAgents({}, { groups: { groups: [], onSelectGroup: vi.fn(), onNewGroup } });
    await user.click(screen.getByRole("button", { name: "Chat with several agents at once" }));
    expect(onNewGroup).toHaveBeenCalledOnce();
  });

  it("offers no new thread for a destroyed agent", () => {
    renderAgents({ selectedAgentId: "agent-mailer", threads: [] });
    // Destroyed agents are hidden until shown, so nothing is expanded.
    expect(screen.queryByRole("button", { name: /New chat with/ })).not.toBeInTheDocument();
  });
});
