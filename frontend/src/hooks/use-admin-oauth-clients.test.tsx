import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  useAdminOAuthClients,
  useBrokerSettings,
  useUpdateAdminOAuthClient,
  useUpdateBrokerSettings,
} from "./use-admin-oauth-clients";

const { mockGet, mockPatch } = vi.hoisted(() => ({
  mockGet: vi.fn(),
  mockPatch: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: { get: mockGet, patch: mockPatch },
}));

function createQueryClient() {
  return new QueryClient({
    defaultOptions: {
      mutations: { retry: false },
      queries: { retry: false },
    },
  });
}

function createWrapper(queryClient = createQueryClient()) {
  return function Wrapper({ children }: PropsWithChildren) {
    return (
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    );
  };
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe("admin OAuth client hooks", () => {
  it("fetches the admin OAuth clients list", async () => {
    mockGet.mockResolvedValue({ clients: [] });
    const { result } = renderHook(() => useAdminOAuthClients(), {
      wrapper: createWrapper(),
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/admin/oauth-clients");
  });

  it("patches an admin OAuth client by id", async () => {
    mockPatch.mockResolvedValue({ id: "client-1" });
    const { result } = renderHook(() => useUpdateAdminOAuthClient(), {
      wrapper: createWrapper(),
    });

    await result.current.mutateAsync({
      clientId: "client/1",
      data: { broker_capability_enabled: true },
    });

    expect(mockPatch).toHaveBeenCalledWith(
      "/admin/oauth-clients/client%2F1",
      { broker_capability_enabled: true },
    );
  });

  it("fetches broker settings only when enabled", async () => {
    mockGet.mockResolvedValue({
      broker_require_sender_constraint: {
        effective: false,
        env_default: false,
        override: null,
        source: "env_default",
      },
      broker_require_admin_capability: {
        effective: false,
        env_default: false,
        override: null,
        source: "env_default",
      },
    });
    const idle = renderHook(() => useBrokerSettings(false), {
      wrapper: createWrapper(),
    });
    expect(idle.result.current.fetchStatus).toBe("idle");
    expect(mockGet).not.toHaveBeenCalled();

    const active = renderHook(() => useBrokerSettings(true), {
      wrapper: createWrapper(),
    });
    await waitFor(() => expect(active.result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/admin/settings/broker");
  });

  it("patches broker settings with boolean and null overrides", async () => {
    mockPatch.mockResolvedValue({});
    const queryClient = createQueryClient();
    const invalidateQueries = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useUpdateBrokerSettings(), {
      wrapper: createWrapper(queryClient),
    });

    await result.current.mutateAsync({
      broker_require_sender_constraint: true,
      broker_require_admin_capability: null,
    });

    expect(mockPatch).toHaveBeenCalledWith("/admin/settings/broker", {
      broker_require_sender_constraint: true,
      broker_require_admin_capability: null,
    });
    expect(invalidateQueries).toHaveBeenCalledWith({
      queryKey: ["admin", "settings", "broker"],
    });
    expect(invalidateQueries).toHaveBeenCalledWith({
      queryKey: ["admin", "oauth-clients"],
    });
  });
});
