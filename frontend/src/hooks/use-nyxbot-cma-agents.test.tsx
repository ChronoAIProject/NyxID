import { act, cleanup, renderHook } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { PropsWithChildren } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";
import { useCmaEventStream } from "./use-nyxbot-cma-agents";
const mocks = vi.hoisted(() => ({ http: vi.fn() }));
vi.mock("@/lib/assistant/assistant-http", () => ({
  assistantHttp: mocks.http,
}));
let client: QueryClient;
function Wrapper({ children }: PropsWithChildren) {
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}
function mockEventStream(
  {
    every15Seconds,
    ignoreAbort = false,
  }: { every15Seconds?: Uint8Array; ignoreAbort?: boolean } = {},
) {
  const streams: {
    controller: ReadableStreamDefaultController<Uint8Array>;
    signal: AbortSignal;
  }[] = [];
  mocks.http.mockImplementation(
    (_path: string, { signal }: { signal: AbortSignal }) => {
      let interval: ReturnType<typeof setInterval> | undefined;
      return new Response(
        new ReadableStream<Uint8Array>({
          start(controller) {
            streams.push({ controller, signal });
            if (every15Seconds !== undefined)
              interval = setInterval(
                () => controller.enqueue(every15Seconds),
                15_000,
              );
            if (!ignoreAbort)
              signal.addEventListener(
                "abort",
                () => {
                  clearInterval(interval);
                  controller.error(new DOMException("Aborted", "AbortError"));
                },
                { once: true },
              );
          },
          cancel() {
            clearInterval(interval);
          },
        }),
        { headers: { "Content-Type": "text/event-stream" } },
      );
    },
  );
  return () => {
    const stream = streams[0];
    if (!stream) throw new Error("CMA event stream has not been opened");
    return stream;
  };
}
beforeEach(() => {
  vi.useFakeTimers();
  mocks.http.mockReset();
  client = new QueryClient();
  useAuthStore.setState({
    user: {
      id: "owner",
      capabilities: { enabled_features: ["assistant:cma-sandboxes"] },
    } as unknown as User,
  });
});
afterEach(() => {
  cleanup();
  client.clear();
  vi.useRealTimers();
});
it("shows a reconnectable timeout when the SSE handshake stalls", async () => {
  mocks.http.mockImplementation(
    (_path: string, { signal }: { signal: AbortSignal }) =>
      new Promise((_resolve, reject) =>
        signal.addEventListener("abort", () =>
          reject(new DOMException("Aborted", "AbortError")),
        ),
      ),
  );
  const { result } = renderHook(
    () => useCmaEventStream("local", "agt_one", undefined, true),
    { wrapper: Wrapper },
  );
  await act(async () => {
    await Promise.resolve();
  });
  expect(result.current.state).toBe("connecting");
  await act(async () => {
    await vi.advanceTimersByTimeAsync(20_000);
  });
  expect(result.current.state).toBe("disconnected");
  expect(result.current.error).toMatch(/timed out/);
});
it.each([
  { kind: "Agent", responseId: undefined, cursor: "agent-cursor" },
  { kind: "Response", responseId: "rsp_one", cursor: "41" },
])(
  "keeps the $kind stream connected on comment heartbeats without business progress",
  async ({ responseId, cursor }) => {
    const firstStream = mockEventStream({
      every15Seconds: new TextEncoder().encode(": keep-alive\n\n"),
    });
    const invalidate = vi.spyOn(client, "invalidateQueries");
    const { result } = renderHook(
      () => useCmaEventStream("local", "agt_one", responseId, true, cursor),
      { wrapper: Wrapper },
    );
    await act(async () => {
      await vi.advanceTimersByTimeAsync(75_000);
    });
    expect(result.current.state).toBe("connected");
    expect(result.current.error).toBeUndefined();
    expect(result.current.events).toEqual([]);
    expect(invalidate).not.toHaveBeenCalled();
    await act(async () => {
      result.current.reconnect();
    });
    expect(firstStream().signal.aborted).toBe(true);
    expect(mocks.http).toHaveBeenCalledTimes(2);
    expect(mocks.http.mock.lastCall?.[0]).toBe(
      `/assistant/nyxagent/cma-agents/local${responseId ? `/responses/${responseId}` : ""}/events?after_cursor=${cursor}`,
    );
  },
);
it.each([
  { kind: "silent", every15Seconds: undefined },
  { kind: "empty-chunk", every15Seconds: new Uint8Array() },
])("times out a $kind stream after 60 seconds", async ({ every15Seconds }) => {
  const firstStream = mockEventStream({ every15Seconds });
  const { result } = renderHook(
    () => useCmaEventStream("local", "agt_one", undefined, true),
    { wrapper: Wrapper },
  );
  await act(async () => {
    await vi.advanceTimersByTimeAsync(59_999);
  });
  expect(result.current.state).toBe("connected");
  await act(async () => {
    await vi.advanceTimersByTimeAsync(1);
  });
  expect(result.current.state).toBe("disconnected");
  expect(result.current.error).toMatch(/timed out/);
  expect(firstStream().signal.aborted).toBe(true);
});
it("closes at five minutes even while comment heartbeats continue", async () => {
  const firstStream = mockEventStream({
    every15Seconds: new TextEncoder().encode(": keep-alive\n\n"),
  });
  const { result } = renderHook(
    () => useCmaEventStream("local", "agt_one", undefined, true),
    { wrapper: Wrapper },
  );
  await act(async () => {
    await vi.advanceTimersByTimeAsync(5 * 60_000 - 1);
  });
  expect(result.current.state).toBe("connected");
  await act(async () => {
    await vi.advanceTimersByTimeAsync(1);
  });
  expect(result.current.state).toBe("disconnected");
  expect(result.current.error).toMatch(/timed out/);
  expect(firstStream().signal.aborted).toBe(true);
});
it.each(["unmount", "timeout"])(
  "does not rearm idle timers for late comment bytes after %s",
  async (end) => {
    // An already queued network read can settle after its connection is aborted.
    const firstStream = mockEventStream({ ignoreAbort: true });
    const { unmount } = renderHook(
      () => useCmaEventStream("local", "agt_one", undefined, true),
      { wrapper: Wrapper },
    );
    await act(async () => {
      await vi.advanceTimersByTimeAsync(end === "timeout" ? 60_000 : 0);
    });
    if (end === "unmount") unmount();
    const stream = firstStream();
    expect(stream.signal.aborted).toBe(true);
    const timers = vi.getTimerCount();
    await act(async () => {
      stream.controller.enqueue(
        new TextEncoder().encode(": keep-alive\n\n"),
      );
    });
    expect(vi.getTimerCount()).toBe(timers);
    await act(async () => {
      stream.controller.close();
    });
  },
);
