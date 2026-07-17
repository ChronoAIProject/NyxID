import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { PropsWithChildren } from "react";
import {
  useClearOrgFeatureFlag,
  useOrgFeatureFlags,
  useSetOrgFeatureFlag,
} from "./use-org-feature-flags";

const { mockDelete, mockGet, mockPut } = vi.hoisted(() => ({
  mockDelete: vi.fn(),
  mockGet: vi.fn(),
  mockPut: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: { delete: mockDelete, get: mockGet, put: mockPut },
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

describe("useOrgFeatureFlags", () => {
  it("GETs the org feature-flags collection and gates on orgId", async () => {
    mockGet.mockResolvedValue({ flags: [] });

    const idle = renderHook(() => useOrgFeatureFlags(""), {
      wrapper: createWrapper(),
    });
    expect(idle.result.current.fetchStatus).toBe("idle");
    expect(mockGet).not.toHaveBeenCalled();

    const active = renderHook(() => useOrgFeatureFlags("org-1"), {
      wrapper: createWrapper(),
    });
    await waitFor(() => expect(active.result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/orgs/org-1/feature-flags");
  });
});

describe("feature-flag mutations", () => {
  it("useSetOrgFeatureFlag PUTs the override to the flag path", async () => {
    mockPut.mockResolvedValue({});
    const { result } = renderHook(() => useSetOrgFeatureFlag("org-1"), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({
      flagKey: "example_ui",
      body: { target_kind: "role", target_value: "admin", enabled: true },
    });
    expect(mockPut).toHaveBeenCalledWith("/orgs/org-1/feature-flags/example_ui", {
      target_kind: "role",
      target_value: "admin",
      enabled: true,
    });
  });

  it("useClearOrgFeatureFlag DELETEs with target query params", async () => {
    mockDelete.mockResolvedValue(undefined);
    const { result } = renderHook(() => useClearOrgFeatureFlag("org-1"), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({
      flagKey: "example_ui",
      targetKind: "role",
      targetValue: "admin",
    });
    expect(mockDelete).toHaveBeenCalledWith(
      "/orgs/org-1/feature-flags/example_ui?target_kind=role&target_value=admin",
    );
  });

  it("omits target_value from the DELETE query for org scope", async () => {
    mockDelete.mockResolvedValue(undefined);
    const { result } = renderHook(() => useClearOrgFeatureFlag("org-1"), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({
      flagKey: "example_ui",
      targetKind: "org",
    });
    expect(mockDelete).toHaveBeenCalledWith(
      "/orgs/org-1/feature-flags/example_ui?target_kind=org",
    );
  });

  it("invalidates the /users/me query so personal-surface gates update live", async () => {
    // Org grants feed personal resolution (sidebar, /assistant guard); a
    // toggle that skips this invalidation regresses to hard-reload-only.
    mockPut.mockResolvedValue({});
    const queryClient = new QueryClient({
      defaultOptions: { mutations: { retry: false }, queries: { retry: false } },
    });
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    function Wrapper({ children }: PropsWithChildren) {
      return (
        <QueryClientProvider client={queryClient}>
          {children}
        </QueryClientProvider>
      );
    }
    const { result } = renderHook(() => useSetOrgFeatureFlag("org-1"), {
      wrapper: Wrapper,
    });
    await result.current.mutateAsync({
      flagKey: "example_ui",
      body: { target_kind: "org", target_value: null, enabled: true },
    });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: ["user"] });
  });
});
