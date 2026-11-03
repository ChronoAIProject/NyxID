import { act, renderHook } from "@testing-library/react";
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
