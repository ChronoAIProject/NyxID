import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  useAllAdminedApiKeys,
  useApiKey,
  useApiKeys,
  useApiKeyUsage,
  useApiKeysUsage,
  useCreateApiKey,
  useDeleteApiKey,
  useRotateApiKey,
  useUpdateApiKey,
} from "./use-api-keys";

const { mockGet, mockPost, mockPut, mockDelete, mockUseOrgs } = vi.hoisted(
  () => ({
    mockGet: vi.fn(),
    mockPost: vi.fn(),
    mockPut: vi.fn(),
    mockDelete: vi.fn(),
    mockUseOrgs: vi.fn(),
  }),
);

vi.mock("@/lib/api-client", () => ({
  api: { get: mockGet, post: mockPost, put: mockPut, delete: mockDelete },
}));

vi.mock("./use-orgs", () => ({
  useOrgs: mockUseOrgs,
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
  // Default: no orgs so the aggregate hooks only fire the personal query.
  mockUseOrgs.mockReturnValue({ data: [] });
});

describe("useApiKeys", () => {
  it("lists personal keys and unwraps the `keys` array", async () => {
    mockGet.mockResolvedValue({ keys: [{ id: "k1" }] });
    const { result } = renderHook(() => useApiKeys(), {
      wrapper: wrapperFactory(),
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/api-keys");
    expect(result.current.data).toEqual([{ id: "k1" }]);
  });

  it("appends an url-encoded org_id when listing org keys", async () => {
    mockGet.mockResolvedValue({ keys: [] });
    const { result } = renderHook(() => useApiKeys({ orgId: "org/1" }), {
      wrapper: wrapperFactory(),
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/api-keys?org_id=org%2F1");
  });
});

describe("useApiKey / useApiKeyUsage gating", () => {
  it("useApiKey fetches by id and stays idle for an empty id", async () => {
    mockGet.mockResolvedValue({ id: "k1" });
    const idle = renderHook(() => useApiKey(""), { wrapper: wrapperFactory() });
    expect(idle.result.current.fetchStatus).toBe("idle");
    expect(mockGet).not.toHaveBeenCalled();

    const active = renderHook(() => useApiKey("k1"), {
      wrapper: wrapperFactory(),
    });
    await waitFor(() => expect(active.result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/api-keys/k1");
  });

  it("useApiKeyUsage gates on keyId and passes the days query param", async () => {
    mockGet.mockResolvedValue({ api_key_id: "k1" });
    const idle = renderHook(() => useApiKeyUsage(""), {
      wrapper: wrapperFactory(),
    });
    expect(idle.result.current.fetchStatus).toBe("idle");
    expect(mockGet).not.toHaveBeenCalled();

    const active = renderHook(() => useApiKeyUsage("k1", 30), {
      wrapper: wrapperFactory(),
    });
    await waitFor(() => expect(active.result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/api-keys/k1/usage?days=30");
  });
});

describe("useApiKeysUsage", () => {
  it("unwraps the `usage` array and defaults to 7 days", async () => {
    mockGet.mockResolvedValue({ usage: [{ api_key_id: "k1" }] });
    const { result } = renderHook(() => useApiKeysUsage(), {
      wrapper: wrapperFactory(),
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/api-keys/usage?days=7");
    expect(result.current.data).toEqual([{ api_key_id: "k1" }]);
  });
});

describe("useAllAdminedApiKeys", () => {
  it("merges personal + admined-org keys, deduping by id", async () => {
    mockUseOrgs.mockReturnValue({
      data: [
        { id: "org-1", your_role: "admin" },
        { id: "org-2", your_role: "member" },
      ],
    });
    mockGet.mockImplementation((path: string) => {
      if (path === "/api-keys") {
        return Promise.resolve({ keys: [{ id: "k1" }, { id: "shared" }] });
      }
      if (path === "/api-keys?org_id=org-1") {
        return Promise.resolve({ keys: [{ id: "k2" }, { id: "shared" }] });
      }
      return Promise.resolve({ keys: [] });
    });

    const { result } = renderHook(() => useAllAdminedApiKeys(), {
      wrapper: wrapperFactory(),
    });

    await waitFor(() =>
      expect(result.current.data.map((k) => k.id)).toEqual([
        "k1",
        "shared",
        "k2",
      ]),
    );
    // Only the admin org should be queried, never the member org.
    expect(mockGet).toHaveBeenCalledWith("/api-keys?org_id=org-1");
    expect(mockGet).not.toHaveBeenCalledWith("/api-keys?org_id=org-2");
  });
});

describe("useCreateApiKey body transforms", () => {
  it("joins scopes with spaces, ISO-shapes expires_at, and drops scope lists when allow_all", async () => {
    mockPost.mockResolvedValue({ id: "k1", api_key: "nyxid_..." });
    const { result } = renderHook(() => useCreateApiKey(), {
      wrapper: wrapperFactory(),
    });
    await result.current.mutateAsync({
      name: "agent",
      scopes: ["read", "write"],
      expires_at: "2026-03-25",
      description: undefined,
      allow_all_services: true,
      allow_all_nodes: true,
    } as never);

    expect(mockPost).toHaveBeenCalledWith("/api-keys", {
      name: "agent",
      scopes: "read write",
      expires_at: "2026-03-25T23:59:59.000Z",
      description: undefined,
      allowed_service_ids: undefined,
      allowed_node_ids: undefined,
      allow_all_services: true,
      allow_all_nodes: true,
      callback_url: undefined,
      target_org_id: undefined,
    });
  });

  it("keeps scope id lists when allow_all flags are false and sends null expiry", async () => {
    mockPost.mockResolvedValue({ id: "k1", api_key: "nyxid_..." });
    const { result } = renderHook(() => useCreateApiKey(), {
      wrapper: wrapperFactory(),
    });
    await result.current.mutateAsync({
      name: "scoped",
      scopes: ["read"],
      expires_at: null,
      allow_all_services: false,
      allow_all_nodes: false,
      allowed_service_ids: ["svc-1"],
      allowed_node_ids: ["node-1"],
      target_org_id: "org-9",
    } as never);

    expect(mockPost).toHaveBeenCalledWith("/api-keys", {
      name: "scoped",
      scopes: "read",
      expires_at: null,
      description: undefined,
      allowed_service_ids: ["svc-1"],
      allowed_node_ids: ["node-1"],
      allow_all_services: false,
      allow_all_nodes: false,
      callback_url: undefined,
      target_org_id: "org-9",
    });
  });
});

describe("useUpdateApiKey / useDeleteApiKey / useRotateApiKey", () => {
  it("PUTs the body to /api-keys/{id} with keyId stripped from the body", async () => {
    mockPut.mockResolvedValue({ id: "k1" });
    const { result } = renderHook(() => useUpdateApiKey(), {
      wrapper: wrapperFactory(),
    });
    await result.current.mutateAsync({
      keyId: "k1",
      name: "renamed",
      rate_limit_per_second: 5,
    });
    expect(mockPut).toHaveBeenCalledWith("/api-keys/k1", {
      name: "renamed",
      rate_limit_per_second: 5,
    });
  });

  it("DELETEs /api-keys/{id}", async () => {
    mockDelete.mockResolvedValue(undefined);
    const { result } = renderHook(() => useDeleteApiKey(), {
      wrapper: wrapperFactory(),
    });
    await result.current.mutateAsync("k1");
    expect(mockDelete).toHaveBeenCalledWith("/api-keys/k1");
  });

  it("POSTs to the rotate endpoint with no body", async () => {
    mockPost.mockResolvedValue({ id: "k1", api_key: "nyxid_new" });
    const { result } = renderHook(() => useRotateApiKey(), {
      wrapper: wrapperFactory(),
    });
    await result.current.mutateAsync("k1");
    expect(mockPost).toHaveBeenCalledWith("/api-keys/k1/rotate");
  });
});
