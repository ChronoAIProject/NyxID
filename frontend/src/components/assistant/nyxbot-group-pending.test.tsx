import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { GroupPendingActions } from "@/components/assistant/nyxbot-group-view";
import { assistantGroupMessagesSchema } from "@/schemas/assistant-nyxagent";

const action = {
  conversation_id: "nyxa-00000000000000000000000000000001",
  acknowledgement_id: "7b1d2c3e-0000-4000-8000-000000000001",
  agent_id: "agent-1",
  summary: "Delete agent key 'ci-bot'",
  confirm_phrase: "yes 4821",
  expires_at: "2026-09-29T12:00:00Z",
};

describe("GroupPendingActions", () => {
  it("renders nothing without pending actions", () => {
    const { container } = render(
      <GroupPendingActions actions={[]} members={[]} sending={false} onAnswer={vi.fn()} />,
    );
    expect(container).toBeEmptyDOMElement();
  });

  it("answers a member's action by posting its phrase", async () => {
    const onAnswer = vi.fn().mockResolvedValue(undefined);
    render(
      <GroupPendingActions
        actions={[action]}
        members={[{ id: "agent-1", name: "keeper", display_name: "Keeper" }]}
        sending={false}
        onAnswer={onAnswer}
      />,
    );
    const card = screen.getByRole("region", { name: "Confirm: Delete agent key 'ci-bot'" });
    expect(card).toHaveTextContent("Keeper wants to: Delete agent key 'ci-bot'");
    await userEvent.click(screen.getByRole("button", { name: "Confirm" }));
    expect(onAnswer).toHaveBeenLastCalledWith("yes 4821");
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onAnswer).toHaveBeenLastCalledWith("no 4821");
  });

  it("parses pending actions and defaults them to none", () => {
    const group = {
      id: "nyxg-00000000000000000000000000000001",
      name: "Ops",
      members: [],
      lead_agent_id: "agent-1",
      working_agent_ids: [],
      message_count: 0,
      last_message_at: null,
      created_at: "2026-09-29T00:00:00Z",
    };
    expect(
      assistantGroupMessagesSchema.parse({ group, messages: [], before_seq: null }).pending_actions,
    ).toEqual([]);
    expect(() =>
      assistantGroupMessagesSchema.parse({
        group,
        messages: [],
        before_seq: null,
        pending_actions: [{ ...action, confirm_phrase: "yes please" }],
      }),
    ).toThrow();
  });
});
