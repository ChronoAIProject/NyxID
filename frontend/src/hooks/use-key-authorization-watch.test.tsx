import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useKeyAuthorizationWatch } from "./use-keys";

const { mockGet } = vi.hoisted(() => ({ mockGet: vi.fn() }));

vi.mock("@/lib/api-client", () => ({
  api: { get: mockGet, post: vi.fn(), put: vi.fn(), delete: vi.fn() },
}));

function harness() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  function Wrapper({ children }: PropsWithChildren) {
    return (
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    );
  }
  return { Wrapper };
}

beforeEach(() => {
  vi.clearAllMocks();
  vi.useFakeTimers({ shouldAdvanceTime: true });
});

afterEach(() => {
  vi.useRealTimers();
});

describe("useKeyAuthorizationWatch", () => {
  it("does not expose a stale failed result to a new attempt on the same key", async () => {
    mockGet
      .mockResolvedValueOnce({ id: "k1", status: "failed" })
      .mockResolvedValueOnce({ id: "k1", status: "pending_auth" })
      .mockResolvedValue({
        id: "k1",
        status: "active",
        last_authorized_at: "2026-08-06T12:00:00Z",
      });
    const { Wrapper } = harness();
    const deadlineAt = Date.now() + 60_000;
    const { result, rerender } = renderHook(
      ({ attemptId }: { readonly attemptId: string }) =>
        useKeyAuthorizationWatch("k1", {
          attemptId,
          enabled: true,
          deadlineAt,
        }),
      { wrapper: Wrapper, initialProps: { attemptId: "attempt-a" } },
    );

    await waitFor(() => expect(result.current.status).toBe("failed"));
    rerender({ attemptId: "attempt-b" });
    expect(result.current.status).toBeUndefined();
    await waitFor(() => expect(result.current.status).toBe("pending_auth"));
    expect(result.current.authorized).toBe(false);

    await vi.advanceTimersByTimeAsync(2_100);
    await waitFor(() => expect(result.current.authorized).toBe(true));
  });

  it("requires last_authorized_at to advance before reconnect is terminal", async () => {
    mockGet
      .mockResolvedValueOnce({
        id: "k1",
        status: "active",
        last_authorized_at: "2026-08-06T10:00:00Z",
      })
      .mockResolvedValue({
        id: "k1",
        status: "active",
        last_authorized_at: "2026-08-06T10:05:00Z",
      });
    const { Wrapper } = harness();
    const { result } = renderHook(
      () =>
        useKeyAuthorizationWatch("k1", {
          attemptId: "attempt-reconnect",
          previousAuthorizationAt: "2026-08-06T10:00:00Z",
          enabled: true,
          deadlineAt: Date.now() + 60_000,
        }),
      { wrapper: Wrapper },
    );

    await waitFor(() => expect(result.current.status).toBe("active"));
    expect(result.current.authorized).toBe(false);

    await vi.advanceTimersByTimeAsync(2_100);
    await waitFor(() => expect(result.current.authorized).toBe(true));
  });
});
