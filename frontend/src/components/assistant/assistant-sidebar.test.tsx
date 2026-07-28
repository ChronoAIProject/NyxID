import { render, screen, waitFor } from "@testing-library/react";
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

function renderSidebar() {
  const onSelect = vi.fn();
  const onDelete = vi.fn();
  render(
    <TooltipProvider>
      <AssistantSidebar
        conversations={[CONVERSATION]}
        activeConversationId={CONVERSATION.id}
        onNewChat={vi.fn()}
        onSelect={onSelect}
        onDelete={onDelete}
      />
    </TooltipProvider>,
  );
  return { onSelect, onDelete };
}

describe("AssistantSidebar conversation rows", () => {
  it("deletes only after the popover confirm, never on menu open", async () => {
    const user = userEvent.setup();
    const { onDelete } = renderSidebar();

    await user.click(screen.getByLabelText("Options for Quarterly digest"));
    expect(await screen.findByText("Delete this chat?")).toBeInTheDocument();
    expect(onDelete).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Delete" }));
    expect(onDelete).toHaveBeenCalledTimes(1);
    expect(onDelete).toHaveBeenCalledWith(CONVERSATION.id);
  });

  it("cancel closes the confirm without deleting", async () => {
    const user = userEvent.setup();
    const { onDelete } = renderSidebar();

    await user.click(screen.getByLabelText("Options for Quarterly digest"));
    await user.click(screen.getByRole("button", { name: "Cancel" }));

    await waitFor(() => {
      expect(screen.queryByText("Delete this chat?")).not.toBeInTheDocument();
    });
    expect(onDelete).not.toHaveBeenCalled();
  });

  it("clicking the row body selects the conversation instead of deleting", async () => {
    const user = userEvent.setup();
    const { onSelect, onDelete } = renderSidebar();

    await user.click(screen.getByText("Quarterly digest"));

    expect(onSelect).toHaveBeenCalledWith(CONVERSATION.id);
    expect(onDelete).not.toHaveBeenCalled();
  });
});
