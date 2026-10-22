import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { GroupPendingActions } from "@/components/assistant/nyxbot-group-view";
import { assistantGroupMessagesSchema } from "@/schemas/assistant-nyxagent";
import { api } from "@/lib/api-client";
vi.mock("@/lib/api-client", async (importOriginal) => ({
  ...await importOriginal<typeof import("@/lib/api-client")>(),
  api: { put: vi.fn(), delete: vi.fn() },
}));

vi.mock("@tanstack/react-router", async (importOriginal) => ({
  ...await importOriginal<typeof import("@tanstack/react-router")>(),
  Link: ({ to, search, children, ...props }: { to: string; search: Record<string, string>; children: React.ReactNode }) =>
    <a {...props} href={`${to}?${new URLSearchParams(search).toString()}`}>{children}</a>,
}));

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
      <GroupPendingActions
        actions={[]}
        members={[]}
        sending={false}
        onAnswer={vi.fn()}
      />,
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
    const card = screen.getByRole("region", {
      name: "Confirm: Delete agent key 'ci-bot'",
    });
    expect(card).toHaveTextContent(
      "Keeper wants to: Delete agent key 'ci-bot'",
    );
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
      assistantGroupMessagesSchema.parse({
        group,
        messages: [],
        before_seq: null,
      }).pending_actions,
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

it("shows the triggering person and hides decisions from other participants", () => {
  const metadata = { ...action, confirm_phrase: undefined };
  render(
    <GroupPendingActions
      actions={[
        {
          ...metadata,
          triggering_person: { id: "b", display_name: "Blair" },
          can_decide: false,
        },
      ]}
      members={[]}
      sending={false}
      onAnswer={vi.fn()}
      onDecide={vi.fn()}
    />,
  );
  expect(screen.getByText("Awaiting Blair")).toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Allow" }),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Deny" }),
  ).not.toBeInTheDocument();
});

it("decides an org card by id instead of posting a confirmation phrase", async () => {
  const onDecide = vi.fn().mockResolvedValue(undefined);
  const onAnswer = vi.fn();
  const card = {
    ...action,
    confirm_phrase: undefined,
    triggering_person: { id: "b", display_name: "Blair" },
    can_decide: true,
  };
  render(
    <GroupPendingActions
      actions={[card]}
      members={[]}
      sending={false}
      onAnswer={onAnswer}
      onDecide={onDecide}
    />,
  );
  await userEvent.click(screen.getByRole("button", { name: "Allow" }));
  expect(onDecide).toHaveBeenLastCalledWith(card, "allow");
  await userEvent.click(screen.getByRole("button", { name: "Deny" }));
  expect(onDecide).toHaveBeenLastCalledWith(card, "deny");
  expect(onAnswer).not.toHaveBeenCalled();
});

import { GroupMessageRow } from "@/components/assistant/nyxbot-group-view";

it("group message composition toggles pressed state through the feedback handler", async () => {
  vi.mocked(api.put).mockResolvedValue({ vote: "up" });
  render(
    <GroupMessageRow
      groupId="g"
      message={
        {
          id: "m",
          group_id: "g",
          seq: 1,
          role: "agent",
          text: "reply",
          created_at: new Date().toISOString(),
          activities: [],
          attachments: [],
          feedback: { vote: "up" },
        } as never
      }
      names={[]}
      onOpenAgent={() => undefined}
    />,
  );
  expect(screen.getByRole("button", { name: "Helpful" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  expect(screen.getByRole("button", { name: "Not helpful" })).toBeEnabled();
  await userEvent.click(screen.getByRole("button", { name: "Not helpful" }));
  expect(api.put).toHaveBeenCalledWith(
    "/assistant/nyxagent/groups/g/messages/m/feedback",
    { vote: "down", comment: undefined },
  );
  expect(screen.getByRole("button", { name: "Not helpful" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
});

it("group replies take an optional comment after a vote without blocking it", async () => {
  vi.mocked(api.put).mockReset();
  vi.mocked(api.put).mockResolvedValue({ vote: "down" });
  render(
    <GroupMessageRow
      groupId="g"
      message={
        {
          id: "m2",
          group_id: "g",
          seq: 2,
          role: "agent",
          text: "reply",
          created_at: new Date().toISOString(),
          activities: [],
          attachments: [],
        } as never
      }
      names={[]}
      onOpenAgent={() => undefined}
    />,
  );
  await userEvent.click(screen.getByRole("button", { name: "Add feedback comment" }));
  // Sending a comment needs a vote first; the vote itself never waits for one.
  expect(screen.getByRole("button", { name: "Send" })).toBeDisabled();
  await userEvent.click(screen.getByRole("button", { name: "Not helpful" }));
  expect(api.put).toHaveBeenLastCalledWith(
    "/assistant/nyxagent/groups/g/messages/m2/feedback",
    { vote: "down", comment: undefined },
  );
  await userEvent.type(screen.getByLabelText("Optional feedback comment"), "wrong calendar");
  await userEvent.click(screen.getByRole("button", { name: "Send" }));
  expect(api.put).toHaveBeenLastCalledWith(
    "/assistant/nyxagent/groups/g/messages/m2/feedback",
    { vote: "down", comment: "wrong calendar" },
  );
  expect(screen.queryByLabelText("Optional feedback comment")).not.toBeInTheDocument();
});

it("group feedback failures are visible", async () => {
  vi.mocked(api.put).mockReset();
  vi.mocked(api.put).mockRejectedValue(new Error("offline"));
  render(
    <GroupMessageRow
      groupId="g"
      message={
        {
          id: "m3",
          group_id: "g",
          seq: 3,
          role: "agent",
          text: "reply",
          created_at: new Date().toISOString(),
          activities: [],
          attachments: [],
        } as never
      }
      names={[]}
      onOpenAgent={() => undefined}
    />,
  );
  await userEvent.click(screen.getByRole("button", { name: "Helpful" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Unable to save feedback");
  expect(screen.getByRole("button", { name: "Helpful" })).toHaveAttribute("aria-pressed", "false");
});

it("links authored drafts to file review without any confirmation buttons", () => {
  const answer = vi.fn();
  render(<GroupPendingActions actions={[{
    conversation_id: "nyxa-hidden", acknowledgement_id: "draft-card", summary: "Publish skill",
    expires_at: "2099-01-01T00:00:00Z", authored_skill: true,
    review_url: "https://nyx.test/assistant?c=nyxa-hidden&card=draft-card",
    confirm_phrase: "yes 1234", can_decide: true,
  }]} members={[]} sending={false} onAnswer={answer} />);
  expect(screen.getByRole("link", { name: "Review draft" })).toHaveAttribute("href", "/assistant?c=nyxa-hidden&card=draft-card");
  expect(screen.queryByRole("button")).not.toBeInTheDocument();
  expect(answer).not.toHaveBeenCalled();
});
