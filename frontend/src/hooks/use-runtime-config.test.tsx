import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useRuntimeConfig } from "./use-runtime-config";

const { mockGet } = vi.hoisted(() => ({
  mockGet: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: {
    get: mockGet,
  },
}));

function createWrapper() {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
    },
  });

  return function Wrapper({ children }: PropsWithChildren) {
    return (
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    );
  };
}

describe("useRuntimeConfig", () => {
  beforeEach(() => {
    mockGet.mockReset();
  });

  it("fetches and validates runtime config", async () => {
    mockGet.mockResolvedValue({
      api_base_url: "https://nyx-api.chrono-ai.fun/",
    });

    const { result } = renderHook(() => useRuntimeConfig(), {
      wrapper: createWrapper(),
    });

    await waitFor(() => {
      expect(result.current.isSuccess).toBe(true);
    });

    expect(mockGet).toHaveBeenCalledWith("/runtime-config");
    expect(result.current.data).toEqual({
      api_base_url: "https://nyx-api.chrono-ai.fun",
    });
  });
});
