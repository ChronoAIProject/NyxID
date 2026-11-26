import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import fixtures from "../../../../shared/voice-message-groups.json";
import { api } from "@/lib/api-client";
import type { ChatMessage, ChatSessionState } from "@/lib/assistant/chat-types";
import { ChatMessageEntry } from "./chat-message";
import {
  NyxAgentAssistantEntry,
  NyxAgentThreadMessageList,
  VoiceMessageGroupContent,
} from "./assistant-chat-page";

afterEach(() => vi.restoreAllMocks());

it.each([
  ["XML", "<function_calls><invoke>private-only</invoke></function_calls>"],
  [
    "DSML",
    "<|DSML|function_calls><invoke>private-only</invoke></|DSML|function_calls>",
  ],
])(
  "grouped voice sanitizes each segment like a single row and copies sanitized joined text (%s)",
  async (_format, block) => {
    const rows = fixtures[0]!.rows.slice(0, 2).map((row, index) => ({
      ...row,
      role: "assistant",
      content: index === 0 ? `Before\n${block}\nAfter` : "Second answer",
      timestamp: index,
      status: "complete",
      authorName: "NyxBot",
    })) as ChatMessage[];
    const single = rows[0]!;
    const { container, rerender } = render(
      <NyxAgentAssistantEntry
        message={single}
        conversationId="sanitized-thread"
        retry={vi.fn()}
        checkRecovery={vi.fn()}
      />,
    );
    const singleContent = container.querySelector(".type-chat-message")!;
    expect(singleContent).toHaveTextContent("Before");
    expect(singleContent).toHaveTextContent("After");
    expect(singleContent).not.toHaveTextContent("private-only");
    const expectedMarkup = singleContent.innerHTML;
    const session: ChatSessionState = {
      clientId: "voice",
      conversationId: "sanitized-thread",
      title: "Voice",
      status: "completed_text",
      messages: rows,
      expectedTurnCount: 0,
    };
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText },
    });
    rerender(
      <NyxAgentThreadMessageList
        session={session}
        bottomInset={0}
        renderMessage={(message) => (
          <NyxAgentAssistantEntry
            message={message}
            conversationId={session.conversationId}
            retry={vi.fn()}
            checkRecovery={vi.fn()}
            contentOverride={
              <VoiceMessageGroupContent
                message={message}
                liveCaptions={[]}
                playbackMs={0}
              />
            }
          />
        )}
      />,
    );
    expect(container.querySelector(".type-chat-message")!.innerHTML).toBe(
      expectedMarkup,
    );
    expect(screen.getByText("Second answer")).toBeVisible();
    expect(container).not.toHaveTextContent("private-only");
    await act(async () =>
      fireEvent.click(screen.getByRole("button", { name: "Copy message" })),
    );
    expect(writeText).toHaveBeenCalledWith("Before\n\nAfter\n\nSecond answer");
  },
);

it.each([false, true])(
  "one voice group renders one author, Copy, date and feedback anchor (saved feedback: %s)",
  async (savedFeedback) => {
    const rows = fixtures[0]!.rows.map((row, index) => ({
      ...row,
      role: "assistant",
      content: "text" in row ? row.text : "",
      timestamp: 1_700_000_000_000 + index * 1000,
      status: "complete",
      authorName: "NyxBot",
      ...(savedFeedback && index === 1 ? { feedback: { vote: "up" } } : {}),
    })) as ChatMessage[];
    const session: ChatSessionState = {
      clientId: "voice",
      conversationId: "voice-thread",
      title: "Voice",
      status: "completed_text",
      messages: rows,
      expectedTurnCount: 0,
    };
    const put = vi.spyOn(api, "put").mockResolvedValue({});
    const remove = vi.spyOn(api, "delete").mockResolvedValue(undefined);
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText },
    });
    const { container } = render(
      <NyxAgentThreadMessageList
        session={session}
        bottomInset={0}
        renderMessage={(message) => (
          <NyxAgentAssistantEntry
            message={message}
            conversationId={session.conversationId}
            retry={vi.fn()}
            checkRecovery={vi.fn()}
          />
        )}
      />,
    );
    expect(screen.getAllByText("NyxBot · Voice")).toHaveLength(1);
    expect(
      screen.getAllByRole("button", { name: "Copy message" }),
    ).toHaveLength(1);
    expect(screen.getAllByRole("button", { name: "Helpful" })).toHaveLength(1);
    expect(container.querySelectorAll("time")).toHaveLength(1);
    for (const row of rows) expect(screen.getByText(row.content)).toBeVisible();
    await act(async () =>
      fireEvent.click(screen.getByRole("button", { name: "Copy message" })),
    );
    expect(writeText).toHaveBeenCalledWith(fixtures[0]!.copy![0]);
    const anchor = savedFeedback ? "7" : "8";
    const path = `/assistant/nyxagent/conversations/voice-thread/messages/${anchor}/feedback`;
    await act(async () =>
      fireEvent.click(screen.getByRole("button", { name: "Not helpful" })),
    );
    expect(put).toHaveBeenCalledWith(path, { vote: "down" });
    await act(async () =>
      fireEvent.click(screen.getByRole("button", { name: "Cancel" })),
    );
    await act(async () =>
      fireEvent.click(screen.getByRole("button", { name: "Not helpful" })),
    );
    expect(remove).toHaveBeenCalledWith(path);
    expect(rows).toHaveLength(3);
  },
);

it.each([false, true])(
  "appending speech preserves the first web row and DOM node while feedback uses its anchor (saved feedback: %s)",
  async (savedFeedback) => {
    const rows = fixtures[0]!.rows.map((row, index) => ({
      ...row,
      role: "assistant",
      content: "text" in row ? row.text : "",
      timestamp: index,
      status: "complete",
      authorName: "NyxBot",
      ...(savedFeedback && index === 1 ? { feedback: { vote: "up" } } : {}),
    })) as ChatMessage[];
    const session: ChatSessionState = {
      clientId: "voice",
      conversationId: "stable-thread",
      title: "Voice",
      status: "completed_text",
      messages: rows.slice(0, 1),
      expectedTurnCount: 0,
    };
    const put = vi.spyOn(api, "put").mockResolvedValue({});
    const view = (value: ChatSessionState) => (
      <NyxAgentThreadMessageList
        session={value}
        bottomInset={0}
        renderMessage={(message) => (
          <NyxAgentAssistantEntry
            message={message}
            conversationId={value.conversationId}
            retry={vi.fn()}
            checkRecovery={vi.fn()}
          />
        )}
      />
    );
    const { container, rerender } = render(view(session));
    const first = container.querySelector("#message-6");
    const bubble = container.querySelector("#message-6 article");
    expect(first).not.toBeNull();
    expect(bubble).not.toBeNull();
    rerender(view({ ...session, messages: rows }));
    expect(container.querySelector("#message-6")).toBe(first);
    expect(container.querySelector("#message-6 article")).toBe(bubble);
    expect(screen.getByRole("button", { name: "Helpful" })).toHaveAttribute(
      "aria-pressed",
      String(savedFeedback),
    );
    expect(container.querySelector("#message-8")).toBeNull();
    await act(async () =>
      fireEvent.click(screen.getByRole("button", { name: "Not helpful" })),
    );
    expect(put).toHaveBeenCalledWith(
      `/assistant/nyxagent/conversations/stable-thread/messages/${savedFeedback ? "7" : "8"}/feedback`,
      { vote: "down" },
    );
  },
);
it("voice attribution marks assistant speech and leaves user attribution unchanged", () => {
  const base = {
    id: "voice",
    content: "Hello",
    status: "complete",
    timestamp: 0,
    authorName: "Owner",
    voice: { session_id: "s", segment_id: "p", sealed: true, complete: true },
  } as const;
  const { rerender } = render(
    <ChatMessageEntry message={{ ...base, role: "user" }} />,
  );
  expect(screen.queryByText("Owner · Voice")).toBeNull();
  rerender(<ChatMessageEntry message={{ ...base, role: "assistant" }} />);
  expect(screen.getByText("Owner · Voice")).toBeVisible();
});
