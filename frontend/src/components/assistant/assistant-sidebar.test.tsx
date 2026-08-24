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
import type { User } from "@/types/api";
import { AssistantSidebar } from "./assistant-sidebar";

vi.mock("@/hooks/use-assistant", () => ({
  useWorkspaceCounts: () => ({
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
