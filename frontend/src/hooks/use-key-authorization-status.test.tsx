import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useKeyAuthorizationStatus } from "./use-keys";

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
  return { queryClient, Wrapper };
}

beforeEach(() => {
  vi.clearAllMocks();
  vi.useFakeTimers({ shouldAdvanceTime: true });
});

afterEach(() => {
  vi.useRealTimers();
});

/**
 * OAuth hands the user to a provider in another tab, so nothing in this tab
 * observes the callback — the placeholder key is polled instead. Two
 * properties matter and neither is obvious from reading the hook:
 * the poll must STOP once the row is terminal (or a dialog left open hammers
 * `/keys/:id` forever), and reaching a terminal state must refresh the
 * `["keys"]` LIST, because that is what the in-chat connect card renders from
 * — otherwise the dialog says Connected while the transcript still says
 * Authorizing.
 */
describe("useKeyAuthorizationStatus", () => {
  it("polls while the key is pending_auth", async () => {
    mockGet.mockResolvedValue({ id: "k1", status: "pending_auth" });
    const { Wrapper } = harness();

    const { result } = renderHook(
      () => useKeyAuthorizationStatus("k1", true),
      { wrapper: Wrapper },
    );

    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    const afterFirst = mockGet.mock.calls.length;

    await vi.advanceTimersByTimeAsync(2_100);
    await waitFor(() =>
      expect(mockGet.mock.calls.length).toBeGreaterThan(afterFirst),
    );
  });

  it("stops polling once the key is active", async () => {
    mockGet.mockResolvedValue({ id: "k1", status: "active" });
    const { Wrapper } = harness();

    const { result } = renderHook(
      () => useKeyAuthorizationStatus("k1", true),
      { wrapper: Wrapper },
    );
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    const settled = mockGet.mock.calls.length;

    await vi.advanceTimersByTimeAsync(10_000);

    expect(mockGet.mock.calls.length).toBe(settled);
  });

  it("stops polling once the key has failed", async () => {
    mockGet.mockResolvedValue({ id: "k1", status: "failed" });
    const { Wrapper } = harness();

    const { result } = renderHook(
      () => useKeyAuthorizationStatus("k1", true),
      { wrapper: Wrapper },
    );
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    const settled = mockGet.mock.calls.length;

    await vi.advanceTimersByTimeAsync(10_000);

    expect(mockGet.mock.calls.length).toBe(settled);
  });

  it("refreshes the keys LIST when authorization completes", async () => {
    // The list is what the transcript's connect card reads. Without this the
    // card stays "Authorizing" after the dialog says "Connected".
    mockGet.mockResolvedValue({ id: "k1", status: "active" });
    const { queryClient, Wrapper } = harness();
    const invalidate = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(
      () => useKeyAuthorizationStatus("k1", true),
      { wrapper: Wrapper },
    );
    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    await waitFor(() =>
      expect(invalidate).toHaveBeenCalledWith({
        queryKey: ["keys"],
        exact: true,
      }),
    );
  });

  it("does not touch the list while still pending", async () => {
    mockGet.mockResolvedValue({ id: "k1", status: "pending_auth" });
    const { queryClient, Wrapper } = harness();
    const invalidate = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(
      () => useKeyAuthorizationStatus("k1", true),
      { wrapper: Wrapper },
    );
    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(invalidate).not.toHaveBeenCalledWith({
      queryKey: ["keys"],
      exact: true,
    });
  });

  it("issues no request when disabled or without a key id", async () => {
    const { Wrapper } = harness();

    renderHook(() => useKeyAuthorizationStatus("k1", false), {
      wrapper: Wrapper,
    });
    renderHook(() => useKeyAuthorizationStatus(null, true), {
      wrapper: Wrapper,
    });
    await vi.advanceTimersByTimeAsync(5_000);

    expect(mockGet).not.toHaveBeenCalled();
  });
});
