import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { AnchorHTMLAttributes, ReactNode } from "react";
import { TooltipProvider } from "@/components/ui/tooltip";
import type { Conversation } from "@/types/assistant";
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

function renderSidebar(
  onDelete: (id: string) => void | Promise<void> = vi.fn(),
  conversations: readonly Conversation[] = [CONVERSATION],
) {
  const onSelect = vi.fn();
  render(
    <TooltipProvider>
      <AssistantSidebar
        conversations={conversations}
        activeConversationId={CONVERSATION.id}
        creating={false}
        onNewChat={vi.fn()}
        onSelect={onSelect}
        onDelete={onDelete}
      />
    </TooltipProvider>,
  );
  return { onSelect, onDelete };
}

describe("AssistantSidebar conversation rows", () => {
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
});
