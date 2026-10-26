import { createMemoryHistory, createRootRoute, createRoute, createRouter, RouterProvider } from "@tanstack/react-router";
import { withAccountPanelSearch } from "@/lib/assistant/account-panel-search";
import { CAPACITY_ACKNOWLEDGEMENT } from "@/lib/assistant/capacity-wait";
import { act, fireEvent, render, screen, within } from "@testing-library/react";
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
  ChannelUserEntry,
  NyxAgentThreadMessageList,
  NyxAgentAssistantEntry,
} from "@/components/assistant/assistant-chat-page";
import type { ChatMessage, ChatSessionState } from "@/lib/assistant/chat-types";
import { messageTime } from "@/lib/assistant/message-time";
import { describeToolActivity } from "@/lib/assistant/tool-activity-text";

const BASE: ChatMessage = {
  id: "assistant-1",
  role: "assistant",
  content: "Answer",
  timestamp: 1,
  status: "complete",
};

describe("canonical chat presentation", () => {
  it.each([
    ["insufficient_credits", "Not enough credits", "Add credits in Billing & Usage"],
    ["wallet_suspended", "Billing wallet suspended", "Contact support"],
    ["plan_entitlement_required", "Service not included in your plan", "Check your plan"],
    ["billing_not_configured", "Billing setup incomplete", "Contact support to complete the billing configuration"],
    ["billing_provider_unavailable", "Billing service unavailable", "Try again shortly"],
  ])("shows the billing cause and remedy before diagnostics for %s", (code, title, remedy) => {
    render(<ChatMessageBubble message={{ ...BASE, content: "", status: "error", errorCode: code,
      error: "Turn failed. Provide a trace.",
      toolCalls: [{ id: "tool", name: "nyx__search_tools", status: "done", startedAt: 1 }],
      errorDetail: { stage: "stream", turnId: "turn-billing", continuations: 0 },
    }} />);
    const alert = screen.getByRole("alert");
    expect(alert).toHaveTextContent(title);
    expect(alert).toHaveTextContent(remedy);
    expect(screen.queryByText("Turn failed. Provide a trace.")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /1 action/ })).toHaveTextContent(title);
    const details = screen.getByText("Details");
    expect(alert.compareDocumentPosition(details) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("shows the panic stage in failure Details", () => {
    render(<ChatMessageBubble message={{ ...BASE, status: "error", content: "",
      errorCode: "internal_error",
      error: "NyxBot hit an internal error on this turn. Check which actions completed before retrying.",
      errorDetail: { stage: "panic", turnId: "turn-panic", occurredAt: "2026-10-15T00:00:00Z", continuations: 0 },
    }} />);
    expect(screen.getByText("Details")).toBeInTheDocument();
    expect(screen.getByText("panic")).toBeInTheDocument();
    expect(screen.getByText(/Check which actions completed/)).toBeInTheDocument();
  });
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
    const failed: ChatMessage = { ...BASE, id: "failed-1", status: "error", error: "Turn lost",
      recovery: { status: "available", reason: "available", text: "Question", attachments: [] } };
    // The page's own assistant renderer, as the thread page passes it.
    const { rerender } = render(
      <NyxAgentThreadMessageList
        session={{ ...session, messages: [BASE, failed] }}
        bottomInset={0}
        renderMessage={(message) => (
          <NyxAgentAssistantEntry message={message} conversationId={session.conversationId}
            retry={retry} checkRecovery={vi.fn()} />
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

  it("checks temporary recovery status without posting and reports copy failure", async () => {
    const check = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { value: { writeText: vi.fn().mockRejectedValue(new Error("denied")) }, configurable: true });
    render(<ChatMessageBubble message={{ ...BASE, status: "error", recovery: {
      status: "review", reason: "status_unavailable", text: "Original request", attachments: ["scan.txt"],
    } }} onCheckRecovery={check} />);
    expect(screen.queryByRole("button", { name: "Retry" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Check retry availability" }));
    await screen.findByRole("button", { name: "Check retry availability" });
    expect(check).toHaveBeenCalledOnce();
    fireEvent.click(screen.getByRole("button", { name: "Review request" }));
    expect(screen.getByText("Original request")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Copy request" }));
    expect(await screen.findByRole("status")).toHaveTextContent("Copy failed");
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
            upstreamCode: "turn_failed",
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
    for (const value of ["turn_failed", "no_progress", "503", "turn-1", "resp_1", "req-1", "2"]) {
      expect(screen.getByText(value)).toBeInTheDocument();
    }
    expect(
      within(screen.getByText("Details").closest("details")!).getByRole("time"),
    ).toHaveAttribute("datetime", "2026-01-01T00:00:00Z");
    fireEvent.click(screen.getByRole("button", { name: "Copy diagnostic ID" }));
    expect(writeText).toHaveBeenCalledWith("turn turn-1 / upstream turn_failed / response resp_1 / request req-1");
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

  it("counts the seconds a reply has waited, as thinking and then beside a running tool", () => {
    vi.useFakeTimers();
    try {
      const waiting = { ...BASE, content: "", status: "streaming" as const };
      const { container, rerender } = render(<ChatMessageBubble message={waiting} />);
      expect(container.querySelector("[data-waiting-status]")).toBeNull();
      act(() => void vi.advanceTimersByTime(3000));
      expect(container.querySelector("[data-waiting-status]")).toHaveTextContent("Thinking · 3s");
      rerender(
        <ChatMessageBubble
          message={{
            ...waiting,
            toolCalls: [{ id: "t", name: "github__list_issues", status: "running", startedAt: 1 }],
          }}
        />,
      );
      act(() => void vi.advanceTimersByTime(4000));
      expect(container.querySelector("[data-running-tool]")).toHaveTextContent(
        `${describeToolActivity("github__list_issues", "running")}… · 7s`,
      );
    } finally {
      vi.useRealTimers();
    }
  });

  it("re-renders only the rows whose message changed while a reply streams", () => {
    const session: ChatSessionState = {
      clientId: "client", conversationId: "nyxagent-page", status: "streaming", title: "Test",
      messages: [], expectedTurnCount: 1,
    };
    const settled: ChatMessage = { ...BASE, id: "settled" };
    const live: ChatMessage = { ...BASE, id: "live", content: "A", status: "streaming" };
    const rendered: string[] = [];
    const renderMessage = (message: ChatMessage) => {
      rendered.push(message.id);
      return <p>{message.content}</p>;
    };
    const { rerender } = render(
      <ChatMessageList session={{ ...session, messages: [settled, live] }} bottomInset={0} renderMessage={renderMessage} />,
    );
    rendered.length = 0;
    rerender(
      <ChatMessageList
        session={{ ...session, messages: [settled, { ...live, content: "AB" }] }}
        bottomInset={0}
        renderMessage={(message) => renderMessage(message)}
      />,
    );
    expect(rendered).toEqual(["live"]);
    expect(screen.getByText("AB")).toBeVisible();
    rendered.length = 0;
    rerender(
      <ChatMessageList
        session={{ ...session, messages: [settled, { ...live, content: "AB" }] }}
        bottomInset={0}
        renderMessage={renderMessage}
        projectionVersion="card:1"
      />,
    );
    expect(rendered).toEqual(["settled", "live"]);
  });

  it("mounts a bounded window of a long transcript, ending at the newest message", () => {
    // happy-dom has no layout: give every element a 600px box.
    const rect = vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({
      x: 0, y: 0, top: 0, left: 0, right: 600, bottom: 600, width: 600, height: 600, toJSON: () => ({}),
    } as DOMRect);
    const width = vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(600);
    const height = vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(600);
    const offsetWidth = vi.spyOn(HTMLElement.prototype, "offsetWidth", "get").mockReturnValue(600);
    const offsetHeight = vi.spyOn(HTMLElement.prototype, "offsetHeight", "get").mockReturnValue(600);
    try {
      const messages = Array.from({ length: 200 }, (_, index): ChatMessage => ({
        ...BASE, id: `m-${String(index)}`, content: `Message ${String(index)}`,
        role: index % 2 ? "assistant" : "user",
      }));
      const { container } = render(
        <ChatMessageList
          session={{ clientId: "c", conversationId: "nyxagent-long", status: "completed_text", title: "Long", messages, expectedTurnCount: 100 }}
          bottomInset={0}
        />,
      );
      const rows = container.querySelectorAll("[id^=message-]");
      expect(rows.length).toBeGreaterThan(0);
      expect(rows.length).toBeLessThan(20);
      expect(container.querySelector("#message-m-199")).not.toBeNull();
      expect(container.querySelector("#message-m-0")).toBeNull();
    } finally {
      rect.mockRestore();
      width.mockRestore();
      height.mockRestore();
      offsetWidth.mockRestore();
      offsetHeight.mockRestore();
    }
  });

  it("starts windowing only while the reader follows the newest message", () => {
    const width = vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(600);
    const height = vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(600);
    try {
      const session = (count: number): ChatSessionState => ({
        clientId: "c", conversationId: "nyxagent-growing", status: "completed_text", title: "T",
        expectedTurnCount: 0,
        messages: Array.from({ length: count }, (_, index): ChatMessage => ({ ...BASE, id: `m-${String(index)}` })),
      });
      const { container, rerender } = render(<ChatMessageList session={session(30)} bottomInset={0} />);
      const scroller = container.querySelector<HTMLDivElement>("[id^=message-]")!.closest<HTMLDivElement>(".overflow-y-auto")!;
      let top = 0;
      Object.defineProperty(scroller, "scrollHeight", { configurable: true, get: () => 5000 });
      Object.defineProperty(scroller, "clientHeight", { configurable: true, get: () => 600 });
      Object.defineProperty(scroller, "scrollTop", { configurable: true, get: () => top, set: (value: number) => { top = value; } });
      const scrollTo = (value: number) => { top = value; fireEvent.scroll(scroller); };
      scrollTo(4400);
      scrollTo(1000); // the reader scrolls up
      rerender(<ChatMessageList session={session(31)} bottomInset={0} />);
      expect(container.querySelectorAll("[id^=message-]")).toHaveLength(31);
      expect(top).toBe(1000);
      scrollTo(4400); // back at the newest message
      rerender(<ChatMessageList session={session(32)} bottomInset={0} />);
      expect(container.querySelectorAll("[id^=message-]").length).toBeLessThan(32);
    } finally {
      width.mockRestore();
      height.mockRestore();
    }
  });

  it("shows a reply's images, and a message's attachments, above its text", () => {
    // The image stays loading: only its position matters here.
    globalThis.__nyxidAssistantHttpMock = () => new Promise<Response>(() => undefined);
    const image = { id: "img", endpoint: "/assistant/nyxagent/conversations/c/attachments/img", contentType: "image/png", label: "chart.png" };
    for (const role of ["assistant", "user"] as const) {
      const { container, unmount } = render(
        <ChatMessageBubble message={{ ...BASE, role, content: "Here is the chart.", images: [image] }} />,
      );
      const html = container.innerHTML;
      expect(html.indexOf("Loading image from chart.png")).toBeGreaterThan(-1);
      expect(html.indexOf("Loading image from chart.png")).toBeLessThan(html.indexOf("Here is the chart."));
      unmount();
    }
    globalThis.__nyxidAssistantHttpMock = undefined;
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
    const scroller = container.querySelector<HTMLDivElement>("[data-chat-transcript]")!;
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

describe("turn status and action ledger", () => {
  it("keeps total turn time across tool changes, remounts and returning to the tab", () => {
    vi.useFakeTimers();
    try {
      vi.setSystemTime(new Date("2026-10-21T00:00:30Z"));
      const startedAt = "2026-10-21T00:00:00Z";
      const message = { ...BASE, status: "streaming" as const, turnStatus: { phase: "tool" as const, label: "Running first", startedAt, toolStartedAt: "2026-10-21T00:00:20Z" } };
      const { container, rerender, unmount } = render(<ChatMessageBubble message={message} />);
      expect(container.querySelector("[data-turn-status]")).toHaveTextContent("30s total");
      rerender(<ChatMessageBubble message={{ ...message, turnStatus: { ...message.turnStatus, toolStartedAt: "2026-10-21T00:00:29Z", label: "Running second" } }} />);
      expect(container.querySelector("[data-turn-status]")).toHaveTextContent("30s total");
      rerender(<ChatMessageBubble message={{ ...message, turnStatus: { phase: "working", label: "Working…", startedAt } }} />);
      expect(container.querySelector("[data-turn-status]")).toHaveTextContent("30s total");
      // Browser background throttling skips ticks; visibility must catch up at once.
      vi.setSystemTime(new Date("2026-10-21T00:01:05Z"));
      fireEvent(document, new Event("visibilitychange"));
      expect(container.querySelector("[data-turn-status]")).toHaveTextContent("1:05 total");
      unmount();
      const reopened = render(<ChatMessageBubble message={message} />);
      expect(reopened.container.querySelector("[data-turn-status]")).toHaveTextContent("1:05 total");
    } finally {
      vi.useRealTimers();
    }
  });

  it("keeps each tool's own duration beside the correct action", () => {
    vi.useFakeTimers();
    try {
      vi.setSystemTime(new Date("2026-10-21T00:00:30Z"));
      const from = Date.parse("2026-10-21T00:00:00Z");
      const { container } = render(<ChatMessageBubble message={{ ...BASE, toolCalls: [
        { id: "finished", name: "github__read", status: "done", startedAt: from, finishedAt: from + 8000 },
        { id: "running", name: "github__search", status: "running", startedAt: from + 20000 },
      ] }} />);
      fireEvent.click(screen.getByRole("button", { name: /2 actions/ }));
      const done = container.querySelector('[data-tool-status="done"]')!;
      const running = container.querySelector('[data-tool-status="running"]')!;
      expect(done).toHaveTextContent("8s");
      expect(running).toHaveTextContent("10s");
      act(() => vi.advanceTimersByTime(5000));
      expect(done).toHaveTextContent("8s");
      expect(running).toHaveTextContent("15s");
    } finally {
      vi.useRealTimers();
    }
  });
  it.each(["approval", "retrying", "continuing", "working", "active"] as const)("keeps %s status visible after reply text", (phase) => {
    const labels = { approval: "Waiting for approval", retrying: "Retrying…", continuing: "Continuing task…", working: "Working…", active: "Working — the reply appears when it is ready" };
    const { container, rerender } = render(<ChatMessageBubble message={{ ...BASE, status: "streaming", turnStatus: { phase, label: labels[phase] } }} />);
    const row = container.querySelector("[data-turn-status]")!;
    expect(row).toHaveTextContent(labels[phase]);
    expect(row.className).not.toContain("animate");
    expect(row.querySelector("span")?.className ?? "").not.toContain("animate");
    for (const status of ["stopped", "error", "complete"]) {
      rerender(<ChatMessageBubble message={{ ...BASE, status }} />);
      expect(container.querySelector("[data-turn-status]")).toBeNull();
    }
  });

  it("shows elapsed tool time and the interruption notice", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-10-13T00:00:12Z"));
    const { rerender } = render(<ChatMessageBubble message={{ ...BASE, status: "streaming", turnStatus: { phase: "tool", label: "Running github__list", toolStartedAt: "2026-10-13T00:00:00Z", interrupted: true } }} />);
    expect(screen.getByText("· 12s")).toBeVisible();
    expect(screen.getByText("Connection interrupted; checking every few seconds")).toBeVisible();
    const phase = screen.getByRole("status");
    expect(phase).toHaveTextContent("Connection interrupted; checking every few seconds");
    const announcement = phase.textContent;
    act(() => vi.advanceTimersByTime(48_000));
    expect(screen.getByText("· 1:00")).toBeVisible();
    expect(phase.textContent).toBe(announcement);
    expect(phase).not.toHaveTextContent("1:00");
    rerender(<ChatMessageBubble message={{ ...BASE, status: "streaming", turnStatus: { phase: "tool", label: "Running github__list", toolStartedAt: "2026-10-13T00:00:00Z", interrupted: false } }} />);
    expect(phase).not.toHaveTextContent("Connection interrupted");
    expect(screen.getByText("· 1:00")).toBeVisible();
    vi.useRealTimers();
  });

  it.each(["streaming", "complete", "error", "stopped"])("renders %s with an earlier failed action truthfully", (status) => {
    render(<ChatMessageBubble message={{ ...BASE, status, activityTotal: 41, turnStatus: { phase: "working", label: "Working…" }, toolCalls: [{ id: "f", name: "github__read", status: "error", startedAt: 1, failure: { kind: "service_status", status: 404 } }] }} />);
    const button = screen.getByRole("button", { name: /action/ });
    expect(button).toHaveTextContent("41 actions");
    expect(button).toHaveTextContent("latest 1 of 41 shown");
    if (status === "streaming") expect(screen.queryByLabelText("Turn failed")).toBeNull();
    if (status === "complete") {
      expect(button).toHaveTextContent("Completed");
      expect(button).toHaveTextContent("1 failed");
      expect(screen.getByText(/1 failed/).className).toContain("text-warning");
    }
    fireEvent.click(button);
    expect(screen.getByLabelText("Failed")).toBeVisible();
    expect(screen.getByText(/The service responded with HTTP 404/)).toHaveTextContent("The assistant can continue after a failed step.");
  });
  it.each(["complete", "error", "stopped"])("shows unknown details for legacy or turn-terminated steps in a %s turn", (status) => {
    render(<ChatMessageBubble message={{ ...BASE, status, toolCalls: [{ id: "legacy", name: "tool", status: "error", startedAt: 1 }] }} />);
    fireEvent.click(screen.getByRole("button", { name: /action/ }));
    expect(screen.getByLabelText("Failed")).toBeVisible();
    expect(screen.getByText(/This step did not complete/)).toHaveTextContent("No additional error details are available. The assistant can continue after a failed step.");
    expect(screen.queryByText(/The tool reported an error/)).toBeNull();
  });

  it("uses tool-error copy only for an explicit tool error", () => {
    render(<ChatMessageBubble message={{ ...BASE, status: "complete", toolCalls: [{ id: "error", name: "tool", status: "error", startedAt: 1, failure: { kind: "tool_error" } }] }} />);
    fireEvent.click(screen.getByRole("button", { name: /action/ }));
    expect(screen.getByText(/The tool reported an error/)).toBeVisible();
  });

});


it.each(["", "Partial reply"])("shows a persistent stopped label without activities: %s", (content) => {
  const message = { ...BASE, content, status: "stopped" };
  const { container, rerender } = render(<ChatMessageBubble message={message} />);
  expect(screen.getByRole("status", { name: "Turn stopped" })).toHaveTextContent("Stopped");
  expect(container.querySelector("[data-turn-status]")).toBeNull();
  rerender(<ChatMessageList session={{ clientId: "chat", conversationId: "chat", expectedTurnCount: 1, title: "Chat", status: "completed_text", messages: [message] }} bottomInset={0} />);
  expect(screen.getByRole("status", { name: "Turn stopped" })).toBeVisible();
  expect(container.querySelector("[data-empty-turn-error]")).toBeNull();
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

it("separates the described tool label from elapsed time without duplicate Thinking", () => {
  vi.useFakeTimers();
  try {
    vi.setSystemTime(new Date("2026-10-13T00:00:12Z"));
    const { container } = render(<ChatMessageBubble message={{
      ...BASE,
      content: "",
      status: "streaming",
      turnStatus: { phase: "tool", label: "Running github__read_issue", toolStartedAt: "2026-10-13T00:00:00Z" },
      toolCalls: [{ id: "read", name: "github__read_issue", status: "running", startedAt: 1 }],
    }} />);
    const row = container.querySelector("[data-turn-status]")!;
    expect(row).toHaveTextContent("Reading issue on GitHub… · 12s");
    expect(row).not.toHaveTextContent("Thinking");
    expect(container.querySelectorAll("[data-running-tool]")).toHaveLength(1);
    const live = row.querySelector('[aria-live="polite"]')!;
    const label = live.textContent;
    act(() => vi.advanceTimersByTime(48000));
    expect(live.textContent).toBe(label);
    expect(row).toHaveTextContent("Reading issue on GitHub… · 1:00");
  } finally {
    vi.useRealTimers();
  }
});

it("shows one working label with elapsed time only after three seconds", () => {
  vi.useFakeTimers();
  try {
    const { container } = render(<ChatMessageBubble message={{
      ...BASE, status: "streaming", turnStatus: { phase: "working", label: "Working…" },
    }} />);
    const row = container.querySelector("[data-turn-status]")!;
    expect(row).toHaveTextContent(/^Working…$/);
    act(() => vi.advanceTimersByTime(2000));
    expect(row).toHaveTextContent(/^Working…$/);
    act(() => vi.advanceTimersByTime(1000));
    expect(row).toHaveTextContent(/^Working… · 3s$/);
    expect(row).not.toHaveTextContent("Thinking");
  } finally {
    vi.useRealTimers();
  }
});

it("does not invent an HTTP status when service failure metadata lacks it", () => {
  render(<ChatMessageBubble message={{ ...BASE, status: "complete", toolCalls: [{
    id: "failed", name: "tool", status: "error", startedAt: 1, failure: { kind: "service_status" },
  }] }} />);
  fireEvent.click(screen.getByRole("button", { name: /action/ }));
  expect(screen.getByText(/The service returned an error/)).toHaveTextContent(
    "No HTTP status is available. The assistant can continue after a failed step.",
  );
  expect(screen.queryByText(/HTTP undefined/)).toBeNull();
});

it("renders a decision continuation as a system status without the owner's instruction bubble", () => {
  render(<ChatMessageBubble message={{ ...BASE, role: "user", content: "Confirmed: attach it now", continuation_of: "11111111-1111-4111-8111-111111111111" }} />);
  expect(screen.getByRole("status")).toHaveTextContent("Continuing after your decision");
  expect(screen.queryByText("Confirmed: attach it now")).not.toBeInTheDocument();
});

describe("message timestamps", () => {
  const SENT = "2026-10-12T07:40:43.000Z";
  const at = Date.parse(SENT);

  function sentTime(container: HTMLElement) {
    return container.querySelector(`time[datetime="${SENT}"]`);
  }

  it("stamps user messages and settled replies with their local send time", () => {
    const expected = messageTime(at)!;
    for (const message of [
      { ...BASE, id: "user-1", role: "user", timestamp: at },
      { ...BASE, timestamp: at },
    ] satisfies ChatMessage[]) {
      const { container, unmount } = render(
        <ChatMessageEntry message={message} conversationId="conversation-1" />,
      );
      const time = sentTime(container);
      expect(time).toHaveTextContent(expected.label);
      expect(time).not.toHaveAttribute("title");
      unmount();
    }
  });

  it("stamps a user message written in a chat app", () => {
    const { container } = render(
      <ChannelUserEntry
        message={{ ...BASE, id: "user-1", role: "user", content: "From Telegram", timestamp: at, via: "telegram" }}
        platform="telegram"
      />,
    );
    expect(screen.getByText("From Telegram")).toBeInTheDocument();
    expect(sentTime(container)).toHaveTextContent(messageTime(at)!.label);
  });

  it("puts a reply's time on its feedback line, and keeps it without feedback", () => {
    const { container, rerender } = render(
      <ChatMessageBubble message={{ ...BASE, timestamp: at }} conversationId="conversation-1" />,
    );
    const feedback = screen.getByRole("group", { name: "Reply feedback" });
    expect(feedback.parentElement).toContainElement(sentTime(container) as HTMLElement);
    rerender(
      <ChatMessageBubble
        message={{ ...BASE, timestamp: at, status: "error", error: "Turn lost" }}
        conversationId="conversation-1"
      />,
    );
    expect(screen.queryByRole("group", { name: "Reply feedback" })).not.toBeInTheDocument();
    expect(sentTime(container)).toBeInTheDocument();
  });

  it("waits for a streaming reply to settle instead of stamping each update", () => {
    const { container } = render(
      <ChatMessageBubble message={{ ...BASE, timestamp: Date.now(), status: "streaming" }} />,
    );
    expect(container.querySelector("time")).toBeNull();
  });

  it("omits a missing or invalid time rather than inventing one", () => {
    for (const timestamp of [Number.NaN, Date.parse("not a date")]) {
      const { container, unmount } = render(
        <>
          <ChatMessageBubble message={{ ...BASE, id: "user-1", role: "user", timestamp }} />
          <ChatMessageBubble message={{ ...BASE, timestamp }} />
        </>,
      );
      expect(container.querySelector("time")).toBeNull();
      expect(container).not.toHaveTextContent("Invalid Date");
      unmount();
    }
  });

  it("stamps every restored history row in a transcript", () => {
    const earlier = "2026-10-11T22:05:00.000Z";
    const session: ChatSessionState = {
      clientId: "history",
      conversationId: "conversation-1",
      status: "completed_text",
      title: "History",
      expectedTurnCount: 2,
      messages: [
        { ...BASE, id: "user-1", role: "user", content: "Yesterday", timestamp: Date.parse(earlier) },
        { ...BASE, id: "assistant-1", timestamp: Date.parse(earlier) + 1000 },
        { ...BASE, id: "user-2", role: "user", content: "Today", timestamp: at },
        { ...BASE, id: "assistant-2", timestamp: at + 1000 },
      ],
    };
    const { container } = render(<ChatMessageList session={session} bottomInset={0} />);
    const times = [...container.querySelectorAll("time")].map((time) => time.getAttribute("datetime"));
    expect(times).toEqual([
      earlier,
      new Date(Date.parse(earlier) + 1000).toISOString(),
      SENT,
      new Date(at + 1000).toISOString(),
    ]);
  });
});

it("shows capacity waiting without an error and links billing remedies", () => {
  const { rerender } = render(<ChatMessageBubble message={{ ...BASE, content: "", status: "streaming", waitingForCapacity: true }} />);
  expect(screen.getByText(/Waiting for the assistant/)).toBeVisible();
  rerender(<ChatMessageBubble message={{ ...BASE, status: "error", errorCode: "wallet_suspended", error: "Wallet suspended" }} />);
  expect(screen.queryByRole("button", { name: "Open Billing & Usage" })).not.toBeInTheDocument();
  expect(screen.queryByText("Details")).not.toBeInTheDocument();
});

it("labels a queued reply with its queue clock", () => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date("2026-10-15T00:00:12Z"));
  render(<ChatMessageBubble message={{ ...BASE, content: "", status: "streaming", waitingForCapacity: true, queued: true,
    capacityWaitStartedAt: "2026-10-15T00:00:00Z", capacityWaitDeadline: "2026-10-15T00:10:00Z" }} />);
  expect(screen.queryByText(/Waiting for the assistant/)).not.toBeInTheDocument();
  expect(screen.getByText(/Queued until another reply finishes/).closest("[data-waiting-status]")).toHaveTextContent("0:12 of up to 10:00");
  vi.useRealTimers();
});

it("restores the shared capacity clock after remount and clears it on acceptance", () => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date("2026-10-15T00:00:23Z"));
  const message: ChatMessage = { ...BASE, content: "", status: "streaming", waitingForCapacity: true, systemNote: "capacity_wait",
    capacityWaitStartedAt: "2026-10-15T00:00:00Z", capacityWaitDeadline: "2026-10-15T00:02:00Z",
    turnStatus: { phase: "active", label: "Working — the reply appears when it is ready" } };
  const first = render(<ChatMessageBubble message={message} />);
  expect(screen.queryByText(/Working —/)).not.toBeInTheDocument();
  expect(screen.getByText(/Waiting for the assistant/).closest("[data-waiting-status]")).toHaveTextContent("0:23 of up to 2:00");
  act(() => vi.advanceTimersByTime(1000));
  expect(screen.getByText(/Waiting for the assistant/).closest("[data-waiting-status]")).toHaveTextContent("0:24 of up to 2:00");
  first.unmount();
  vi.setSystemTime(new Date("2026-10-15T00:00:40Z"));
  const second = render(<ChatMessageBubble message={message} />);
  expect(screen.getByText(/Waiting for the assistant/).closest("[data-waiting-status]")).toHaveTextContent("0:40 of up to 2:00");
  second.rerender(<ChatMessageBubble message={{ ...message, waitingForCapacity: false, systemNote: undefined }} />);
  expect(screen.queryByText(/Waiting for the assistant/)).not.toBeInTheDocument();
  expect(screen.getByText(/Working —/)).toBeVisible();
  vi.useRealTimers();
});

it("shows attempt and capacity wait details for failed and successful replies", () => {
  const { rerender } = render(<ChatMessageBubble message={{ ...BASE, status: "error", error: "Failed",
    errorDetail: { stage: "stream", turnId: "turn", continuations: 0, attempts: 7, capacityWaitMs: 80_000 } }} />);
  expect(screen.getByText("attempts").nextElementSibling).toHaveTextContent("7");
  expect(screen.getByText("waited 1 minute 20 seconds for capacity")).toBeInTheDocument();
  rerender(<ChatMessageBubble message={{ ...BASE, turnMetadata: { attempts: 3, capacityWaitMs: 33_000 } }} />);
  expect(screen.getByText("attempts").nextElementSibling).toHaveTextContent("3");
  expect(screen.getByText("waited 33 seconds for capacity")).toBeInTheDocument();
});

it("renders the system acknowledgement in the pending agent bubble and replaces it on acceptance, exhaustion and Stop", () => {
  const copy = CAPACITY_ACKNOWLEDGEMENT;
  const pending: ChatMessage = { ...BASE, content: "", status: "streaming", systemNote: "capacity_wait", waitingForCapacity: false };
  const { container, rerender } = render(<ChatMessageEntry message={{ ...pending, authorName: "Ada" }} conversationId="chat" feedbackEnabled />);
  expect(screen.getByText("Ada")).toBeVisible();
  expect(screen.getByRole("note")).toHaveTextContent(copy);
  expect(container.querySelector('[data-system-note="capacity_wait"]')).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: /Good response|Bad response/ })).not.toBeInTheDocument();
  rerender(<ChatMessageEntry message={{ ...BASE, content: "The real reply", status: "streaming", authorName: "Ada" }} />);
  expect(screen.queryByText(copy)).not.toBeInTheDocument();
  expect(screen.getByText("The real reply")).toBeVisible();
  rerender(<ChatMessageEntry message={{ ...pending, status: "error", errorCode: "capacity_exceeded", error: "NyxBot is at capacity right now (waited 2 minutes). Try again shortly." }} />);
  expect(screen.queryByText(copy)).not.toBeInTheDocument();
  expect(screen.getByText(/waited 2 minutes/)).toBeVisible();
  rerender(<ChatMessageEntry message={{ ...pending, status: "complete", errorCode: "cancelled" }} />);
  expect(screen.queryByText(copy)).not.toBeInTheDocument();
  expect(container.querySelector('[data-system-note]')).not.toBeInTheDocument();
});

it("hides ordinary successful Details and shows retries without a wait", () => {
  const { rerender } = render(<ChatMessageBubble message={{ ...BASE, turnMetadata: { attempts: 1, capacityWaitMs: 0 } }} />);
  expect(screen.queryByText("Details")).not.toBeInTheDocument();
  rerender(<ChatMessageBubble message={{ ...BASE, turnMetadata: { attempts: 2, capacityWaitMs: 0 } }} />);
  expect(screen.getByText("Details")).toBeInTheDocument();
  expect(screen.queryByText(/waited/)).not.toBeInTheDocument();
  rerender(<ChatMessageBubble message={{ ...BASE, status: "error", errorDetail: {
    stage: "stream", turnId: "turn", continuations: 0, attempts: 1, capacityWaitMs: 0,
  } }} />);
  expect(screen.getByText("Details")).toBeInTheDocument();
});

it.each([
  ["/assistant", "wallet_suspended"],
  ["/assistant/automations", "wallet_suspended"],
  ["/replay", "wallet_suspended"],
  ["/assistant", "insufficient_credits"],
  ["/assistant", "plan_entitlement_required"],
])("opens billing over the current assistant shell only: %s (%s)", async (path, code) => {
  const root = createRootRoute();
  const route = createRoute({ getParentRoute: () => root, path,
    validateSearch: withAccountPanelSearch((search) => ({ c: String(search.c ?? "") })),
    component: () => <ChatMessageBubble message={{ ...BASE, status: "error", errorCode: code }} />,
  });
  const router = createRouter({ routeTree: root.addChildren([route]),
    history: createMemoryHistory({ initialEntries: [`${path}?c=current-chat&panel=settings&panelTab=profile`] }),
  });
  await router.load();
  render(<RouterProvider router={router} />);
  if (path === "/replay") {
    expect(screen.queryByRole("button", { name: "Open Billing & Usage" })).not.toBeInTheDocument();
    return;
  }
  fireEvent.click(await screen.findByRole("button", { name: "Open Billing & Usage" }));
  await vi.waitFor(() => expect(router.state.location.search).toEqual({ c: "current-chat", panel: "billing" }));
  expect(router.state.location.pathname).toBe(path);
});


describe("message body copy integration", () => {
  it("copies sanitized assistant Markdown without thinking, activities or controls", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
    const markdown = "**回答**\n\n```rust\n    let n = 1;\n```";
    render(<ChatMessageBubble message={{ ...BASE, content: `${markdown}\n<function_calls><invoke>private-tool-input</invoke></function_calls>`, thinking: "private-thought" }} />);
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Copy message" })));
    expect(writeText).toHaveBeenCalledExactlyOnceWith(markdown);
    expect(await screen.findByRole("button", { name: "Copied" })).toBeVisible();
  });
  it("copies the user's original body including literal markup and whitespace", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
    const original = "  <function_calls>example</function_calls>\n```js\n  x()\n```  ";
    render(<ChatMessageBubble message={{ ...BASE, role: "user", content: original }} />);
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Copy message" })));
    expect(writeText).toHaveBeenCalledExactlyOnceWith(original);
  });
  it.each([
    { content: "", status: "complete" as const },
    { content: "Streaming text", status: "streaming" as const },
    { content: "<function_calls>hidden</function_calls>", status: "complete" as const },
  ])("has no body copy control for $status / $content", (message) => {
    render(<ChatMessageBubble message={{ ...BASE, ...message }} />);
    expect(screen.queryByRole("button", { name: "Copy message" })).not.toBeInTheDocument();
  });
});
