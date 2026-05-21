import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { PropsWithChildren } from "react";
import {
  useCreateDeveloperApp,
  useDeleteDeveloperApp,
  useDeveloperApp,
  useDeveloperApps,
  useRotateDeveloperAppSecret,
  useUpdateDeveloperApp,
} from "./use-developer-apps";

const { mockDelete, mockGet, mockPatch, mockPost } = vi.hoisted(() => ({
  mockDelete: vi.fn(),
  mockGet: vi.fn(),
  mockPatch: vi.fn(),
  mockPost: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: {
    delete: mockDelete,
    get: mockGet,
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

beforeEach(() => {
  vi.clearAllMocks();
});

describe("useDeveloperApps", () => {
  it("hits the bare endpoint and returns the full response when no org is given", async () => {
    mockGet.mockResolvedValue({ clients: [{ id: "app-1" }] });
    const { result } = renderHook(() => useDeveloperApps(), {
      wrapper: createWrapper(),
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/developer/oauth-clients");
    expect(result.current.data).toEqual({ clients: [{ id: "app-1" }] });
  });

  it("appends an org_id query param when scoped to an org", async () => {
    mockGet.mockResolvedValue({ clients: [] });
    const { result } = renderHook(() => useDeveloperApps("org-1"), {
      wrapper: createWrapper(),
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith(
      "/developer/oauth-clients?org_id=org-1",
    );
  });
});

describe("useDeveloperApp", () => {
  it("fetches by client id and stays idle for an empty id", async () => {
    mockGet.mockResolvedValue({ id: "app-1" });
    const idle = renderHook(() => useDeveloperApp(""), {
      wrapper: createWrapper(),
    });
    expect(idle.result.current.fetchStatus).toBe("idle");
    expect(mockGet).not.toHaveBeenCalled();

    const active = renderHook(() => useDeveloperApp("app-1"), {
      wrapper: createWrapper(),
    });
    await waitFor(() => expect(active.result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/developer/oauth-clients/app-1");
  });
});

describe("developer app mutations", () => {
  it("useCreateDeveloperApp POSTs the create payload", async () => {
    mockPost.mockResolvedValue({ id: "app-1" });
    const { result } = renderHook(() => useCreateDeveloperApp(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({
      name: "My App",
      redirect_uris: ["https://app.test/cb"],
      client_type: "confidential",
    });
    expect(mockPost).toHaveBeenCalledWith("/developer/oauth-clients", {
      name: "My App",
      redirect_uris: ["https://app.test/cb"],
      client_type: "confidential",
    });
  });

  it("useUpdateDeveloperApp PATCHes the specific client with the data", async () => {
    mockPatch.mockResolvedValue({ id: "app-1" });
    const { result } = renderHook(() => useUpdateDeveloperApp(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({
      clientId: "app-1",
      data: { name: "Renamed" },
    });
    expect(mockPatch).toHaveBeenCalledWith("/developer/oauth-clients/app-1", {
      name: "Renamed",
    });
  });

  it("useDeleteDeveloperApp DELETEs the specific client", async () => {
    mockDelete.mockResolvedValue({ message: "deleted" });
    const { result } = renderHook(() => useDeleteDeveloperApp(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync("app-1");
    expect(mockDelete).toHaveBeenCalledWith("/developer/oauth-clients/app-1");
  });

  it("useRotateDeveloperAppSecret POSTs to the rotate-secret endpoint", async () => {
    mockPost.mockResolvedValue({ id: "app-1", client_secret: "new-secret" });
    const { result } = renderHook(() => useRotateDeveloperAppSecret(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync("app-1");
    expect(mockPost).toHaveBeenCalledWith(
      "/developer/oauth-clients/app-1/rotate-secret",
    );
  });
});
