import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  useAdminFeatureFlags,
  useClearAdminFeatureFlag,
  useSetAdminFeatureFlag,
} from "./use-admin-feature-flags";

const { mockGet, mockPut, mockDelete } = vi.hoisted(() => ({
  mockGet: vi.fn(),
  mockPut: vi.fn(),
  mockDelete: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: { get: mockGet, put: mockPut, delete: mockDelete },
}));

function createWrapper() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return function Wrapper({ children }: PropsWithChildren) {
    return (
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    );
  };
}

beforeEach(() => vi.clearAllMocks());

describe("admin feature-flag hooks", () => {
  it("lists platform feature flags", async () => {
    mockGet.mockResolvedValue({ flags: [] });
    const { result } = renderHook(() => useAdminFeatureFlags(), {
      wrapper: createWrapper(),
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/admin/feature-flags");
  });

  it("sets an encoded flag key", async () => {
    mockPut.mockResolvedValue({});
    const { result } = renderHook(() => useSetAdminFeatureFlag(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({
      flagKey: "experimental:ai-assistant",
      body: { target_kind: "global", target_key: null, enabled: true },
    });
    expect(mockPut).toHaveBeenCalledWith(
      "/admin/feature-flags/experimental%3Aai-assistant",
      { target_kind: "global", target_key: null, enabled: true },
    );
  });

  it("clears a user override with query parameters", async () => {
    mockDelete.mockResolvedValue(undefined);
    const { result } = renderHook(() => useClearAdminFeatureFlag(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({
      flagKey: "example_ui",
      targetKind: "user",
      targetKey: "user-1",
    });
    expect(mockDelete).toHaveBeenCalledWith(
      "/admin/feature-flags/example_ui?target_kind=user&target_key=user-1",
    );
  });
});
