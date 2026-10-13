import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import * as telemetry from "@/lib/telemetry";
import { api } from "@/lib/api-client";
import {
  ChatMessageBubble,
  ChatMessageEntry,
  ChatMessageList,
} from "@/components/assistant/chat-message";
import {
  AevatarAssistantMessageList,
  NyxAgentThreadMessageList,
  NyxAgentAssistantEntry,
} from "@/components/assistant/assistant-chat-page";
import type { ChatMessage, ChatSessionState } from "@/lib/assistant/chat-types";

const BASE: ChatMessage = {
  id: "assistant-1",
  role: "assistant",
  content: "Answer",
  timestamp: 1,
  status: "complete",
};

describe("canonical chat presentation", () => {
  it("shows feedback only in the page-owned NyxAgent message-list composition", () => {
    const session: ChatSessionState = {
      clientId: "test-client",
      conversationId: "nyxagent-page",
      status: "completed_text",
      title: "Test",
      messages: [BASE],
      expectedTurnCount: 0,
    };
    const retry = vi.fn().mockResolvedValue(undefined);
    const failed: ChatMessage = { ...BASE, id: "failed-1", status: "error", error: "Turn lost" };
    // The page's own assistant renderer, as the thread page passes it.
    const { rerender } = render(
      <NyxAgentThreadMessageList
        session={{ ...session, messages: [BASE, failed] }}
        bottomInset={0}
        renderMessage={(message) => (
          <NyxAgentAssistantEntry message={message} conversationId={session.conversationId} retry={retry} />
        )}
      />,
    );
    expect(screen.getByRole("button", { name: "Helpful" })).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(retry).toHaveBeenCalledWith("failed-1");

    rerender(
      <AevatarAssistantMessageList
        session={{ ...session, conversationId: "aevatar-page" }}
        bottomInset={0}
      />,
    );
    expect(
      screen.queryByRole("button", { name: "Helpful" }),
    ).not.toBeInTheDocument();
  });

  it("hides feedback in replay and without a persisted conversation", () => {
    const { rerender } = render(
      <ChatMessageBubble
        message={BASE}
        conversationId="conversation-1"
        feedbackEnabled={false}
      />,
    );
    expect(
      screen.queryByRole("button", { name: "Helpful" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Add feedback comment" }),
    ).not.toBeInTheDocument();
    rerender(<ChatMessageBubble message={BASE} />);
    expect(
      screen.queryByRole("group", { name: "Reply feedback" }),
    ).not.toBeInTheDocument();
  });
  it("gates the entire feedback control block behind the page flag", () => {
    render(
      <ChatMessageBubble
        message={BASE}
        conversationId="conversation-1"
        feedbackEnabled={false}
      />,
    );
    expect(
      screen.queryByRole("group", { name: "Reply feedback" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Helpful" }),
    ).not.toBeInTheDocument();
  });
  it("renders stopped replies without an error banner or feedback controls", () => {
    render(
      <ChatMessageBubble
        message={{ ...BASE, errorCode: "cancelled" }}
        conversationId="conversation-1"
      />,
    );
    expect(
      screen.queryByText("The assistant could not complete this turn."),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Helpful" }),
    ).not.toBeInTheDocument();
  });
  it("keeps feedback controls visibly pressed and blocks duplicate pending clicks", () => {
    render(
      <ChatMessageBubble
        message={{ ...BASE, feedback: { vote: "up" } }}
        conversationId="conversation-1"
      />,
    );
    expect(screen.getByRole("button", { name: "Helpful" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
  });
  it("does not capture telemetry when feedback submission fails", async () => {
    const put = vi
      .spyOn(api, "put")
      .mockRejectedValueOnce(new Error("offline"));
    const capture = vi.spyOn(telemetry, "capture");
    render(
      <ChatMessageBubble message={BASE} conversationId="conversation-1" />,
    );
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Helpful" }));
    });
    expect(capture).not.toHaveBeenCalled();
    capture.mockRestore();
    put.mockRestore();
  });
  afterEach(() => vi.useRealTimers());

  it("opens the optional feedback comment popover without submitting", () => {
    render(
      <ChatMessageBubble message={BASE} conversationId="conversation-1" />,
    );
    fireEvent.click(
      screen.getByRole("button", { name: "Add feedback comment" }),
    );
    expect(
      screen.getByRole("textbox", { name: "Optional feedback comment" }),
    ).toBeVisible();
    expect(screen.getByRole("button", { name: "Send" })).toBeVisible();
  });
  it("keeps comment submission optional", () => {
    render(
      <ChatMessageBubble message={BASE} conversationId="conversation-1" />,
    );
    expect(
      screen.getByRole("button", { name: "Add feedback comment" }),
    ).toBeVisible();
    expect(
      screen.queryByRole("textbox", { name: "Optional feedback comment" }),
    ).not.toBeInTheDocument();
  });

  it("renders an inline context reset as a system note without assistant controls", () => {
    const content =
      "Conversation context was reset; the assistant was given a recap of this chat.";
    render(
      <ChatMessageEntry
        message={{
          ...BASE,
          id: "nyxagent-context-reset:conversation",
          role: "system",
          content,
        }}
      />,
    );
    expect(
      screen.getByRole("note", { name: "Conversation context reset" }),
    ).toHaveTextContent(content);
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });

  it("composes reasoning, actions, sanitized Markdown, and an error", () => {
    const { container } = render(
      <ChatMessageBubble
        message={{
          ...BASE,
          content: "Before <function_calls>secret()</function_calls> **after**",
          thinking: "Checking state",
          steps: [
            {
              id: "step-1",
              name: "Inspect",
              status: "done",
              startedAt: 1,
              finishedAt: 2,
            },
          ],
          toolCalls: [
            {
              id: "tool-1",
              name: "catalog.read",
              status: "running",
              startedAt: 1,
            },
          ],
          status: "error",
          error: "Run failed",
        }}
      />,
    );
    expect(container.querySelector("[data-assistant-halo]")).toBeNull();
    expect(
      screen.getByRole("status", { name: "Assistant reasoning" }),
    ).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: /2 actions/i }));
    expect(screen.getByText("Inspect")).toBeVisible();
    expect(screen.getAllByText("catalog.read")[0]).toBeVisible();
    expect(screen.queryByText(/secret/)).not.toBeInTheDocument();
    expect(screen.getByText("after").tagName).toBe("STRONG");
    expect(screen.getByText("Run failed")).toBeVisible();
  });

  it("ChatMessageEntry keeps feedback controls and forwards Retry (page renderMessage contract)", () => {
    const retry = vi.fn();
    const { unmount } = render(
      <ChatMessageEntry
        message={{ ...BASE, authorName: "NyxBot" }}
        conversationId="conversation-1"
        feedbackEnabled
        onRetry={undefined}
      />,
    );
    expect(screen.getByText("NyxBot")).toBeVisible();
    expect(
      screen.getByRole("button", { name: "Add feedback comment" }),
    ).toBeVisible();
    unmount();
    render(
      <ChatMessageEntry
        message={{ ...BASE, status: "error", error: "Turn lost" }}
        conversationId="conversation-1"
        feedbackEnabled
        onRetry={retry}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(retry).toHaveBeenCalledTimes(1);
  });

  it("exposes Retry for a failed reply", async () => {
    const retry = vi.fn();
    render(
      <ChatMessageBubble
        message={{ ...BASE, status: "error", error: "Turn lost" }}
        onRetry={retry}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(retry).toHaveBeenCalledOnce();
  });

  it("shows a rejected retry beside its Retry button", async () => {
    const retry = vi.fn().mockRejectedValue(new Error("A turn is already active."));
    render(
      <ChatMessageBubble
        message={{ ...BASE, status: "error", error: "Turn lost" }}
        onRetry={retry}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("A turn is already active.");
    expect(screen.getByRole("button", { name: "Retry" })).toBeEnabled();
  });

  it("shows every diagnostic field and copies the diagnostic ID", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    render(
      <ChatMessageBubble
        message={{
          ...BASE,
          status: "error",
          error: "Stopped responding",
          errorDetail: {
            stage: "no_progress",
            httpStatus: 503,
            upstreamRequestId: "req-1",
            responseId: "resp_1",
            continuations: 2,
            turnId: "turn-1",
            occurredAt: "2026-01-01T00:00:00Z",
          },
        }}
      />,
    );
    fireEvent.click(screen.getByText("Details"));
    for (const value of ["no_progress", "503", "turn-1", "resp_1", "req-1", "2"]) {
      expect(screen.getByText(value)).toBeInTheDocument();
    }
    expect(screen.getByRole("time")).toHaveAttribute("datetime", "2026-01-01T00:00:00Z");
    fireEvent.click(screen.getByRole("button", { name: "Copy diagnostic ID" }));
    expect(writeText).toHaveBeenCalledWith("turn turn-1 / response resp_1 / request req-1");
  });

  it("shows thinking treatment only for a content-free streaming message", () => {
    const { container, rerender } = render(
      <ChatMessageBubble
        message={{ ...BASE, content: "", status: "streaming" }}
      />,
    );
    expect(
      screen.getByRole("status", { name: "Assistant is thinking" }),
    ).toBeVisible();
    expect(container.querySelector("[data-assistant-halo]")).not.toBeNull();
    expect(container.querySelector("[data-streaming-dots]")).not.toBeNull();
    rerender(
      <ChatMessageBubble
        message={{ ...BASE, content: "Writing", status: "streaming" }}
      />,
    );
    expect(
      screen.queryByRole("status", { name: "Assistant is thinking" }),
    ).not.toBeInTheDocument();
    expect(container.querySelector("[data-assistant-halo]")).toBeNull();
    expect(container.querySelector("[data-streaming-dots]")).toBeNull();
    expect(container.querySelector("[data-streaming-caret]")).not.toBeNull();
  });

  it("shows each action's outcome and counts failures", () => {
    render(
      <ChatMessageBubble
        message={{
          ...BASE,
          toolCalls: [
            { id: "a", name: "nyx__search_tools", status: "done", startedAt: 1, finishedAt: 2 },
            { id: "b", name: "api-github-4__get_issue", status: "error", startedAt: 2, finishedAt: 3 },
            { id: "c", name: "api-github-4__get_file_contents", status: "running", startedAt: 3 },
          ],
        }}
      />,
    );
    const header = screen.getByRole("button", { name: /3 actions/i });
    expect(header).toHaveTextContent("· 1 failed");
    fireEvent.click(header);
    expect(screen.getByText("Searched for tools").closest("summary")).toHaveAttribute("data-tool-status", "done");
    expect(screen.getByLabelText("Succeeded")).toBeInTheDocument();
    expect(screen.getByText("Couldn't get issue on GitHub").closest("summary")).toHaveAttribute("data-tool-status", "error");
    expect(screen.getByLabelText("Failed")).toBeInTheDocument();
    expect(screen.getByText("Reading file contents on GitHub").closest("summary")).toHaveAttribute("data-tool-status", "running");
    expect(screen.getByText("api-github-4__get_issue")).toBeInTheDocument();
  });

  it("names the running tool beside the streaming dots until text arrives", () => {
    const running = {
      ...BASE,
      content: "",
      status: "streaming",
      toolCalls: [
        {
          id: "t1",
          name: "nyx__search_tools",
          status: "done" as const,
          startedAt: 1,
          finishedAt: 2,
        },
        {
          id: "t2",
          name: "github__list_issues",
          status: "running" as const,
          startedAt: 2,
        },
      ],
    };
    const { container, rerender } = render(
      <ChatMessageBubble message={running} />,
    );
    expect(container.querySelector("[data-running-tool]")).toHaveTextContent(
      "Listing issues on GitHub…",
    );
    expect(screen.getByRole("button", { name: /2 actions/i })).toBeVisible();
    rerender(
      <ChatMessageBubble
        message={{
          ...running,
          toolCalls: running.toolCalls.map((tool) => ({
            ...tool,
            status: "done" as const,
          })),
        }}
      />,
    );
    expect(container.querySelector("[data-running-tool]")).toBeNull();
    expect(container.querySelector("[data-last-tool]")).toHaveTextContent(
      "Listed issues on GitHub",
    );
    rerender(
      <ChatMessageBubble message={{ ...running, content: "Found 3 issues" }} />,
    );
    expect(container.querySelector("[data-running-tool]")).toBeNull();
    rerender(
      <ChatMessageBubble message={{ ...running, status: "complete" }} />,
    );
    expect(container.querySelector("[data-running-tool]")).toBeNull();
  });

  it("renders tool images from the authenticated client and reports failures", async () => {
    const png = new Uint8Array([
      0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a,
    ]);
    const seen: string[] = [];
    globalThis.__nyxidAssistantHttpMock = ({ endpoint }) => {
      seen.push(endpoint);
      return endpoint.endsWith("/missing")
        ? new Response(JSON.stringify({ message: "Attachment not found" }), {
            status: 404,
          })
        : new Response(png, { headers: { "content-type": "image/png" } });
    };
    const createObjectURL = vi.fn(() => "blob:tool-image");
    const revokeObjectURL = vi.fn();
    const original = {
      createObjectURL: URL.createObjectURL,
      revokeObjectURL: URL.revokeObjectURL,
    };
    Object.assign(URL, { createObjectURL, revokeObjectURL });
    const image = (id: string) => ({
      id,
      endpoint: `/assistant/nyxagent/conversations/c/attachments/${id}`,
      contentType: "image/png",
      label: "lobby-camera__snapshot",
    });
    const { unmount } = render(
      <ChatMessageBubble
        message={{
          ...BASE,
          content: "Here is the lobby.",
          images: [image("ok"), image("missing")],
        }}
      />,
    );
    const img = await screen.findByRole("img", {
      name: "Image from lobby-camera__snapshot",
    });
    expect(img).toHaveAttribute("src", "blob:tool-image");
    expect(await screen.findByText("Image unavailable")).toBeVisible();
    expect(seen).toEqual([
      "/assistant/nyxagent/conversations/c/attachments/ok",
      "/assistant/nyxagent/conversations/c/attachments/missing",
    ]);
    unmount();
    expect(revokeObjectURL).toHaveBeenCalledWith("blob:tool-image");
    Object.assign(URL, original);
    globalThis.__nyxidAssistantHttpMock = undefined;
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
    expect(
      screen.queryByRole("button", { name: /approve/i }),
    ).not.toBeInTheDocument();
  });

  it("renders accumulated media with the shipped artifact block", () => {
    const { container } = render(
      <ChatMessageBubble
        message={{
          ...BASE,
          content: "",
          artifacts: [
            {
              type: "artifact",
              block_id: "artifact-1",
              artifact_id: "media-1",
              name: "chart.png",
              mime: "image/png",
              size_bytes: 6,
              preview: null,
              download_url: "data:image/png;base64,aGVsbG8=",
            },
          ],
        }}
      />,
    );
    expect(screen.getByText("chart.png")).toBeVisible();
    expect(
      screen.getByRole("link", { name: "Download chart.png" }),
    ).toHaveAttribute("href", "data:image/png;base64,aGVsbG8=");
    expect(container.querySelector("[data-assistant-halo]")).toBeNull();
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
    expect(
      screen.getByRole("article", { name: "Policy agent auditor message" }),
    ).toBeVisible();
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

it("focuses the comment field and closes it with Escape, showing save failures", async () => {
  render(<ChatMessageBubble message={BASE} conversationId="conversation-1" />);
  fireEvent.click(screen.getByRole("button", { name: "Add feedback comment" }));
  const textarea = screen.getByRole("textbox", {
    name: "Optional feedback comment",
  });
  expect(textarea).toHaveFocus();
  fireEvent.keyDown(textarea, { key: "Escape" });
  expect(
    screen.queryByRole("textbox", { name: "Optional feedback comment" }),
  ).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Add feedback comment" }));
  fireEvent.click(screen.getByRole("button", { name: "Helpful" }));
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
});
