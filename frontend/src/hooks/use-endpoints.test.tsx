import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { PropsWithChildren } from "react";
import type { CreateEndpointFormData } from "@/schemas/endpoints";
import {
  useCreateEndpoint,
  useDeleteEndpoint,
  useDiscoverEndpoints,
  useEndpoints,
  useUpdateEndpoint,
} from "./use-endpoints";

const { mockDelete, mockGet, mockPost, mockPut } = vi.hoisted(() => ({
  mockDelete: vi.fn(),
  mockGet: vi.fn(),
  mockPost: vi.fn(),
  mockPut: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: {
    delete: mockDelete,
    get: mockGet,
    post: mockPost,
    put: mockPut,
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

describe("useEndpoints", () => {
  it("unwraps the `endpoints` array and gates on serviceId", async () => {
    mockGet.mockResolvedValue({ endpoints: [{ id: "ep-1" }] });

    const idle = renderHook(() => useEndpoints(""), {
      wrapper: createWrapper(),
    });
    expect(idle.result.current.fetchStatus).toBe("idle");
    expect(mockGet).not.toHaveBeenCalled();

    const active = renderHook(() => useEndpoints("svc-1"), {
      wrapper: createWrapper(),
    });
    await waitFor(() => expect(active.result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/services/svc-1/endpoints");
    expect(active.result.current.data).toEqual([{ id: "ep-1" }]);
  });
});

describe("useCreateEndpoint formToPayload transform", () => {
  it("JSON.parses parameters/request_body_schema and coerces empty strings to null", async () => {
    mockPost.mockResolvedValue({ id: "ep-1" });
    const { result } = renderHook(() => useCreateEndpoint(), {
      wrapper: createWrapper(),
    });

    const form: CreateEndpointFormData = {
      name: "list_items",
      description: "",
      method: "GET",
      path: "/items",
      parameters: '{"limit":10}',
      request_body_schema: "",
      response_description: "",
    };

    await result.current.mutateAsync({ serviceId: "svc-1", data: form });

    expect(mockPost).toHaveBeenCalledWith("/services/svc-1/endpoints", {
      name: "list_items",
      description: null,
      method: "GET",
      path: "/items",
      parameters: { limit: 10 },
      request_body_schema: null,
      response_description: null,
    });
  });

  it("preserves non-empty description/response_description and parses both JSON fields", async () => {
    mockPost.mockResolvedValue({ id: "ep-2" });
    const { result } = renderHook(() => useCreateEndpoint(), {
      wrapper: createWrapper(),
    });

    const form: CreateEndpointFormData = {
      name: "create_item",
      description: "Creates an item",
      method: "POST",
      path: "/items",
      parameters: '{"q":"x"}',
      request_body_schema: '{"type":"object"}',
      response_description: "Created",
    };

    await result.current.mutateAsync({ serviceId: "svc-1", data: form });

    expect(mockPost).toHaveBeenCalledWith("/services/svc-1/endpoints", {
      name: "create_item",
      description: "Creates an item",
      method: "POST",
      path: "/items",
      parameters: { q: "x" },
      request_body_schema: { type: "object" },
      response_description: "Created",
    });
  });
});

describe("useUpdateEndpoint", () => {
  it("PUTs the transformed payload to the specific endpoint", async () => {
    mockPut.mockResolvedValue(undefined);
    const { result } = renderHook(() => useUpdateEndpoint(), {
      wrapper: createWrapper(),
    });

    const form: CreateEndpointFormData = {
      name: "list_items",
      description: "",
      method: "GET",
      path: "/items",
      parameters: "",
      request_body_schema: "",
      response_description: "",
    };

    await result.current.mutateAsync({
      serviceId: "svc-1",
      endpointId: "ep-1",
      data: form,
    });

    expect(mockPut).toHaveBeenCalledWith("/services/svc-1/endpoints/ep-1", {
      name: "list_items",
      description: null,
      method: "GET",
      path: "/items",
      parameters: null,
      request_body_schema: null,
      response_description: null,
    });
  });
});

describe("useDeleteEndpoint", () => {
  it("DELETEs the specific endpoint", async () => {
    mockDelete.mockResolvedValue(undefined);
    const { result } = renderHook(() => useDeleteEndpoint(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({
      serviceId: "svc-1",
      endpointId: "ep-1",
    });
    expect(mockDelete).toHaveBeenCalledWith("/services/svc-1/endpoints/ep-1");
  });
});

describe("useDiscoverEndpoints", () => {
  it("POSTs to the discover-endpoints endpoint", async () => {
    mockPost.mockResolvedValue({ discovered: 3 });
    const { result } = renderHook(() => useDiscoverEndpoints(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync("svc-1");
    expect(mockPost).toHaveBeenCalledWith(
      "/services/svc-1/discover-endpoints",
    );
  });
});
