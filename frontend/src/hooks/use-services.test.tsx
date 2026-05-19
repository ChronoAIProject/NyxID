import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { PropsWithChildren } from "react";
import { useTestSshConnection, useUpdateSshAuthMode } from "./use-services";

const { mockPatch, mockPost } = vi.hoisted(() => ({
  mockPatch: vi.fn(),
  mockPost: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: {
    patch: mockPatch,
    post: mockPost,
  },
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

describe("SSH service hooks", () => {
  beforeEach(() => {
    mockPatch.mockReset();
    mockPost.mockReset();
  });

  it("patches the user-service SSH auth mode", async () => {
    mockPatch.mockResolvedValue(undefined);

    const { result } = renderHook(() => useUpdateSshAuthMode(), {
      wrapper: createWrapper(),
    });

    await result.current.mutateAsync({
      userServiceId: "usvc-1",
      mode: "node_key",
    });

    expect(mockPatch).toHaveBeenCalledWith(
      "/user-services/usvc-1/ssh-auth-mode",
      { mode: "node_key" },
    );
  });

  it("uses the node-key exec path for Test connection", async () => {
    mockPost.mockResolvedValue({ exit_code: 0 });

    const { result } = renderHook(() => useTestSshConnection(), {
      wrapper: createWrapper(),
    });

    await result.current.mutateAsync({
      serviceId: "svc-routeros",
      principal: "nyxid-ro",
    });

    expect(mockPost).toHaveBeenCalledWith("/ssh/svc-routeros/exec", {
      principal: "nyxid-ro",
      command: "true",
      timeout_secs: 10,
    });
  });
});
