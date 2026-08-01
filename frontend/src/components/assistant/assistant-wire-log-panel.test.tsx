import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { TooltipProvider } from "@/components/ui/tooltip";
import { useAssistantContextStore } from "@/stores/assistant-context-store";
import { useAssistantDraftStore } from "@/stores/assistant-draft-store";
import { useAssistantWireLogStore } from "@/stores/assistant-wire-log-store";
import type { User } from "@/types/api";
import {
  AssistantWireLogAction,
  AssistantWireLogPanel,
} from "./assistant-wire-log-panel";

const admin: User = {
  id: "admin-1",
  email: "admin@example.com",
  display_name: "Admin",
  avatar_url: null,
  email_verified: true,
  mfa_enabled: false,
  is_admin: true,
  role: "admin",
  is_active: true,
  created_at: "2026-07-31T00:00:00.000Z",
};

function renderWithTooltips(node: React.ReactNode) {
  return render(<TooltipProvider>{node}</TooltipProvider>);
}

function responseEnvelope() {
  return {
    degraded: false as const,
    method: "POST",
    path: "api/chat",
    commandType: "text",
    body: { type: "text", prompt: "inspect this payload" },
    headers: { accept: "text/event-stream" },
    identity: {
      mode: "jwt",
      forward_access_token: false,
      inject_delegation_token: true,
      bridge_minted: false,
    },
    truncated: false,
    response: {
      status: 202,
      headers: {
        "content-type": { value: "text/event-stream", truncated: false },
        "x-request-id": { value: "aevatar-request-1", truncated: false },
      },
      sse: true,
    },
    upstreamOutcome: "response" as const,
  };
}

function sseLines(frames: readonly Record<string, unknown>[]) {
  return frames.flatMap((frame) => [
    { text: `data: ${JSON.stringify(frame)}`, ending: "\n" as const },
    { text: "", ending: "\n" as const },
  ]);
}

describe("AssistantWireLogPanel", () => {
  beforeEach(() => {
    localStorage.clear();
    useAssistantWireLogStore.setState({
      captureEnabled: true,
      showResponses: true,
      entries: [],
      totalBytes: 0,
      captureBytes: 0,
    });
    vi.stubGlobal(
      "ResizeObserver",
      class {
        observe() {}
        unobserve() {}
        disconnect() {}
      },
    );
  });

  it("renders, expands, and clears captured entries", () => {
    useAssistantWireLogStore.getState().recordExchange(
      [
        {
          degraded: false,
          method: "POST",
          path: "api/chat",
          commandType: "text",
          body: { type: "text", prompt: "inspect this payload" },
          headers: { accept: "text/event-stream" },
          identity: {
            mode: "jwt",
            forward_access_token: false,
            inject_delegation_token: true,
            bridge_minted: false,
          },
          truncated: false,
          response: null,
          upstreamOutcome: "no_response",
        },
      ],
      "sse",
      200,
    );
    renderWithTooltips(<AssistantWireLogPanel />);

    fireEvent.click(screen.getByRole("button", { name: "Aevatar wire log" }));
    expect(screen.getByText("/api/chat")).toBeInTheDocument();
    fireEvent.click(screen.getByText("/api/chat").closest("button")!);
    expect(screen.getByText(/inspect this payload/)).toBeInTheDocument();
    expect(screen.getByText(/"path": "\/api\/chat"/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /clear/i }));
    expect(screen.getByText("No captured requests")).toBeInTheDocument();
    expect(useAssistantWireLogStore.getState().entries).toHaveLength(0);
  });

  it("hides the icon for a non-admin user", () => {
    renderWithTooltips(
      <AssistantWireLogAction
        user={{ ...admin, is_admin: false, role: "user" }}
      />,
    );

    expect(
      screen.queryByRole("button", { name: "Aevatar wire log" }),
    ).not.toBeInTheDocument();
  });

  it("gates every response-derived surface with the top Responses switch", () => {
    const exchangeId = useAssistantWireLogStore
      .getState()
      .recordExchange([responseEnvelope()], "sse", 200);
    if (!exchangeId) throw new Error("expected exchange");
    useAssistantWireLogStore
      .getState()
      .attachWireLines(
        exchangeId,
        sseLines([
          { type: "RUN_STARTED", turnId: "turn-panel" },
          { type: "RUN_FINISHED" },
        ]),
        100,
        false,
      );
    useAssistantWireLogStore.getState().finalizeCapture(exchangeId, "complete");
    renderWithTooltips(<AssistantWireLogPanel />);

    fireEvent.click(screen.getByRole("button", { name: "Aevatar wire log" }));
    expect(
      screen.getByRole("switch", { name: "Show Aevatar responses" }),
    ).toBeChecked();
    expect(screen.getByText("Aevatar 202")).toBeInTheDocument();
    fireEvent.click(screen.getByText("/api/chat").closest("button")!);
    expect(screen.getByText("Upstream response 1")).toBeInTheDocument();
    expect(screen.getByText("Delivered response")).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Raw" })).toBeInTheDocument();

    fireEvent.click(
      screen.getByRole("switch", { name: "Show Aevatar responses" }),
    );
    expect(screen.queryByText("Aevatar 202")).not.toBeInTheDocument();
    expect(screen.queryByText("Upstream response 1")).not.toBeInTheDocument();
    expect(screen.queryByText("Delivered response")).not.toBeInTheDocument();
    expect(screen.queryByRole("tab", { name: "Raw" })).not.toBeInTheDocument();
    expect(screen.getByText(/inspect this payload/)).toBeInTheDocument();
  });

  it("mounts raw SSE lines in bounded windows", () => {
    const exchangeId = useAssistantWireLogStore
      .getState()
      .recordExchange([responseEnvelope()], "sse", 200);
    if (!exchangeId) throw new Error("expected exchange");
    const lines = Array.from({ length: 450 }, (_, index) => ({
      text: `line-${String(index)}`,
      ending: "\n" as const,
    }));
    useAssistantWireLogStore
      .getState()
      .attachWireLines(exchangeId, lines, 3_590, false);
    useAssistantWireLogStore.getState().finalizeCapture(exchangeId, "complete");
    renderWithTooltips(<AssistantWireLogPanel />);

    fireEvent.click(screen.getByRole("button", { name: "Aevatar wire log" }));
    fireEvent.click(screen.getByText("/api/chat").closest("button")!);
    expect(screen.getByText("line-199")).toBeInTheDocument();
    expect(screen.queryByText("line-200")).not.toBeInTheDocument();
    expect(screen.queryByText("line-449")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Show 200 more" }));
    expect(screen.getByText("line-200")).toBeInTheDocument();
    expect(screen.getByText("line-399")).toBeInTheDocument();
    expect(screen.queryByText("line-400")).not.toBeInTheDocument();
  });

  it("renders text and inert original-frame placeholders without side effects", () => {
    const exchangeId = useAssistantWireLogStore
      .getState()
      .recordExchange([responseEnvelope()], "sse", 200);
    if (!exchangeId) throw new Error("expected exchange");
    const connectFrame = {
      type: "CUSTOM",
      custom: {
        name: "nyxid.authorization.required",
        payload: {
          reasonCode: "NYXID_SERVICE_NOT_CONNECTED",
          serviceSlug: "api-github",
          serviceLabel: "GitHub",
          safeMessage: "Connect GitHub to continue.",
        },
      },
    };
    const lines = sseLines([
      { type: "RUN_STARTED", turnId: "turn-panel" },
      {
        type: "TEXT_MESSAGE_START",
        textMessageStart: { messageId: "message-panel", role: "assistant" },
      },
      {
        type: "TEXT_MESSAGE_CONTENT",
        textMessageContent: { delta: "Rendered **markdown**" },
      },
      {
        type: "TEXT_MESSAGE_END",
        textMessageEnd: { messageId: "message-panel" },
      },
      connectFrame,
      { type: "RUN_FINISHED", runFinished: { status: "blocked" } },
    ]);
    useAssistantWireLogStore
      .getState()
      .attachWireLines(exchangeId, lines, 500, false);
    useAssistantWireLogStore.getState().finalizeCapture(exchangeId, "complete");

    const fetchSpy = vi.fn();
    vi.stubGlobal("fetch", fetchSpy);
    const pushState = vi.spyOn(window.history, "pushState");
    const replaceState = vi.spyOn(window.history, "replaceState");
    const contextBefore = useAssistantContextStore.getState();
    const draftBefore = useAssistantDraftStore.getState();
    const wireBefore = useAssistantWireLogStore.getState().entries;
    const queryClient = new QueryClient({
      defaultOptions: {
        queries: { retry: false },
        mutations: { retry: false },
      },
    });
    const queryActivity = vi.fn();
    const unsubscribeQuery = queryClient
      .getQueryCache()
      .subscribe(queryActivity);

    render(
      <QueryClientProvider client={queryClient}>
        <TooltipProvider>
          <AssistantWireLogPanel />
        </TooltipProvider>
      </QueryClientProvider>,
    );
    fireEvent.click(screen.getByRole("button", { name: "Aevatar wire log" }));
    fireEvent.click(screen.getByText("/api/chat").closest("button")!);
    const renderedTab = screen.getByRole("tab", { name: "Rendered" });
    fireEvent.click(renderedTab);

    expect(renderedTab).toHaveAttribute("aria-selected", "true");
    expect(screen.getByText("markdown").tagName).toBe("STRONG");
    expect(
      screen.getByLabelText("Connection card not replayed"),
    ).toBeInTheDocument();
    expect(screen.getByText("Original source frame JSON")).toBeInTheDocument();
    expect(screen.getByText(/NYXID_SERVICE_NOT_CONNECTED/)).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /connect github/i }),
    ).not.toBeInTheDocument();
    expect(fetchSpy).not.toHaveBeenCalled();
    expect(queryActivity).not.toHaveBeenCalled();
    expect(pushState).not.toHaveBeenCalled();
    expect(replaceState).not.toHaveBeenCalled();
    expect(useAssistantContextStore.getState()).toBe(contextBefore);
    expect(useAssistantDraftStore.getState()).toBe(draftBefore);
    expect(useAssistantWireLogStore.getState().entries).toBe(wireBefore);
    unsubscribeQuery();
  });
});
