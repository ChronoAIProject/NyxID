import { act, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AssistantMessage } from "@/types/assistant";
import { ChatThread } from "./chat-thread";

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
});
