import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  useAdminDeleteNode,
  useAdminDisconnectNode,
  useAdminNode,
  useAdminNodes,
} from "./use-admin-nodes";

const { mockGet, mockPost, mockDelete } = vi.hoisted(() => ({
  mockGet: vi.fn(),
  mockPost: vi.fn(),
  mockDelete: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: { get: mockGet, post: mockPost, delete: mockDelete },
}));

function createWrapper() {
  const queryClient = new QueryClient({
    defaultOptions: {
      mutations: { retry: false },
      queries: { retry: false },
    },
  });
  return function Wrapper({ children }: PropsWithChildren) {
    return (
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    );
  };
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe("useAdminNodes list query", () => {
  it("builds page/per_page with defaults only", async () => {
    mockGet.mockResolvedValue({ nodes: [], total: 0 });
    const { result } = renderHook(() => useAdminNodes(3, 50), {
      wrapper: createWrapper(),
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/admin/nodes?page=3&per_page=50");
  });

  it("maps status into `status` and search into the `user_id` param", async () => {
    mockGet.mockResolvedValue({ nodes: [], total: 0 });
    const { result } = renderHook(
      () => useAdminNodes(1, 20, "online", "user-1"),
      { wrapper: createWrapper() },
    );
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith(
      "/admin/nodes?page=1&per_page=20&status=online&user_id=user-1",
    );
  });
});

describe("useAdminNode detail query", () => {
  it("fetches by id and stays idle for an empty id", async () => {
    mockGet.mockResolvedValue({ id: "node-1" });
    const idle = renderHook(() => useAdminNode(""), {
      wrapper: createWrapper(),
    });
    expect(idle.result.current.fetchStatus).toBe("idle");
    expect(mockGet).not.toHaveBeenCalled();

    const active = renderHook(() => useAdminNode("node-1"), {
      wrapper: createWrapper(),
    });
    await waitFor(() => expect(active.result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/admin/nodes/node-1");
  });
});

describe("admin node mutations", () => {
  it("useAdminDisconnectNode POSTs to the disconnect endpoint", async () => {
    mockPost.mockResolvedValue(undefined);
    const { result } = renderHook(() => useAdminDisconnectNode(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync("node-1");
    expect(mockPost).toHaveBeenCalledWith("/admin/nodes/node-1/disconnect");
  });

  it("useAdminDeleteNode DELETEs /admin/nodes/{nodeId}", async () => {
    mockDelete.mockResolvedValue(undefined);
    const { result } = renderHook(() => useAdminDeleteNode(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync("node-1");
    expect(mockDelete).toHaveBeenCalledWith("/admin/nodes/node-1");
  });
});
