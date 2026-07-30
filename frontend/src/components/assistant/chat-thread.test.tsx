import { act, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AssistantMessage } from "@/types/assistant";
import { ChatThread } from "./chat-thread";

vi.mock("@/components/assistant/blocks/action-card", () => ({
  ActionCard: ({
    block,
  }: {
    readonly block: { readonly action_request_id: string };
  }) => <div data-testid="action-card-dispatch">{block.action_request_id}</div>,
}));

function message(overrides: Partial<AssistantMessage>): AssistantMessage {
  return {
    id: "message-1",
    role: "assistant",
    schema_version: 1,
    blocks: [],
    created_at: "2026-07-16T04:00:00.000Z",
    ...overrides,
  };
}

describe("ChatThread", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("renders an unsupported shell for an unknown block type", () => {
    const unknown = message({
      blocks: [
        {
          type: "future_card",
          block_id: "future-1",
          payload: "opaque",
        },
      ] as unknown as AssistantMessage["blocks"],
    });
    render(<ChatThread messages={[unknown]} onDecideApproval={vi.fn()} />);

    expect(
      screen.getByText("Unsupported assistant content"),
    ).toBeInTheDocument();
  });

  it("renders an unsupported shell for a newer message schema", () => {
    render(
      <ChatThread
        messages={[
          message({
            schema_version: 2,
            blocks: [{ type: "text", block_id: "text-1", text: "Hidden" }],
          }),
        ]}
        onDecideApproval={vi.fn()}
      />,
    );

    expect(
      screen.getByText("Unsupported assistant content"),
    ).toBeInTheDocument();
    expect(screen.queryByText("Hidden")).not.toBeInTheDocument();
  });

  it("shows the thinking indicator while a turn awaits its first content", () => {
    render(
      <ChatThread
        messages={[
          message({
            role: "user",
            blocks: [{ type: "text", block_id: "text-1", text: "Hi" }],
          }),
        ]}
        thinking
        onDecideApproval={vi.fn()}
      />,
    );

    expect(
      screen.getByRole("status", { name: "Assistant is thinking" }),
    ).toBeInTheDocument();
    expect(document.querySelector("[data-assistant-halo]")).toHaveAttribute(
      "aria-hidden",
      "true",
    );
  });

  it("draws the halo from the sprite strip rather than a bare CSS ring", () => {
    render(
      <ChatThread
        messages={[
          message({
            role: "user",
            blocks: [{ type: "text", block_id: "text-1", text: "Hi" }],
          }),
        ]}
        thinking
        onDecideApproval={vi.fn()}
      />,
    );

    // The strip URL has to come from the JS import: app.css is shared with the
    // CLI wizard bundle, whose single-file build would inline any asset a
    // stylesheet references as base64 into a binary that has no chat UI.
    const sprite = document.querySelector<HTMLElement>(
      "[data-assistant-halo] .assistant-halo-sprite",
    );
    expect(sprite).toBeInTheDocument();
    expect(sprite?.style.backgroundImage).toMatch(/^url\(.+\)$/);
  });

  it("reserves tail room and fades over the composer it scrolls behind", () => {
    const { container } = render(
      <ChatThread
        messages={[
          message({
            blocks: [{ type: "text", block_id: "text-1", text: "Answer" }],
          }),
        ]}
        bottomInset={140}
        onDecideApproval={vi.fn()}
      />,
    );

    const scroller = container.querySelector<HTMLElement>(".overflow-y-auto");
    expect(scroller?.style.maskImage).toContain("calc(100% - 140px)");
    // Tail padding keeps the last turn clear of the composer at scroll bottom.
    expect(
      scroller?.firstElementChild?.getAttribute("style"),
    ).toContain("140px");
  });

  it("hides the thinking indicator once assistant content streams", () => {
    render(
      <ChatThread
        messages={[
          message({
            blocks: [{ type: "text", block_id: "text-1", text: "Answer" }],
          }),
        ]}
        onDecideApproval={vi.fn()}
      />,
    );

    expect(
      screen.queryByRole("status", { name: "Assistant is thinking" }),
    ).not.toBeInTheDocument();
    expect(
      document.querySelector("[data-assistant-halo]"),
    ).not.toBeInTheDocument();
  });

  it("shows the halo only on the actively streaming assistant identity", () => {
    render(
      <ChatThread
        messages={[
          message({
            id: "assistant-history",
            blocks: [{ type: "text", block_id: "text-1", text: "Earlier" }],
          }),
          message({
            id: "user-latest",
            role: "user",
            blocks: [{ type: "text", block_id: "text-2", text: "Next" }],
          }),
          message({
            id: "assistant-active",
            blocks: [{ type: "text", block_id: "text-3", text: "Streaming" }],
          }),
        ]}
        streaming
        onDecideApproval={vi.fn()}
      />,
    );

    expect(document.querySelectorAll("[data-assistant-halo]")).toHaveLength(1);
  });

  it("keeps the halo mounted until its exit fade completes", () => {
    vi.useFakeTimers();
    const activeMessage = message({
      blocks: [{ type: "text", block_id: "text-1", text: "Answer" }],
    });
    const { rerender } = render(
      <ChatThread
        messages={[activeMessage]}
        streaming
        onDecideApproval={vi.fn()}
      />,
    );

    act(() => vi.advanceTimersByTime(0));
    expect(document.querySelector("[data-assistant-halo]")).toHaveClass(
      "assistant-halo--visible",
    );

    rerender(
      <ChatThread messages={[activeMessage]} onDecideApproval={vi.fn()} />,
    );
    expect(document.querySelector("[data-assistant-halo]")).toBeInTheDocument();
    expect(
      screen.queryByRole("status", { name: "Assistant is thinking" }),
    ).not.toBeInTheDocument();
    expect(document.querySelector("[data-assistant-halo]")).not.toHaveClass(
      "assistant-halo--visible",
    );

    act(() => vi.advanceTimersByTime(499));
    expect(document.querySelector("[data-assistant-halo]")).toBeInTheDocument();
    act(() => vi.advanceTimersByTime(1));
    expect(
      document.querySelector("[data-assistant-halo]"),
    ).not.toBeInTheDocument();
  });

  it("shows dots where the answer will appear until a turn has printed", () => {
    const userMessage = message({
      role: "user",
      blocks: [{ type: "text", block_id: "text-1", text: "Question" }],
    });
    const { rerender } = render(
      <ChatThread
        messages={[userMessage]}
        thinking
        onDecideApproval={vi.fn()}
      />,
    );

    expect(
      document.querySelector("[data-streaming-dots]"),
    ).toBeInTheDocument();

    // Content arrives: the dots must give way to it, not sit alongside.
    rerender(
      <ChatThread
        messages={[
          userMessage,
          message({ blocks: [{ type: "text", block_id: "t2", text: "Hi" }] }),
        ]}
        streaming
        onDecideApproval={vi.fn()}
      />,
    );

    expect(screen.getByText("Hi")).toBeInTheDocument();
    expect(
      document.querySelector("[data-streaming-dots]"),
    ).not.toBeInTheDocument();
  });

  it("keeps the dots up for a started message whose blocks are still blank", () => {
    // An opened-but-empty text block is present-yet-blank; counting blocks
    // rather than printable content would drop the dots onto an empty column.
    render(
      <ChatThread
        messages={[
          message({
            role: "user",
            blocks: [{ type: "text", block_id: "t1", text: "Question" }],
          }),
          message({ blocks: [{ type: "text", block_id: "t2", text: "" }] }),
        ]}
        streaming
        onDecideApproval={vi.fn()}
      />,
    );

    expect(
      document.querySelector("[data-streaming-dots]"),
    ).toBeInTheDocument();
  });

  it("reports an error when a turn closes having printed nothing", () => {
    vi.useFakeTimers();
    render(
      <ChatThread
        messages={[
          message({
            role: "user",
            blocks: [{ type: "text", block_id: "t1", text: "Question" }],
          }),
        ]}
        turnEnded
        onDecideApproval={vi.fn()}
      />,
    );

    // Held back briefly: turn status and transcript projection race, so a turn
    // that did answer can look empty for a frame.
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();

    act(() => vi.advanceTimersByTime(700));

    expect(
      screen.getByText("Sorry, there seems to be an error with the request for now."),
    ).toBeInTheDocument();
  });

  it("reports an error for a closed turn whose only block stayed blank", () => {
    vi.useFakeTimers();
    render(
      <ChatThread
        messages={[
          message({
            role: "user",
            blocks: [{ type: "text", block_id: "t1", text: "Question" }],
          }),
          message({ blocks: [{ type: "text", block_id: "t2", text: "" }] }),
        ]}
        turnEnded
        onDecideApproval={vi.fn()}
      />,
    );
    act(() => vi.advanceTimersByTime(700));

    // Carried inside the existing assistant group — a second identity mark for
    // a group that is already there would read as two separate turns.
    expect(screen.getAllByRole("alert")).toHaveLength(1);
    expect(document.querySelectorAll("[data-empty-turn-error]")).toHaveLength(1);
  });

  it("does not let the empty-conversation screen bury a live turn", () => {
    // A first turn can die before any history row materializes, leaving the
    // transcript bare. Showing "start a conversation" there hides the fact that
    // anything happened at all.
    const { rerender } = render(
      <ChatThread messages={[]} thinking onDecideApproval={vi.fn()} />,
    );

    expect(
      screen.queryByText("Start a new conversation"),
    ).not.toBeInTheDocument();
    expect(
      document.querySelector("[data-streaming-dots]"),
    ).toBeInTheDocument();

    rerender(<ChatThread messages={[]} onDecideApproval={vi.fn()} />);
    expect(screen.getByText("Start a new conversation")).toBeInTheDocument();
  });

  it("does not let the empty-conversation screen bury a closed-empty turn", () => {
    vi.useFakeTimers();
    render(<ChatThread messages={[]} turnEnded onDecideApproval={vi.fn()} />);
    act(() => vi.advanceTimersByTime(700));

    expect(screen.getByRole("alert")).toBeInTheDocument();
    expect(
      screen.queryByText("Start a new conversation"),
    ).not.toBeInTheDocument();
  });

  it("waits for an in-flight transcript read instead of guessing at it", () => {
    vi.useFakeTimers();
    const messages = [
      message({
        role: "user",
        blocks: [{ type: "text", block_id: "t1", text: "Question" }],
      }),
    ];
    const { rerender } = render(
      <ChatThread
        messages={messages}
        turnEnded
        transcriptSettling
        onDecideApproval={vi.fn()}
      />,
    );
    // However slow the read is, a pending one is never called an error.
    act(() => vi.advanceTimersByTime(10_000));
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();

    rerender(
      <ChatThread messages={messages} turnEnded onDecideApproval={vi.fn()} />,
    );
    act(() => vi.advanceTimersByTime(700));
    expect(screen.getByRole("alert")).toBeInTheDocument();
  });

  it("makes a new settle episode wait even if the last one had settled", () => {
    vi.useFakeTimers();
    const userMessage = message({
      role: "user",
      blocks: [{ type: "text", block_id: "t1", text: "Question" }],
    });
    const { rerender } = render(
      <ChatThread
        messages={[userMessage]}
        turnEnded
        onDecideApproval={vi.fn()}
      />,
    );
    act(() => vi.advanceTimersByTime(700));
    expect(screen.getByRole("alert")).toBeInTheDocument();

    // Condition drops and returns within one macrotask: the second episode must
    // re-serve its own grace period, not inherit the first one's verdict.
    rerender(
      <ChatThread
        messages={[userMessage]}
        turnEnded
        transcriptSettling
        onDecideApproval={vi.fn()}
      />,
    );
    rerender(
      <ChatThread
        messages={[userMessage]}
        turnEnded
        onDecideApproval={vi.fn()}
      />,
    );
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();

    act(() => vi.advanceTimersByTime(700));
    expect(screen.getByRole("alert")).toBeInTheDocument();
  });

  it("keeps the dots up for a continuation appended to an answered turn", () => {
    // An approval continuation is appended to the SAME assistant group and
    // reuses the turn id, so the group's earlier content is indistinguishable
    // from the continuation's own. Only the episode can say it printed nothing.
    render(
      <ChatThread
        messages={[
          message({
            role: "user",
            blocks: [{ type: "text", block_id: "t1", text: "Question" }],
          }),
          message({
            id: "answered",
            blocks: [{ type: "text", block_id: "t2", text: "Earlier answer" }],
          }),
        ]}
        streaming
        turnPrinted={false}
        onDecideApproval={vi.fn()}
      />,
    );

    expect(screen.getByText("Earlier answer")).toBeInTheDocument();
    expect(
      document.querySelector("[data-streaming-dots]"),
    ).toBeInTheDocument();
  });

  it("reports a continuation that closed empty behind an answered turn", () => {
    vi.useFakeTimers();
    render(
      <ChatThread
        messages={[
          message({
            role: "user",
            blocks: [{ type: "text", block_id: "t1", text: "Question" }],
          }),
          message({
            id: "answered",
            blocks: [{ type: "text", block_id: "t2", text: "Earlier answer" }],
          }),
        ]}
        turnEnded
        turnPrinted={false}
        onDecideApproval={vi.fn()}
      />,
    );
    act(() => vi.advanceTimersByTime(700));

    expect(screen.getByRole("alert")).toBeInTheDocument();
  });

  it("trusts the episode over the transcript when it says content printed", () => {
    vi.useFakeTimers();
    render(
      <ChatThread
        messages={[
          message({
            role: "user",
            blocks: [{ type: "text", block_id: "t1", text: "Question" }],
          }),
          // Blank in the transcript, but the episode saw content stream.
          message({ blocks: [{ type: "text", block_id: "t2", text: "" }] }),
        ]}
        turnEnded
        turnPrinted
        onDecideApproval={vi.fn()}
      />,
    );
    act(() => vi.advanceTimersByTime(5000));

    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("stays quiet when a closed turn did print an answer", () => {
    vi.useFakeTimers();
    render(
      <ChatThread
        messages={[
          message({
            role: "user",
            blocks: [{ type: "text", block_id: "t1", text: "Question" }],
          }),
          message({ blocks: [{ type: "text", block_id: "t2", text: "Answer" }] }),
        ]}
        turnEnded
        onDecideApproval={vi.fn()}
      />,
    );
    act(() => vi.advanceTimersByTime(5000));

    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("withdraws the pending error if content lands inside the grace window", () => {
    vi.useFakeTimers();
    const userMessage = message({
      role: "user",
      blocks: [{ type: "text", block_id: "t1", text: "Question" }],
    });
    const { rerender } = render(
      <ChatThread
        messages={[userMessage]}
        turnEnded
        onDecideApproval={vi.fn()}
      />,
    );
    act(() => vi.advanceTimersByTime(400));

    rerender(
      <ChatThread
        messages={[
          userMessage,
          message({ blocks: [{ type: "text", block_id: "t2", text: "Late" }] }),
        ]}
        turnEnded
        onDecideApproval={vi.fn()}
      />,
    );
    act(() => vi.advanceTimersByTime(5000));

    expect(screen.getByText("Late")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("removes live-region semantics during the thinking exit fade", () => {
    vi.useFakeTimers();
    const userMessage = message({
      role: "user",
      blocks: [{ type: "text", block_id: "text-1", text: "Question" }],
    });
    const { rerender } = render(
      <ChatThread
        messages={[userMessage]}
        thinking
        onDecideApproval={vi.fn()}
      />,
    );
    act(() => vi.advanceTimersByTime(0));

    rerender(
      <ChatThread messages={[userMessage]} onDecideApproval={vi.fn()} />,
    );

    const exitingHalo = document.querySelector("[data-assistant-halo]");
    expect(exitingHalo).toBeInTheDocument();
    expect(
      screen.queryByRole("status", { name: "Assistant is thinking" }),
    ).not.toBeInTheDocument();
    expect(exitingHalo?.closest("article")).toHaveAttribute(
      "aria-hidden",
      "true",
    );
  });

  it("suppresses an exiting halo when a new turn starts", () => {
    vi.useFakeTimers();
    const assistantMessage = message({
      id: "assistant-previous",
      blocks: [{ type: "text", block_id: "text-1", text: "Answer" }],
    });
    const userMessage = message({
      id: "user-current",
      role: "user",
      blocks: [{ type: "text", block_id: "text-2", text: "Follow-up" }],
    });
    const { rerender } = render(
      <ChatThread
        messages={[assistantMessage]}
        streaming
        onDecideApproval={vi.fn()}
      />,
    );
    act(() => vi.advanceTimersByTime(0));

    rerender(
      <ChatThread
        messages={[assistantMessage, userMessage]}
        thinking
        onDecideApproval={vi.fn()}
      />,
    );

    expect(document.querySelectorAll("[data-assistant-halo]")).toHaveLength(1);
    expect(
      screen.getByRole("status", { name: "Assistant is thinking" }),
    ).toContainElement(document.querySelector("[data-assistant-halo]"));
  });

  it("dispatches action_card blocks to the rich action renderer", () => {
    render(
      <ChatThread
        messages={[
          message({
            blocks: [
              {
                type: "action_card",
                block_id: "action-card-1",
                action: "service.connect",
                action_request_id: "act-1",
                origin_turn_id: "turn-1",
                params: {
                  variant: "catalog",
                  service_slug: "api-github",
                  requested_scopes: ["repo"],
                  via_node_id: null,
                  target_org_id: null,
                },
                status: "pending",
                outcome_note: "",
              },
            ],
          }),
        ]}
        onDecideApproval={vi.fn()}
      />,
    );

    expect(screen.getByTestId("action-card-dispatch")).toHaveTextContent(
      "act-1",
    );
  });
});
