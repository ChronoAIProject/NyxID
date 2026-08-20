import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useAssistantWireLog } from "./use-assistant-wire-log";

const WIRE_LOG_ID = "d7dbbf38-a31c-4331-8ddb-13fda5a70d12";

function wireLogRecord() {
  return {
    id: WIRE_LOG_ID,
    conversation_id: "nyxchat-hook-test",
    created_at: "2026-08-20T12:00:00Z",
    payload: {
      version: 2 as const,
      echoes: [
        {
          degraded: true as const,
          method: "POST",
          path: "api/chat",
          commandType: "text",
          upstreamOutcome: "response" as const,
          status: 200,
        },
      ],
      droppedEchoCount: 0,
    },
  };
}

function createHarness() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const Wrapper = ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );
  return { queryClient, Wrapper };
}

beforeEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("useAssistantWireLog", () => {
  it("stays idle when no wire-log id is available", () => {
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);
    const { Wrapper } = createHarness();
    const hook = renderHook(() => useAssistantWireLog(null, true), {
      wrapper: Wrapper,
    });

    expect(hook.result.current.fetchStatus).toBe("idle");
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("stays idle until enabled and then loads a schema-validated record", async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(JSON.stringify(wireLogRecord()), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      }),
    );
    vi.stubGlobal("fetch", fetchMock);
    const { Wrapper } = createHarness();
    const hook = renderHook(
      ({ enabled }) => useAssistantWireLog(WIRE_LOG_ID, enabled),
      { initialProps: { enabled: false }, wrapper: Wrapper },
    );

    expect(hook.result.current.fetchStatus).toBe("idle");
    expect(fetchMock).not.toHaveBeenCalled();

    hook.rerender({ enabled: true });
    await waitFor(() => expect(hook.result.current.isSuccess).toBe(true));

    expect(hook.result.current.data).toEqual({
      status: "loaded",
      record: wireLogRecord(),
    });
    expect(fetchMock).toHaveBeenCalledOnce();
    expect(fetchMock.mock.calls[0]?.[0]).toBe(
      `/api/v1/assistant/wire-logs/${WIRE_LOG_ID}`,
    );
    const init = fetchMock.mock.calls[0]?.[1] as RequestInit;
    expect(init.headers).not.toHaveProperty("X-NyxID-Debug-Upstream");
  });

  it("reuses the immutable query cache after collapse and re-expansion", async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(JSON.stringify(wireLogRecord()), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      }),
    );
    vi.stubGlobal("fetch", fetchMock);
    const { Wrapper } = createHarness();
    const first = renderHook(() => useAssistantWireLog(WIRE_LOG_ID, true), {
      wrapper: Wrapper,
    });
    await waitFor(() => expect(first.result.current.isSuccess).toBe(true));
    first.unmount();

    const second = renderHook(() => useAssistantWireLog(WIRE_LOG_ID, true), {
      wrapper: Wrapper,
    });
    await waitFor(() => expect(second.result.current.isSuccess).toBe(true));

    expect(fetchMock).toHaveBeenCalledOnce();
  });

  it("returns an expired result for 404 without retrying or throwing", async () => {
    const fetchMock = vi.fn().mockResolvedValue(
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
    );
    vi.stubGlobal("fetch", fetchMock);
    const { Wrapper } = createHarness();
    const hook = renderHook(() => useAssistantWireLog(WIRE_LOG_ID, true), {
      wrapper: Wrapper,
    });

    await waitFor(() => expect(hook.result.current.isSuccess).toBe(true));

    expect(hook.result.current.data).toEqual({ status: "expired" });
    expect(hook.result.current.isError).toBe(false);
    expect(fetchMock).toHaveBeenCalledOnce();
  });
});
