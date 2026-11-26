import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { markAdmitted } from "@/lib/assistant/send-admission";
import { parseNyxBotHandoff } from "@/schemas/nyxbot-handoff";
import type { NyxBotHandoff } from "@/schemas/nyxbot-handoff";
import { transitionAssistantIdentity } from "@/lib/assistant/identity";
import { TextBlock } from "./text-block";
import { NyxBotHandoffContext } from "@/lib/assistant/nyxbot-handoff-context";

const handoff = {
  version: 1,
  title: "Create a Telegram bot",
  task: "Help me create a Telegram bot named Support and link it to my support specialist.",
} as const;
const artifact = `\`\`\`nyxbot-handoff\n${JSON.stringify(handoff)}\n\`\`\``;

const source = {
  ownerUserId: "owner",
  sourceConversationId: "nyxa-source",
  eligible: true,
};
beforeEach(() => {
  localStorage.clear();
  transitionAssistantIdentity(crypto.randomUUID());
});

describe("specialist handoff artifact", () => {
  it("reopens the receipt when its transcript row is unmounted and remounted", async () => {
    const next = vi.fn(
      async (_proposal: NyxBotHandoff, onAdopt: (id: string) => void) =>
        onAdopt(`nyxa-${"a".repeat(32)}`),
    );
    const open = vi.fn();
    const tree = (
      <NyxBotHandoffContext value={{ ...source, continue: next, open }}>
        <TextBlock
          text={artifact}
          handoffMessageId="message-one"
          allowHandoff
        />
      </NyxBotHandoffContext>
    );
    const view = render(tree);
    fireEvent.click(
      screen.getByRole("button", { name: "Continue with NyxBot" }),
    );
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Open NyxBot chat" }),
      ).toBeEnabled(),
    );
    view.unmount();
    render(tree);
    fireEvent.click(screen.getByRole("button", { name: "Open NyxBot chat" }));
    expect(next).toHaveBeenCalledTimes(1);
    expect(open).toHaveBeenCalledWith(`nyxa-${"a".repeat(32)}`);
  });

  it("activates only the first top-level artifact", () => {
    render(
      <NyxBotHandoffContext
        value={{ ...source, continue: vi.fn(), open: vi.fn() }}
      >
        <TextBlock
          text={`${artifact}\n\n${artifact}`}
          handoffMessageId="message-one"
          allowHandoff
        />
      </NyxBotHandoffContext>,
    );
    expect(
      screen.getAllByRole("button", { name: "Continue with NyxBot" }),
    ).toHaveLength(1);
  });
  it("shows the proposed task and starts only when clicked, with duplicate clicks blocked", async () => {
    let finish!: () => void;
    const next = vi.fn(
      () =>
        new Promise<void>((resolve) => {
          finish = resolve;
        }),
    );
    render(
      <NyxBotHandoffContext
        value={{ ...source, continue: next, open: vi.fn() }}
      >
        <TextBlock
          text={artifact}
          handoffMessageId="message-one"
          allowHandoff
        />
      </NyxBotHandoffContext>,
    );
    expect(screen.getByText(handoff.task)).toBeVisible();
    expect(next).not.toHaveBeenCalled();
    const button = screen.getByRole("button", { name: "Continue with NyxBot" });
    fireEvent.click(button);
    fireEvent.click(button);
    expect(next).toHaveBeenCalledTimes(1);
    expect(next).toHaveBeenCalledWith(
      handoff,
      expect.any(Function),
      expect.any(String),
    );
    expect(button).toBeDisabled();
    finish();
    await waitFor(() => expect(button).toBeEnabled());
  });

  it("opens an admitted thread after a reply failure without sending the task again", async () => {
    const open = vi.fn();
    const next = vi.fn(
      async (_proposal: NyxBotHandoff, onAdopt: (id: string) => void) => {
        onAdopt("nyxa-new");
        throw markAdmitted(new Error("Stream interrupted"));
      },
    );
    render(
      <NyxBotHandoffContext value={{ ...source, continue: next, open }}>
        <TextBlock
          text={artifact}
          handoffMessageId="message-one"
          allowHandoff
        />
      </NyxBotHandoffContext>,
    );
    fireEvent.click(
      screen.getByRole("button", { name: "Continue with NyxBot" }),
    );
    const button = await screen.findByRole("button", {
      name: "Open NyxBot chat",
    });
    await waitFor(() => expect(button).toBeEnabled());
    fireEvent.click(button);
    expect(open).toHaveBeenCalledWith("nyxa-new");
    expect(next).toHaveBeenCalledTimes(1);
  });

  it("allows retry after a rejected send and explains its failure", async () => {
    const next = vi
      .fn()
      .mockRejectedValueOnce(new Error("NyxBot is at capacity right now."))
      .mockResolvedValue(undefined);
    render(
      <NyxBotHandoffContext
        value={{ ...source, continue: next, open: vi.fn() }}
      >
        <TextBlock
          text={artifact}
          handoffMessageId="message-one"
          allowHandoff
        />
      </NyxBotHandoffContext>,
    );
    const button = screen.getByRole("button", { name: "Continue with NyxBot" });
    fireEvent.click(button);
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "NyxBot is at capacity",
    );
    fireEvent.click(button);
    await waitFor(() => expect(next).toHaveBeenCalledTimes(2));
  });

  it("does not replay a task accepted before its conversation ID was lost", async () => {
    const next = vi
      .fn()
      .mockRejectedValue(markAdmitted(new Error("Connection lost")));
    render(
      <NyxBotHandoffContext
        value={{ ...source, continue: next, open: vi.fn() }}
      >
        <TextBlock
          text={artifact}
          handoffMessageId="message-one"
          allowHandoff
        />
      </NyxBotHandoffContext>,
    );
    const button = screen.getByRole("button", { name: "Continue with NyxBot" });
    fireEvent.click(button);
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "This task may already have started",
    );
    expect(button).toBeDisabled();
    fireEvent.click(button);
    expect(next).toHaveBeenCalledTimes(1);
  });

  it("keeps artifacts inert in user content, while streaming, and without a chat controller", () => {
    const next = vi.fn();
    const view = render(
      <NyxBotHandoffContext
        value={{ ...source, continue: next, open: vi.fn() }}
      >
        <TextBlock text={artifact} />
      </NyxBotHandoffContext>,
    );
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
    view.rerender(
      <TextBlock
        text={artifact}
        handoffMessageId="message-one"
        allowHandoff
        streaming
      />,
    );
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
    view.rerender(
      <TextBlock text={artifact} handoffMessageId="message-one" allowHandoff />,
    );
    expect(
      screen.getByRole("button", { name: "Continue with NyxBot" }),
    ).toBeDisabled();
    expect(next).not.toHaveBeenCalled();
  });

  it("rejects malformed, oversized, secret-bearing and extra-authority payloads", () => {
    for (const payload of [
      "not json",
      '{"version":1}',
      JSON.stringify({ ...handoff, version: 2 }),
      JSON.stringify({ ...handoff, task: " " }),
      JSON.stringify({ ...handoff, task: "a".repeat(4001) }),
      JSON.stringify({ ...handoff, task: "Use Bearer abcdefghijklmnop" }),
      JSON.stringify({ ...handoff, task: "Use nyxid_abcdefghijk" }),
      JSON.stringify({ ...handoff, task: `Token 123456789:${"A".repeat(35)}` }),
      JSON.stringify({ ...handoff, task: `Use sk-${"a".repeat(32)}` }),
      JSON.stringify({ ...handoff, task: `Use ghp_${"a".repeat(32)}` }),
      JSON.stringify({ ...handoff, task: `Use github_pat_${"a".repeat(32)}` }),
      JSON.stringify({ ...handoff, task: `Use xoxb-${"a".repeat(32)}` }),
      JSON.stringify({ ...handoff, task: "-----BEGIN PRIVATE KEY-----" }),
      JSON.stringify({
        ...handoff,
        task: "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJvd25lciJ9.abcdefghijk",
      }),
      JSON.stringify({ ...handoff, approve: true }),
      JSON.stringify({ ...handoff, agent_id: "specialist" }),
    ])
      expect(parseNyxBotHandoff(payload)).toBeNull();
    render(
      <TextBlock
        text={'```nyxbot-handoff\n{"version":1,"title":"Setup"}\n```'}
        handoffMessageId="message-one"
        allowHandoff
      />,
    );
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });
});
