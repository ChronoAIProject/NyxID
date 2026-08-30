import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { PLATFORM_OPERATION_DISCOVERY_QUERY_KEY } from "@/schemas/platform-ops";
import {
  useAgentBindings,
  useCreateBinding,
  useDeleteBinding,
} from "./use-agent-bindings";

const { mockGet, mockPost, mockDelete } = vi.hoisted(() => ({
  mockGet: vi.fn(),
  mockPost: vi.fn(),
  mockDelete: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: { get: mockGet, post: mockPost, delete: mockDelete },
}));

function harness() {
  const queryClient = new QueryClient({
    defaultOptions: {
      mutations: { retry: false },
      queries: { retry: false },
    },
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
});

describe("useAgentBindings", () => {
  it("unwraps the `bindings` array and stays idle for an empty keyId", async () => {
    mockGet.mockResolvedValue({ bindings: [{ id: "b1" }] });

    const { Wrapper } = harness();
    const idle = renderHook(() => useAgentBindings(""), {
      wrapper: Wrapper,
    });
    expect(idle.result.current.fetchStatus).toBe("idle");
    expect(mockGet).not.toHaveBeenCalled();

    const active = renderHook(() => useAgentBindings("k1"), {
      wrapper: Wrapper,
    });
    await waitFor(() => expect(active.result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/api-keys/k1/bindings");
    expect(active.result.current.data).toEqual([{ id: "b1" }]);
  });
});

describe("useCreateBinding", () => {
  it("POSTs to the key's bindings endpoint with keyId stripped from the body", async () => {
    mockPost.mockResolvedValue({ id: "b1" });
    const { queryClient, Wrapper } = harness();
    const invalidateQueries = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useCreateBinding(), {
      wrapper: Wrapper,
    });
    await result.current.mutateAsync({
      keyId: "k1",
      user_service_id: "svc-1",
      user_api_key_id: "uak-1",
    });
    expect(mockPost).toHaveBeenCalledWith("/api-keys/k1/bindings", {
      user_service_id: "svc-1",
      user_api_key_id: "uak-1",
    });
    expect(invalidateQueries).toHaveBeenCalledWith({
      queryKey: PLATFORM_OPERATION_DISCOVERY_QUERY_KEY,
    });
  });
});

describe("useDeleteBinding", () => {
  it("DELETEs the specific binding under the key", async () => {
    mockDelete.mockResolvedValue(undefined);
    const { queryClient, Wrapper } = harness();
    const invalidateQueries = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useDeleteBinding(), {
      wrapper: Wrapper,
    });
    await result.current.mutateAsync({ keyId: "k1", bindingId: "b1" });
    expect(mockDelete).toHaveBeenCalledWith("/api-keys/k1/bindings/b1");
    expect(invalidateQueries).toHaveBeenCalledWith({
      queryKey: PLATFORM_OPERATION_DISCOVERY_QUERY_KEY,
    });
  });
});
