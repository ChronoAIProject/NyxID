import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { TooltipProvider } from "@/components/ui/tooltip";
import { useAssistantContextStore } from "@/stores/assistant-context-store";
import { useAssistantDraftStore } from "@/stores/assistant-draft-store";
import { useAssistantWireLogStore } from "@/stores/assistant-wire-log-store";
import { useAuthStore } from "@/stores/auth-store";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import type { AssistantUpstreamEnvelope } from "@/schemas/assistant-wire-log";
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

const ACTIVE_CONVERSATION_ID = "nyxchat-panel";
const WIRE_LOG_ID = "d7dbbf38-a31c-4331-8ddb-13fda5a70d12";

/**
 * The wire-log gate is the `experimental:aevatar-chat-wire-log` runtime
 * feature flag, resolved server-side and delivered on `/users/me` as
 * `capabilities.enabled_features`. `undefined` models an older backend that
 * omits the field entirely.
 */
function signIn(enabledFeatures?: readonly string[]) {
  useAuthStore.setState({
    user:
      enabledFeatures === undefined
        ? admin
        : { ...admin, capabilities: { enabled_features: enabledFeatures } },
  });
}

function renderWithTooltips(node: React.ReactNode) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <TooltipProvider>{node}</TooltipProvider>
    </QueryClientProvider>,
  );
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

function recordPanelExchange(
  envelopes: readonly AssistantUpstreamEnvelope[],
  kind: "sse" | "header" = "sse",
  status = 200,
  conversationId: string | null = ACTIVE_CONVERSATION_ID,
  label = "POST /assistant/chat",
) {
  return useAssistantWireLogStore.getState().recordExchange({
    kind,
    status,
    conversationId,
    wireLogId: null,
    label,
    envelopes,
  });
}

function recordLazyExchange({
  conversationId = ACTIVE_CONVERSATION_ID,
  label = "POST /assistant/chat",
  wireLogId = WIRE_LOG_ID,
}: {
  readonly conversationId?: string | null;
  readonly label?: string;
  readonly wireLogId?: string;
} = {}) {
  return useAssistantWireLogStore.getState().recordExchange({
    kind: "header",
    status: 200,
    conversationId,
    wireLogId,
    label,
  });
}

function wireLogRecord() {
  return {
    id: WIRE_LOG_ID,
    conversation_id: ACTIVE_CONVERSATION_ID,
    created_at: "2026-08-20T12:00:00Z",
    payload: {
      version: 2 as const,
      echoes: [responseEnvelope()],
      droppedEchoCount: 0,
    },
  };
}

describe("AssistantWireLogPanel", () => {
  beforeEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    localStorage.clear();
    signIn([]);
    useAssistantWireLogStore.setState({
      featureEnabled: true,
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
    recordPanelExchange([
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
    ]);
    const fetchSpy = vi.fn();
    vi.stubGlobal("fetch", fetchSpy);
    renderWithTooltips(
      <AssistantWireLogPanel activeConversationId={ACTIVE_CONVERSATION_ID} />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Aevatar wire log" }));
    expect(screen.getByText("POST /assistant/chat")).toBeInTheDocument();
    fireEvent.click(
      screen.getByRole("button", {
        name: "Toggle POST /assistant/chat details",
      }),
    );
    expect(screen.getByText(/inspect this payload/)).toBeInTheDocument();
    expect(screen.getByText(/"path": "\/api\/chat"/)).toBeInTheDocument();
    expect(fetchSpy).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: /clear/i }));
    expect(screen.getByText("No captured requests")).toBeInTheDocument();
    expect(useAssistantWireLogStore.getState().entries).toHaveLength(0);
  });

  it("shows only the active conversation until all conversations is enabled", () => {
    recordPanelExchange(
      [responseEnvelope()],
      "sse",
      200,
      ACTIVE_CONVERSATION_ID,
      "POST /assistant/chat/active",
    );
    recordPanelExchange(
      [responseEnvelope()],
      "header",
      204,
      "nyxchat-other",
      "GET /assistant/conversations/other",
    );
    recordPanelExchange(
      [responseEnvelope()],
      "header",
      200,
      null,
      "POST /assistant/completions",
    );

    renderWithTooltips(
      <AssistantWireLogPanel activeConversationId={ACTIVE_CONVERSATION_ID} />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Aevatar wire log" }));

    expect(screen.getByText("POST /assistant/chat/active")).toBeInTheDocument();
    expect(
      screen.queryByText("GET /assistant/conversations/other"),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByText("POST /assistant/completions"),
    ).not.toBeInTheDocument();
    expect(
      screen.getByRole("switch", { name: "Show all conversations" }),
    ).not.toBeChecked();

    fireEvent.click(
      screen.getByRole("switch", { name: "Show all conversations" }),
    );

    expect(
      screen.getByText("GET /assistant/conversations/other"),
    ).toBeInTheDocument();
    expect(screen.getByText("POST /assistant/completions")).toBeInTheDocument();
  });

  it("fetches an id-backed payload only after its metadata row expands", async () => {
    recordLazyExchange();
    const fetchSpy = vi.fn().mockResolvedValue(
      new Response(JSON.stringify(wireLogRecord()), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      }),
    );
    vi.stubGlobal("fetch", fetchSpy);
    renderWithTooltips(
      <AssistantWireLogPanel activeConversationId={ACTIVE_CONVERSATION_ID} />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Aevatar wire log" }));
    expect(screen.getByText("POST /assistant/chat")).toBeInTheDocument();
    expect(screen.getByText("Header")).toBeInTheDocument();
    expect(screen.getByText("NyxID 200")).toBeInTheDocument();
    expect(fetchSpy).not.toHaveBeenCalled();

    fireEvent.click(
      screen.getByRole("button", {
        name: "Toggle POST /assistant/chat details",
      }),
    );

    expect(screen.getByText("Loading wire log...")).toBeInTheDocument();
    expect(await screen.findByText(/inspect this payload/)).toBeInTheDocument();
    expect(fetchSpy).toHaveBeenCalledOnce();
  });

  it("renders an expired state for an unavailable id-backed payload", async () => {
    recordLazyExchange();
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        new Response(
          JSON.stringify({
            error: "not_found",
            error_code: 1004,
            message: "Wire log not found.",
          }),
          {
            status: 404,
            headers: { "Content-Type": "application/json" },
          },
        ),
      ),
    );
    renderWithTooltips(
      <AssistantWireLogPanel activeConversationId={ACTIVE_CONVERSATION_ID} />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Aevatar wire log" }));
    fireEvent.click(
      screen.getByRole("button", {
        name: "Toggle POST /assistant/chat details",
      }),
    );

    expect(
      await screen.findByText("Wire log expired or unavailable."),
    ).toBeInTheDocument();
  });

  it("renders a generic error for non-404 wire-log failures", async () => {
    recordLazyExchange();
    vi.stubGlobal(
      "fetch",
      vi.fn().mockRejectedValue(new TypeError("network unavailable")),
    );
    renderWithTooltips(
      <AssistantWireLogPanel activeConversationId={ACTIVE_CONVERSATION_ID} />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Aevatar wire log" }));
    fireEvent.click(
      screen.getByRole("button", {
        name: "Toggle POST /assistant/chat details",
      }),
    );

    expect(
      await screen.findByText("Could not load wire log."),
    ).toBeInTheDocument();
  });

  it("hides the action for everyone while the operator flag is off", async () => {
    signIn([]);
    renderWithTooltips(
      <AssistantWireLogAction activeConversationId={ACTIVE_CONVERSATION_ID} />,
    );

    expect(
      screen.queryByRole("button", { name: "Aevatar wire log" }),
    ).not.toBeInTheDocument();
    await waitFor(() => {
      expect(useAssistantWireLogStore.getState().featureEnabled).toBe(false);
    });
  });

  it("closes the transport gate when a stale persisted capture meets a disabled flag", async () => {
    // A browser that captured before the flag was turned off keeps
    // `captureEnabled` in localStorage — it is persisted, `featureEnabled` is
    // not. Mounting the action must re-derive the feature gate from the
    // server-resolved flag, leaving the composed transport gate
    // (`featureEnabled && captureEnabled`) closed.
    signIn([]);
    useAssistantWireLogStore.setState({
      featureEnabled: true,
      captureEnabled: true,
    });

    renderWithTooltips(
      <AssistantWireLogAction activeConversationId={ACTIVE_CONVERSATION_ID} />,
    );

    await waitFor(() => {
      expect(useAssistantWireLogStore.getState().featureEnabled).toBe(false);
    });
    const { featureEnabled, captureEnabled } =
      useAssistantWireLogStore.getState();
    expect(captureEnabled).toBe(true);
    expect(featureEnabled && captureEnabled).toBe(false);
  });

  it("treats an omitted capability set as disabled", async () => {
    signIn(undefined);
    useAssistantWireLogStore.setState({ featureEnabled: true });

    renderWithTooltips(
      <AssistantWireLogAction activeConversationId={ACTIVE_CONVERSATION_ID} />,
    );

    expect(
      screen.queryByRole("button", { name: "Aevatar wire log" }),
    ).not.toBeInTheDocument();
    await waitFor(() => {
      expect(useAssistantWireLogStore.getState().featureEnabled).toBe(false);
    });
  });

  it("shows the action for authenticated users regardless of role when the flag is on", async () => {
    signIn([FEATURE_FLAG.AEVATAR_CHAT_WIRE_LOG]);
    const { unmount } = renderWithTooltips(
      <AssistantWireLogAction activeConversationId={ACTIVE_CONVERSATION_ID} />,
    );

    expect(
      await screen.findByRole("button", { name: "Aevatar wire log" }),
    ).toBeInTheDocument();
    await waitFor(() => {
      expect(useAssistantWireLogStore.getState().featureEnabled).toBe(true);
    });

    unmount();
    useAuthStore.setState({
      user: {
        ...admin,
        is_admin: false,
        role: "user",
        capabilities: {
          enabled_features: [FEATURE_FLAG.AEVATAR_CHAT_WIRE_LOG],
        },
      },
    });
    renderWithTooltips(
      <AssistantWireLogAction activeConversationId={ACTIVE_CONVERSATION_ID} />,
    );

    expect(
      await screen.findByRole("button", { name: "Aevatar wire log" }),
    ).toBeInTheDocument();
  });

  it("gates every response-derived surface with the top Responses switch", () => {
    const exchangeId = recordPanelExchange([responseEnvelope()]);
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
    renderWithTooltips(
      <AssistantWireLogPanel activeConversationId={ACTIVE_CONVERSATION_ID} />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Aevatar wire log" }));
    expect(
      screen.getByRole("switch", { name: "Show Aevatar responses" }),
    ).toBeChecked();
    expect(screen.queryByText("Aevatar 202")).not.toBeInTheDocument();
    fireEvent.click(
      screen.getByRole("button", {
        name: "Toggle POST /assistant/chat details",
      }),
    );
    expect(screen.getByText("Aevatar 202")).toBeInTheDocument();
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
    const exchangeId = recordPanelExchange([responseEnvelope()]);
    if (!exchangeId) throw new Error("expected exchange");
    const lines = Array.from({ length: 450 }, (_, index) => ({
      text: `line-${String(index)}`,
      ending: "\n" as const,
    }));
    useAssistantWireLogStore
      .getState()
      .attachWireLines(exchangeId, lines, 3_590, false);
    useAssistantWireLogStore.getState().finalizeCapture(exchangeId, "complete");
    renderWithTooltips(
      <AssistantWireLogPanel activeConversationId={ACTIVE_CONVERSATION_ID} />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Aevatar wire log" }));
    fireEvent.click(
      screen.getByRole("button", {
        name: "Toggle POST /assistant/chat details",
      }),
    );
    expect(screen.getByText("line-199")).toBeInTheDocument();
    expect(screen.queryByText("line-200")).not.toBeInTheDocument();
    expect(screen.queryByText("line-449")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Show 200 more" }));
    expect(screen.getByText("line-200")).toBeInTheDocument();
    expect(screen.getByText("line-399")).toBeInTheDocument();
    expect(screen.queryByText("line-400")).not.toBeInTheDocument();
  });

  it("discloses when response headers were dropped by degradation", () => {
    const degradedResponse = responseEnvelope();
    recordPanelExchange(
      [
        {
          ...degradedResponse,
          droppedHeaders: true,
          response: { ...degradedResponse.response, headers: {} },
        },
      ],
      "header",
    );
    renderWithTooltips(
      <AssistantWireLogPanel activeConversationId={ACTIVE_CONVERSATION_ID} />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Aevatar wire log" }));
    fireEvent.click(
      screen.getByRole("button", {
        name: "Toggle POST /assistant/chat details",
      }),
    );

    expect(
      screen.getByText(
        "Allowlisted response headers were dropped by the wire-header size ladder.",
      ),
    ).toBeInTheDocument();
  });

  it("renders text and inert original-frame placeholders without side effects", () => {
    const exchangeId = recordPanelExchange([responseEnvelope()]);
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
          <AssistantWireLogPanel
            activeConversationId={ACTIVE_CONVERSATION_ID}
          />
        </TooltipProvider>
      </QueryClientProvider>,
    );
    fireEvent.click(screen.getByRole("button", { name: "Aevatar wire log" }));
    fireEvent.click(
      screen.getByRole("button", {
        name: "Toggle POST /assistant/chat details",
      }),
    );
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
