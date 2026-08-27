import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { PLATFORM_OPERATION_DISCOVERY_QUERY_KEY } from "@/schemas/platform-ops";
import {
  readApiKeyAuthorization,
  readBindingAuthorization,
  useAssistantKeyBindCredential,
  useAssistantKeyDelete,
  useAssistantKeyExtendScope,
  useAssistantKeyUpdate,
} from "./use-assistant-key-actions";

const { mockGet, mockPost } = vi.hoisted(() => ({
  mockGet: vi.fn(),
  mockPost: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: { get: mockGet, post: mockPost, put: vi.fn(), delete: vi.fn() },
}));

function harness() {
  const queryClient = new QueryClient({
    defaultOptions: {
      mutations: { retry: false },
      queries: { retry: false },
    },
  });
  function Wrapper({ children }: PropsWithChildren) {
    return (
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    );
  }
  return { queryClient, Wrapper };
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe("assistant key action hooks", () => {
  it("POSTs key.update to the nested effect route", async () => {
    mockPost.mockResolvedValue({
      resource: { keyId: "key-1" },
      replayed: false,
    });
    const { Wrapper } = harness();
    const { result } = renderHook(() => useAssistantKeyUpdate(), {
      wrapper: Wrapper,
    });
    await result.current.mutateAsync({
      actionRequestId: "act-1",
      keyId: "key-1",
      name: "renamed",
      expectedStateVersion: 1,
    });
    expect(mockPost).toHaveBeenCalledWith("/assistant/actions/keys/update", {
      actionRequestId: "act-1",
      keyId: "key-1",
      name: "renamed",
      expectedStateVersion: 1,
    });
  });

  it("POSTs key.delete with the expected state version", async () => {
    mockPost.mockResolvedValue({
      resource: { keyId: "key-1" },
      replayed: false,
    });
    const { Wrapper } = harness();
    const { result } = renderHook(() => useAssistantKeyDelete(), {
      wrapper: Wrapper,
    });
    await result.current.mutateAsync({
      actionRequestId: "act-1",
      keyId: "key-1",
      expectedStateVersion: 4,
    });
    expect(mockPost).toHaveBeenCalledWith("/assistant/actions/keys/delete", {
      actionRequestId: "act-1",
      keyId: "key-1",
      expectedStateVersion: 4,
    });
  });

  it("POSTs key.extend_scope with the added service ids", async () => {
    mockPost.mockResolvedValue({
      resource: { keyId: "key-1" },
      replayed: false,
    });
    const { Wrapper } = harness();
    const { result } = renderHook(() => useAssistantKeyExtendScope(), {
      wrapper: Wrapper,
    });
    await result.current.mutateAsync({
      actionRequestId: "act-1",
      keyId: "key-1",
      addServiceIds: ["svc-2"],
      expectedStateVersion: 1,
    });
    expect(mockPost).toHaveBeenCalledWith(
      "/assistant/actions/keys/extend-scope",
      {
        actionRequestId: "act-1",
        keyId: "key-1",
        addServiceIds: ["svc-2"],
        expectedStateVersion: 1,
      },
    );
  });

  it("POSTs key.bind_credential and reads binding evidence by service", async () => {
    mockPost.mockResolvedValue({
      resource: { keyId: "key-1" },
      bindingId: "bind-1",
      replayed: false,
    });
    mockGet.mockResolvedValue({
      id: "bind-1",
      api_key_id: "key-1",
      user_service_id: "svc-1",
      user_api_key_id: "cred-1",
      created_at: "2026-08-11T00:00:00Z",
      updated_at: "2026-08-11T00:00:00Z",
    });
    const { queryClient, Wrapper } = harness();
    const invalidateQueries = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useAssistantKeyBindCredential(), {
      wrapper: Wrapper,
    });
    await result.current.mutateAsync({
      actionRequestId: "act-1",
      keyId: "key-1",
      userServiceId: "svc-1",
      externalKeyId: "cred-1",
    });
    expect(mockPost).toHaveBeenCalledWith(
      "/assistant/actions/keys/bind-credential",
      {
        actionRequestId: "act-1",
        keyId: "key-1",
        userServiceId: "svc-1",
        externalKeyId: "cred-1",
      },
    );
    await expect(readBindingAuthorization("key-1", "svc-1")).resolves.toEqual({
      id: "bind-1",
      api_key_id: "key-1",
      user_service_id: "svc-1",
      user_api_key_id: "cred-1",
      created_at: "2026-08-11T00:00:00Z",
      updated_at: "2026-08-11T00:00:00Z",
    });
    expect(mockGet).toHaveBeenCalledWith(
      "/api-keys/key-1/bindings/by-service/svc-1/authorization",
    );
    expect(invalidateQueries).toHaveBeenCalledWith({
      queryKey: PLATFORM_OPERATION_DISCOVERY_QUERY_KEY,
    });
  });

  it("reads API key authorization evidence from the projection route", async () => {
    mockGet.mockResolvedValue({ id: "key-1", name: "coding-agent" });
    await expect(readApiKeyAuthorization("key-1")).resolves.toEqual({
      id: "key-1",
      name: "coding-agent",
    });
    expect(mockGet).toHaveBeenCalledWith("/api-keys/key-1/authorization");
  });
});
