import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
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
});

describe("query hooks unwrap their list envelopes", () => {
  it("useKeys returns the `keys` array from /keys", async () => {
    mockGet.mockResolvedValue({ keys: [{ id: "k1" }] });
    const { result } = renderHook(() => useKeys(), { wrapper: wrapperFactory() });

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
    const { result } = renderHook(() => useCreateKey(), {
      wrapper: wrapperFactory(),
    });

    const params = { label: "My OpenAI", service_slug: "openai", credential: "sk-x" };
    await result.current.mutateAsync(params);

    expect(mockPost).toHaveBeenCalledWith("/keys", params);
  });

  it("useDeleteKey deletes /keys/{id}", async () => {
    mockDelete.mockResolvedValue(undefined);
    const { result } = renderHook(() => useDeleteKey(), {
      wrapper: wrapperFactory(),
    });

    await result.current.mutateAsync("k1");

    expect(mockDelete).toHaveBeenCalledWith("/keys/k1");
  });

  it("useUpdateKey strips keyId from the body and PUTs to /keys/{id}", async () => {
    mockPut.mockResolvedValue({ id: "k1" });
    const { result } = renderHook(() => useUpdateKey(), {
      wrapper: wrapperFactory(),
    });

    await result.current.mutateAsync({ keyId: "k1", label: "Renamed" });

    expect(mockPut).toHaveBeenCalledWith("/keys/k1", { label: "Renamed" });
  });

  it("useUpdateEndpoint PUTs the url/label/spec triple to /endpoints/{id}", async () => {
    mockPut.mockResolvedValue(undefined);
    const { result } = renderHook(() => useUpdateEndpoint(), {
      wrapper: wrapperFactory(),
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
  });

  it("useUpdateUserService strips serviceId from the body and PUTs to /user-services/{id}", async () => {
    mockPut.mockResolvedValue(undefined);
    const { result } = renderHook(() => useUpdateUserService(), {
      wrapper: wrapperFactory(),
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
  });

  it("useUpdateExternalApiKey strips keyId and PUTs to /api-keys/external/{id}", async () => {
    mockPut.mockResolvedValue(undefined);
    const { result } = renderHook(() => useUpdateExternalApiKey(), {
      wrapper: wrapperFactory(),
    });

    await result.current.mutateAsync({ keyId: "ext1", credential: "sk-new" });

    expect(mockPut).toHaveBeenCalledWith("/api-keys/external/ext1", {
      credential: "sk-new",
    });
  });
});
