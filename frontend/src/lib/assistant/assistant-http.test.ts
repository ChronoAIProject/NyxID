import { beforeEach, describe, expect, it, vi } from "vitest";
import { isTelemetryActive } from "@/lib/telemetry";
import {
  assignAssistantResponseConversation,
  assistantHttp,
} from "@/lib/assistant/assistant-http";
import { ApiError } from "@/lib/api-client";
import { useAssistantWireLogStore } from "@/stores/assistant-wire-log-store";
import { useAuthStore } from "@/stores/auth-store";

vi.mock("@/lib/telemetry", async (importOriginal) => {
  const original = await importOriginal<typeof import("@/lib/telemetry")>();
  return { ...original, isTelemetryActive: vi.fn(() => false) };
});

const ACTOR_ID = "nyxid-chat-f8369965a444433f92ec50e67ad8ee52";
const WIRE_LOG_ID = "d7dbbf38-a31c-4331-8ddb-13fda5a70d12";

describe("assistantHttp", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    globalThis.__nyxidAssistantHttpMock = undefined;
    vi.mocked(isTelemetryActive).mockReturnValue(false);
    useAssistantWireLogStore.setState({
      featureEnabled: false,
      captureEnabled: false,
      showResponses: true,
      entries: [],
      totalBytes: 0,
      captureBytes: 0,
    });
  });

  it("uses cookie auth and conditionally identifies the UI client", async () => {
    vi.mocked(isTelemetryActive).mockReturnValue(true);
    const fetchMock = vi.fn().mockResolvedValue(new Response(null, { status: 204 }));
    vi.stubGlobal("fetch", fetchMock);

    await assistantHttp("/assistant/conversations");

    expect(fetchMock).toHaveBeenCalledWith(
      "/api/v1/assistant/conversations",
      expect.objectContaining({ credentials: "include" }),
    );
    expect(fetchMock.mock.calls[0]?.[1]?.headers).toMatchObject({
      "X-NyxID-Client": "ui",
    });
  });

  it("preserves the session for an unattributed upstream 401", async () => {
    const setUser = vi.fn();
    useAuthStore.setState({ setUser });
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        new Response(JSON.stringify({ code: "UNAUTHORIZED", message: "No" }), {
          status: 401,
          headers: { "Content-Type": "application/json" },
        }),
      ),
    );

    await expect(
      assistantHttp("/assistant/chat"),
    ).rejects.toMatchObject({ status: 401, errorCode: -1 });
    expect(setUser).not.toHaveBeenCalled();
  });

  it.each([1001, 2000, 2001, 2002])(
    "clears the session for attributed NyxID auth code %s",
    async (errorCode) => {
      const setUser = vi.fn();
      useAuthStore.setState({ setUser });
      vi.stubGlobal(
        "fetch",
        vi.fn().mockResolvedValue(
          new Response(
            JSON.stringify({
              error: "session_dead",
              error_code: errorCode,
              message: "Sign in again",
            }),
            { status: 401, headers: { "Content-Type": "application/json" } },
          ),
        ),
      );

      await expect(
        assistantHttp("/assistant/chat"),
      ).rejects.toBeInstanceOf(ApiError);
      expect(setUser).toHaveBeenCalledWith(null);
    },
  );

  it("adds debug capture only for non-list assistant requests", async () => {
    useAssistantWireLogStore.getState().setFeatureEnabled(true);
    useAssistantWireLogStore.getState().setCaptureEnabled(true);
    const fetchMock = vi.fn().mockResolvedValue(new Response(null, { status: 204 }));
    vi.stubGlobal("fetch", fetchMock);

    await assistantHttp("/assistant/conversations");
    await assistantHttp(`/assistant/conversations/${ACTOR_ID}/state`);

    expect(fetchMock.mock.calls[0]?.[1]?.headers).not.toHaveProperty(
      "X-NyxID-Debug-Upstream",
    );
    expect(fetchMock.mock.calls[1]?.[1]?.headers).toMatchObject({
      "X-NyxID-Debug-Upstream": "1",
    });
  });

  it("captures an id-backed response and assigns its adopted conversation", async () => {
    useAssistantWireLogStore.getState().setFeatureEnabled(true);
    useAssistantWireLogStore.getState().setCaptureEnabled(true);
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        new Response("data: {\"type\":\"RUN_FINISHED\"}\n\n", {
          status: 200,
          headers: {
            "Content-Type": "text/event-stream",
            "X-NyxID-Debug-Upstream-Id": WIRE_LOG_ID,
          },
        }),
      ),
    );

    const response = await assistantHttp("/assistant/chat", { method: "POST" });
    assignAssistantResponseConversation(response, ACTOR_ID);

    await vi.waitFor(() => {
      expect(useAssistantWireLogStore.getState().entries[0]).toMatchObject({
        wireLogId: WIRE_LOG_ID,
        conversationId: ACTOR_ID,
        label: "POST /assistant/chat",
        capture: {
          state: "settled",
          outcome: "complete",
          body: { truncated: false },
        },
      });
    });
  });

  it("never captures list or wire-log retrieval responses recursively", async () => {
    useAssistantWireLogStore.getState().setFeatureEnabled(true);
    useAssistantWireLogStore.getState().setCaptureEnabled(true);
    const fetchMock = vi.fn().mockResolvedValue(
      new Response("{}", {
        status: 200,
        headers: { "X-NyxID-Debug-Upstream-Id": WIRE_LOG_ID },
      }),
    );
    vi.stubGlobal("fetch", fetchMock);

    await assistantHttp("/assistant/conversations");
    await assistantHttp(`/assistant/wire-logs/${WIRE_LOG_ID}`);

    for (const call of fetchMock.mock.calls) {
      expect(call[1]?.headers).not.toHaveProperty("X-NyxID-Debug-Upstream");
    }
    expect(useAssistantWireLogStore.getState().entries).toEqual([]);
  });
});
