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
    expect(container.querySelector("[data-assistant-halo]")).not.toBeNull();
    expect(screen.getByRole("status", { name: "Assistant reasoning" })).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: /2 actions/i }));
    expect(screen.getByText("Inspect")).toBeVisible();
    expect(screen.getByText("catalog.read")).toBeVisible();
    expect(screen.queryByText(/secret/)).not.toBeInTheDocument();
    expect(screen.getByText("after").tagName).toBe("STRONG");
    expect(screen.getByText("Run failed")).toBeVisible();
  });

  it("shows live dots only for a content-free streaming message", () => {
    const { container, rerender } = render(
      <ChatMessageBubble message={{ ...BASE, content: "", status: "streaming" }} />,
    );
    expect(container.querySelector("[data-streaming-dots]")).not.toBeNull();
    rerender(
      <ChatMessageBubble message={{ ...BASE, content: "Writing", status: "streaming" }} />,
    );
    expect(container.querySelector("[data-streaming-dots]")).toBeNull();
    expect(container.querySelector("[data-streaming-caret]")).not.toBeNull();
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
});
