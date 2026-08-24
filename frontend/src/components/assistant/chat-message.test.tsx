import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  ChatMessageBubble,
  ChatMessageEntry,
  ChatMessageList,
} from "@/components/assistant/chat-message";
import type { ChatMessage, ChatSessionState } from "@/lib/assistant/chat-types";

const BASE: ChatMessage = {
  id: "assistant-1",
  role: "assistant",
  content: "Answer",
  timestamp: 1,
  status: "complete",
};

describe("canonical chat presentation", () => {
  afterEach(() => vi.useRealTimers());

  it("composes reasoning, actions, sanitized Markdown, and an error", () => {
    const { container } = render(
      <ChatMessageBubble
        message={{
          ...BASE,
          content: "Before <function_calls>secret()</function_calls> **after**",
          thinking: "Checking state",
          steps: [{
            id: "step-1",
            name: "Inspect",
            status: "done",
            startedAt: 1,
            finishedAt: 2,
          }],
          toolCalls: [{
            id: "tool-1",
            name: "catalog.read",
            status: "running",
            startedAt: 1,
          }],
          status: "error",
          error: "Run failed",
        }}
      />,
    );
    expect(container.querySelector("[data-assistant-halo]")).toBeNull();
    expect(screen.getByRole("status", { name: "Assistant reasoning" })).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: /2 actions/i }));
    expect(screen.getByText("Inspect")).toBeVisible();
    expect(screen.getByText("catalog.read")).toBeVisible();
    expect(screen.queryByText(/secret/)).not.toBeInTheDocument();
    expect(screen.getByText("after").tagName).toBe("STRONG");
    expect(screen.getByText("Run failed")).toBeVisible();
  });

  it("shows thinking treatment only for a content-free streaming message", () => {
    const { container, rerender } = render(
      <ChatMessageBubble message={{ ...BASE, content: "", status: "streaming" }} />,
    );
    expect(
      screen.getByRole("status", { name: "Assistant is thinking" }),
    ).toBeVisible();
    expect(container.querySelector("[data-assistant-halo]")).not.toBeNull();
    expect(container.querySelector("[data-streaming-dots]")).not.toBeNull();
    rerender(
      <ChatMessageBubble message={{ ...BASE, content: "Writing", status: "streaming" }} />,
    );
    expect(
      screen.queryByRole("status", { name: "Assistant is thinking" }),
    ).not.toBeInTheDocument();
    expect(container.querySelector("[data-assistant-halo]")).toBeNull();
    expect(container.querySelector("[data-streaming-dots]")).toBeNull();
    expect(container.querySelector("[data-streaming-caret]")).not.toBeNull();
  });

  it("does not render accumulator approval or workflow intervention cards", () => {
    render(
      <ChatMessageBubble
        message={{
          ...BASE,
          pendingApproval: {
            requestId: "runtime-approval",
            toolName: "catalog.write",
            toolCallId: "tool-call-1",
            argumentsJson: "{}",
            isDestructive: true,
            timeoutSeconds: 30,
          },
          pendingRunIntervention: {
            key: "runtime-input",
            kind: "human_input",
            prompt: "Supply workflow input",
            runId: "run-1",
            stepId: "step-1",
          },
        }}
      />,
    );

    expect(screen.queryByText("catalog.write")).not.toBeInTheDocument();
    expect(screen.queryByText("Supply workflow input")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /approve/i })).not.toBeInTheDocument();
  });

  it("renders author labels and nonstandard roles without dropping content", () => {
    const { rerender } = render(
      <ChatMessageEntry message={{ ...BASE, authorName: "NyxID Operator" }} />,
    );
    expect(screen.getByText("NyxID Operator")).toBeVisible();
    rerender(
      <ChatMessageEntry
        message={{ ...BASE, role: "auditor", authorName: "Policy agent" }}
      />,
    );
    expect(screen.getByRole("article", { name: "Policy agent auditor message" })).toBeVisible();
    expect(screen.getByText("Answer")).toBeVisible();
  });

  it("keeps empty-turn detection observable without a visible alert", async () => {
    vi.useFakeTimers();
    const session: ChatSessionState = {
      clientId: "session-1",
      conversationId: "nyxid-chat-alpha",
      expectedTurnCount: 1,
      messages: [{ ...BASE, content: "" }],
      status: "completed_text",
      title: "Empty",
    };
    const { container } = render(
      <ChatMessageList session={session} bottomInset={0} />,
    );
    expect(container.querySelector("[data-empty-turn-error]")).toBeNull();
    await act(async () => vi.advanceTimersByTimeAsync(700));
    expect(container.querySelector("[data-empty-turn-error]")).not.toBeNull();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("follows the tail, suspends on upward scroll, and resumes for a new user message", () => {
    const first: ChatSessionState = {
      clientId: "session-scroll",
      conversationId: "nyxid-chat-scroll",
      expectedTurnCount: 1,
      messages: [
        {
          id: "user-1",
          role: "user",
          content: "First prompt",
          timestamp: 1,
          status: "complete",
        },
      ],
      status: "streaming",
      title: "Scroll",
    };
    const { container, rerender } = render(
      <ChatMessageList session={first} bottomInset={0} />,
    );
    const scroller = container.firstElementChild as HTMLDivElement;
    let scrollHeight = 1_000;
    Object.defineProperty(scroller, "scrollHeight", {
      configurable: true,
      get: () => scrollHeight,
    });
    Object.defineProperty(scroller, "clientHeight", {
      configurable: true,
      get: () => 200,
    });

    const answering: ChatSessionState = {
      ...first,
      messages: [
        ...first.messages,
        {
          id: "assistant-1",
          role: "assistant",
          content: "Writing",
          timestamp: 2,
          status: "streaming",
        },
      ],
    };
    rerender(<ChatMessageList session={answering} bottomInset={0} />);
    expect(scroller.scrollTop).toBe(1_000);

    scroller.scrollTop = 600;
    fireEvent.scroll(scroller);
    scrollHeight = 1_200;
    rerender(
      <ChatMessageList
        session={{
          ...answering,
          messages: answering.messages.map((message) =>
            message.id === "assistant-1"
              ? { ...message, content: "Writing more" }
              : message,
          ),
        }}
        bottomInset={0}
        projectionVersion={2}
      />,
    );
    expect(scroller.scrollTop).toBe(600);

    const steered: ChatSessionState = {
      ...answering,
      messages: [
        ...answering.messages,
        {
          id: "user-2",
          role: "user",
          content: "Use the shorter version",
          timestamp: 3,
          status: "complete",
        },
      ],
    };
    rerender(
      <ChatMessageList
        session={steered}
        bottomInset={0}
        projectionVersion={3}
      />,
    );
    expect(scroller.scrollTop).toBe(1_200);
  });
});
