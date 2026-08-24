import { beforeEach, describe, expect, it, vi } from "vitest";
import { isTelemetryActive } from "@/lib/telemetry";
import { assistantHttp } from "@/lib/assistant/assistant-http";
import { ApiError } from "@/lib/api-client";
import { useAssistantWireLogStore } from "@/stores/assistant-wire-log-store";
import { useAuthStore } from "@/stores/auth-store";

vi.mock("@/lib/telemetry", async (importOriginal) => {
  const original = await importOriginal<typeof import("@/lib/telemetry")>();
  return { ...original, isTelemetryActive: vi.fn(() => false) };
});

const ACTOR_ID = "nyxid-chat-f8369965a444433f92ec50e67ad8ee52";

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

    await assistantHttp("/assistant/conversations", {
      preserveSessionOn401: true,
    });

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
      assistantHttp("/assistant/chat", { preserveSessionOn401: true }),
    ).rejects.toMatchObject({ status: 401, errorCode: -1 });
    expect(setUser).not.toHaveBeenCalled();
  });

  it("never clears an unattributed 401 through the compatibility option", async () => {
    const setUser = vi.fn();
    useAuthStore.setState({ setUser });
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        new Response(JSON.stringify({ error: "upstream_auth", message: "No" }), {
          status: 401,
          headers: { "Content-Type": "application/json" },
        }),
      ),
    );

    await expect(
      assistantHttp("/assistant/chat", { preserveSessionOn401: false }),
    ).rejects.toBeInstanceOf(ApiError);
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
        assistantHttp("/assistant/chat", { preserveSessionOn401: true }),
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
});
