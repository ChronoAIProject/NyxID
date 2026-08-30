import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { PLATFORM_OPERATION_DISCOVERY_QUERY_KEY } from "@/schemas/platform-ops";
import {
  useCatalog,
  useCatalogEntry,
  useCreateKey,
  useDeleteKey,
  useExternalApiKeys,
  useKey,
  useKeys,
  useUpdateEndpoint,
  useUpdateExternalApiKey,
  useUpdateKey,
  useUpdateUserService,
} from "./use-keys";

const { mockGet, mockPost, mockPut, mockDelete } = vi.hoisted(() => ({
  mockGet: vi.fn(),
  mockPost: vi.fn(),
  mockPut: vi.fn(),
  mockDelete: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: { get: mockGet, post: mockPost, put: mockPut, delete: mockDelete },
}));

function createQueryClient() {
  return new QueryClient({
    defaultOptions: {
      mutations: { retry: false },
      queries: { retry: false },
    },
  });
}

function wrapperFactory(queryClient = createQueryClient()) {
  return ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );
}

function mutationHarness() {
  const queryClient = createQueryClient();
  return {
    invalidateQueries: vi.spyOn(queryClient, "invalidateQueries"),
    Wrapper: wrapperFactory(queryClient),
  };
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe("query hooks unwrap their list envelopes", () => {
  it("useKeys returns the `keys` array from /keys", async () => {
    mockGet.mockResolvedValue({ keys: [{ id: "k1" }] });
    const { result } = renderHook(() => useKeys(), {
      wrapper: wrapperFactory(),
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/keys");
    expect(result.current.data).toEqual([{ id: "k1" }]);
  });

  it("useCatalog returns the `entries` array from /catalog", async () => {
    mockGet.mockResolvedValue({ entries: [{ slug: "openai" }] });
    const { result } = renderHook(() => useCatalog(), {
      wrapper: wrapperFactory(),
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/catalog");
    expect(result.current.data).toEqual([{ slug: "openai" }]);
  });

  it("useCatalog can include all catalog entries when requested", async () => {
    mockGet.mockResolvedValue({ entries: [{ slug: "chrono-llm-public" }] });
    const { result } = renderHook(() => useCatalog({ includeAll: true }), {
      wrapper: wrapperFactory(),
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/catalog?include_all=true");
    expect(result.current.data).toEqual([{ slug: "chrono-llm-public" }]);
  });

  it("useExternalApiKeys returns the `api_keys` array from /api-keys/external", async () => {
    mockGet.mockResolvedValue({ api_keys: [{ id: "ext1" }] });
    const { result } = renderHook(() => useExternalApiKeys(), {
      wrapper: wrapperFactory(),
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/api-keys/external");
    expect(result.current.data).toEqual([{ id: "ext1" }]);
  });
});

describe("useKey", () => {
  it("fetches a single key by id", async () => {
    mockGet.mockResolvedValue({ id: "k1" });
    const { result } = renderHook(() => useKey("k1"), {
      wrapper: wrapperFactory(),
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/keys/k1");
  });

  it("is disabled (issues no request) when the id is empty", () => {
    const { result } = renderHook(() => useKey(""), {
      wrapper: wrapperFactory(),
    });

    expect(result.current.fetchStatus).toBe("idle");
    expect(mockGet).not.toHaveBeenCalled();
  });
});

describe("useCatalogEntry", () => {
  it("URL-encodes the slug so namespaced slugs stay valid", async () => {
    mockGet.mockResolvedValue({ slug: "acme/thing" });
    const { result } = renderHook(() => useCatalogEntry("acme/thing"), {
      wrapper: wrapperFactory(),
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/catalog/acme%2Fthing");
  });

  it("disables itself for a custom (slug-less) key", () => {
    const { result } = renderHook(() => useCatalogEntry(null), {
      wrapper: wrapperFactory(),
    });

    expect(result.current.fetchStatus).toBe("idle");
    expect(mockGet).not.toHaveBeenCalled();
  });
});

describe("mutation hooks pin their request contracts", () => {
  it("useCreateKey posts the params to /keys", async () => {
    mockPost.mockResolvedValue({ id: "k1" });
    const { Wrapper, invalidateQueries } = mutationHarness();
    const { result } = renderHook(() => useCreateKey(), {
      wrapper: Wrapper,
    });

    const params = {
      label: "My OpenAI",
      service_slug: "openai",
      credential: "sk-x",
    };
    await result.current.mutateAsync(params);

    expect(mockPost).toHaveBeenCalledWith("/keys", params);
    expect(invalidateQueries).toHaveBeenCalledWith({
      queryKey: PLATFORM_OPERATION_DISCOVERY_QUERY_KEY,
    });
  });

  it("useDeleteKey deletes /keys/{id}", async () => {
    mockDelete.mockResolvedValue(undefined);
    const { Wrapper, invalidateQueries } = mutationHarness();
    const { result } = renderHook(() => useDeleteKey(), {
      wrapper: Wrapper,
    });

    await result.current.mutateAsync("k1");

    expect(mockDelete).toHaveBeenCalledWith("/keys/k1");
    expect(invalidateQueries).toHaveBeenCalledWith({
      queryKey: PLATFORM_OPERATION_DISCOVERY_QUERY_KEY,
    });
  });

  it("useDeleteKey encodes grant cascade retry options", async () => {
    mockDelete.mockResolvedValue({
      deleted: true,
      upstream_revocation_scheduled: true,
    });
    const { result } = renderHook(() => useDeleteKey(), {
      wrapper: wrapperFactory(),
    });

    await result.current.mutateAsync({ keyId: "k1", cascadeGrant: true });
    expect(mockDelete).toHaveBeenLastCalledWith("/keys/k1?cascade_grant=true");

    await result.current.mutateAsync({ keyId: "k1", grantScope: "token" });
    expect(mockDelete).toHaveBeenLastCalledWith("/keys/k1?grant_scope=token");
  });

  it("useUpdateKey strips keyId from the body and PUTs to /keys/{id}", async () => {
    mockPut.mockResolvedValue({ id: "k1" });
    const { Wrapper, invalidateQueries } = mutationHarness();
    const { result } = renderHook(() => useUpdateKey(), {
      wrapper: Wrapper,
    });

    await result.current.mutateAsync({ keyId: "k1", label: "Renamed" });

    expect(mockPut).toHaveBeenCalledWith("/keys/k1", { label: "Renamed" });
    expect(invalidateQueries).toHaveBeenCalledWith({
      queryKey: PLATFORM_OPERATION_DISCOVERY_QUERY_KEY,
    });
  });

  it("useUpdateEndpoint PUTs the url/label/spec triple to /endpoints/{id}", async () => {
    mockPut.mockResolvedValue(undefined);
    const { Wrapper, invalidateQueries } = mutationHarness();
    const { result } = renderHook(() => useUpdateEndpoint(), {
      wrapper: Wrapper,
    });

    await result.current.mutateAsync({
      endpointId: "ep1",
      url: "https://api.example.com",
      label: "Example",
      openapi_spec_url: "",
    });

    expect(mockPut).toHaveBeenCalledWith("/endpoints/ep1", {
      url: "https://api.example.com",
      label: "Example",
      openapi_spec_url: "",
    });
    expect(invalidateQueries).toHaveBeenCalledWith({
      queryKey: PLATFORM_OPERATION_DISCOVERY_QUERY_KEY,
    });
  });

  it("useUpdateUserService strips serviceId from the body and PUTs to /user-services/{id}", async () => {
    mockPut.mockResolvedValue(undefined);
    const { Wrapper, invalidateQueries } = mutationHarness();
    const { result } = renderHook(() => useUpdateUserService(), {
      wrapper: Wrapper,
    });

    await result.current.mutateAsync({
      serviceId: "svc1",
      auth_method: "bearer",
      is_active: false,
    });

    expect(mockPut).toHaveBeenCalledWith("/user-services/svc1", {
      auth_method: "bearer",
      is_active: false,
    });
    expect(invalidateQueries).toHaveBeenCalledWith({
      queryKey: PLATFORM_OPERATION_DISCOVERY_QUERY_KEY,
    });
  });

  it("useUpdateExternalApiKey strips keyId and PUTs to /api-keys/external/{id}", async () => {
    mockPut.mockResolvedValue(undefined);
    const { Wrapper, invalidateQueries } = mutationHarness();
    const { result } = renderHook(() => useUpdateExternalApiKey(), {
      wrapper: Wrapper,
    });

    await result.current.mutateAsync({ keyId: "ext1", credential: "sk-new" });

    expect(mockPut).toHaveBeenCalledWith("/api-keys/external/ext1", {
      credential: "sk-new",
    });
    expect(invalidateQueries).toHaveBeenCalledWith({
      queryKey: PLATFORM_OPERATION_DISCOVERY_QUERY_KEY,
    });
  });
});
