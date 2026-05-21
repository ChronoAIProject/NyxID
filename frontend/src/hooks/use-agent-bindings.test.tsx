import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
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

function wrapperFactory() {
  const queryClient = new QueryClient({
    defaultOptions: {
      mutations: { retry: false },
      queries: { retry: false },
    },
  });
  return ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe("useAgentBindings", () => {
  it("unwraps the `bindings` array and stays idle for an empty keyId", async () => {
    mockGet.mockResolvedValue({ bindings: [{ id: "b1" }] });

    const idle = renderHook(() => useAgentBindings(""), {
      wrapper: wrapperFactory(),
    });
    expect(idle.result.current.fetchStatus).toBe("idle");
    expect(mockGet).not.toHaveBeenCalled();

    const active = renderHook(() => useAgentBindings("k1"), {
      wrapper: wrapperFactory(),
    });
    await waitFor(() => expect(active.result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/api-keys/k1/bindings");
    expect(active.result.current.data).toEqual([{ id: "b1" }]);
  });
});

describe("useCreateBinding", () => {
  it("POSTs to the key's bindings endpoint with keyId stripped from the body", async () => {
    mockPost.mockResolvedValue({ id: "b1" });
    const { result } = renderHook(() => useCreateBinding(), {
      wrapper: wrapperFactory(),
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
  });
});

describe("useDeleteBinding", () => {
  it("DELETEs the specific binding under the key", async () => {
    mockDelete.mockResolvedValue(undefined);
    const { result } = renderHook(() => useDeleteBinding(), {
      wrapper: wrapperFactory(),
    });
    await result.current.mutateAsync({ keyId: "k1", bindingId: "b1" });
    expect(mockDelete).toHaveBeenCalledWith("/api-keys/k1/bindings/b1");
  });
});
