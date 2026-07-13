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
import type { AdminOAuthClientListParams } from "@/types/admin";

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

const defaultParams: AdminOAuthClientListParams = {
  page: 1,
  per_page: 25,
  sort: "-created_at",
};

describe("admin OAuth client hooks", () => {
  it("fetches the admin OAuth clients list", async () => {
    mockGet.mockResolvedValue({
      clients: [],
      total: 0,
      page: 1,
      per_page: 25,
      filter_options: {},
    });
    const { result } = renderHook(() => useAdminOAuthClients(defaultParams), {
      wrapper: createWrapper(),
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith(
      "/admin/oauth-clients?page=1&per_page=25&sort=-created_at",
    );
  });

  it("serializes every multi-value server-side table filter", async () => {
    mockGet.mockResolvedValue({ clients: [] });
    const params: AdminOAuthClientListParams = {
      page: 3,
      per_page: 50,
      search: "aevatar console",
      search_filters: JSON.stringify([
        { field: "client", values: ["aevatar, console", "東京"] },
        { field: "client_type", values: ["public", "confidential"] },
      ]),
      client_type: "public,confidential",
      creator_type: "dynamic_registration,system",
      broker: "enabled,scope",
      is_active: "true,false",
      scope: "openid,urn:nyxid:scope:broker_binding",
      created_from: "2026-07-01",
      created_to: "2026-07-31",
      sort: "client_name",
    };
    const { result } = renderHook(() => useAdminOAuthClients(params), {
      wrapper: createWrapper(),
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    const request = mockGet.mock.calls[0]?.[0] as string;
    const query = new URLSearchParams(request.split("?")[1]);
    expect(query.get("page")).toBe("3");
    expect(query.get("per_page")).toBe("50");
    expect(query.get("search")).toBe("aevatar console");
    expect(query.get("search_filters")).toBe(params.search_filters);
    expect(query.get("client_type")).toBe("public,confidential");
    expect(query.get("creator_type")).toBe("dynamic_registration,system");
    expect(query.get("broker")).toBe("enabled,scope");
    expect(query.get("is_active")).toBe("true,false");
    expect(query.get("scope")).toBe("openid,urn:nyxid:scope:broker_binding");
    expect(query.get("created_from")).toBe("2026-07-01");
    expect(query.get("created_to")).toBe("2026-07-31");
    expect(query.get("sort")).toBe("client_name");
  });

  it("serializes exact creation dates", async () => {
    mockGet.mockResolvedValue({ clients: [] });
    const { result } = renderHook(
      () =>
        useAdminOAuthClients({
          ...defaultParams,
          created_dates: "2026-07-03,2026-07-08",
        }),
      { wrapper: createWrapper() },
    );

    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    const request = mockGet.mock.calls[0]?.[0] as string;
    expect(
      new URLSearchParams(request.split("?")[1]).get("created_dates"),
    ).toBe("2026-07-03,2026-07-08");
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

    expect(mockPatch).toHaveBeenCalledWith("/admin/oauth-clients/client%2F1", {
      broker_capability_enabled: true,
    });
  });

  it("invalidates every cached OAuth-client page after an update", async () => {
    mockPatch.mockResolvedValue({ id: "client-1" });
    const queryClient = createQueryClient();
    const invalidateQueries = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useUpdateAdminOAuthClient(), {
      wrapper: createWrapper(queryClient),
    });

    await result.current.mutateAsync({
      clientId: "client-1",
      data: { is_active: false },
    });

    expect(invalidateQueries).toHaveBeenCalledWith({
      queryKey: ["admin", "oauth-clients"],
    });
  });

  it("skips the admin-only broker settings request when disabled", async () => {
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
    expect(mockGet).toHaveBeenCalledTimes(1);
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
