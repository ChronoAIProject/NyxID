import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  useConnectApiKeyForSa,
  useConnectSaService,
  useCreateServiceAccount,
  useDeleteServiceAccount,
  useDisconnectSaProvider,
  useDisconnectSaService,
  useInitiateDeviceCodeForSa,
  useInitiateOAuthForSa,
  usePollDeviceCodeForSa,
  useRevokeTokens,
  useRotateSecret,
  useSaConnections,
  useSaProviders,
  useServiceAccount,
  useServiceAccounts,
  useUpdateSaConnectionCredential,
  useUpdateServiceAccount,
} from "./use-service-accounts";

const { mockGet, mockPost, mockPut, mockDelete } = vi.hoisted(() => ({
  mockGet: vi.fn(),
  mockPost: vi.fn(),
  mockPut: vi.fn(),
  mockDelete: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: { get: mockGet, post: mockPost, put: mockPut, delete: mockDelete },
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

describe("service account list query", () => {
  it("builds the page/per_page query string with defaults only", async () => {
    mockGet.mockResolvedValue({ service_accounts: [], total: 0 });
    const { result } = renderHook(() => useServiceAccounts(2, 25), {
      wrapper: createWrapper(),
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith(
      "/admin/service-accounts?page=2&per_page=25",
    );
  });

  it("appends search and org_id when provided", async () => {
    mockGet.mockResolvedValue({ service_accounts: [], total: 0 });
    const { result } = renderHook(
      () => useServiceAccounts(1, 10, "bot", "org-1"),
      { wrapper: createWrapper() },
    );
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith(
      "/admin/service-accounts?page=1&per_page=10&search=bot&org_id=org-1",
    );
  });

  it("useServiceAccount fetches by id and stays idle for an empty id", async () => {
    mockGet.mockResolvedValue({ id: "sa-1" });
    const idle = renderHook(() => useServiceAccount(""), {
      wrapper: createWrapper(),
    });
    expect(idle.result.current.fetchStatus).toBe("idle");
    expect(mockGet).not.toHaveBeenCalled();

    const active = renderHook(() => useServiceAccount("sa-1"), {
      wrapper: createWrapper(),
    });
    await waitFor(() => expect(active.result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/admin/service-accounts/sa-1");
  });
});

describe("service account CRUD mutations", () => {
  it("useCreateServiceAccount POSTs the body to /admin/service-accounts", async () => {
    mockPost.mockResolvedValue({ id: "sa-1", client_secret: "secret" });
    const { result } = renderHook(() => useCreateServiceAccount(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({ name: "ci-bot" } as never);
    expect(mockPost).toHaveBeenCalledWith("/admin/service-accounts", {
      name: "ci-bot",
    });
  });

  it("useUpdateServiceAccount PUTs to /admin/service-accounts/{saId}", async () => {
    mockPut.mockResolvedValue({ id: "sa-1" });
    const { result } = renderHook(() => useUpdateServiceAccount(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({
      saId: "sa-1",
      data: { name: "renamed" } as never,
    });
    expect(mockPut).toHaveBeenCalledWith("/admin/service-accounts/sa-1", {
      name: "renamed",
    });
  });

  it("useDeleteServiceAccount DELETEs /admin/service-accounts/{saId}", async () => {
    mockDelete.mockResolvedValue({ message: "deleted" });
    const { result } = renderHook(() => useDeleteServiceAccount(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync("sa-1");
    expect(mockDelete).toHaveBeenCalledWith("/admin/service-accounts/sa-1");
  });

  it("useRotateSecret POSTs to the rotate-secret endpoint", async () => {
    mockPost.mockResolvedValue({ client_secret: "new" });
    const { result } = renderHook(() => useRotateSecret(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync("sa-1");
    expect(mockPost).toHaveBeenCalledWith(
      "/admin/service-accounts/sa-1/rotate-secret",
    );
  });

  it("useRevokeTokens POSTs to the revoke-tokens endpoint", async () => {
    mockPost.mockResolvedValue({ revoked_count: 3 });
    const { result } = renderHook(() => useRevokeTokens(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync("sa-1");
    expect(mockPost).toHaveBeenCalledWith(
      "/admin/service-accounts/sa-1/revoke-tokens",
    );
  });
});

describe("SA provider hooks", () => {
  it("useSaProviders unwraps `tokens` and gates on saId", async () => {
    mockGet.mockResolvedValue({ tokens: [{ provider_id: "openai" }] });
    const idle = renderHook(() => useSaProviders(""), {
      wrapper: createWrapper(),
    });
    expect(idle.result.current.fetchStatus).toBe("idle");
    expect(mockGet).not.toHaveBeenCalled();

    const active = renderHook(() => useSaProviders("sa-1"), {
      wrapper: createWrapper(),
    });
    await waitFor(() => expect(active.result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/admin/service-accounts/sa-1/providers");
    expect(active.result.current.data).toEqual([{ provider_id: "openai" }]);
  });

  it("useConnectApiKeyForSa renames apiKey->api_key in the body", async () => {
    mockPost.mockResolvedValue({ status: "connected" });
    const { result } = renderHook(() => useConnectApiKeyForSa(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({
      saId: "sa-1",
      providerId: "openai",
      apiKey: "sk-x",
      label: "Prod",
    });
    expect(mockPost).toHaveBeenCalledWith(
      "/admin/service-accounts/sa-1/providers/openai/connect/api-key",
      { api_key: "sk-x", label: "Prod" },
    );
  });

  it("useDisconnectSaProvider DELETEs the provider disconnect endpoint", async () => {
    mockDelete.mockResolvedValue({ status: "disconnected" });
    const { result } = renderHook(() => useDisconnectSaProvider(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({ saId: "sa-1", providerId: "openai" });
    expect(mockDelete).toHaveBeenCalledWith(
      "/admin/service-accounts/sa-1/providers/openai/disconnect",
    );
  });

  it("useInitiateOAuthForSa POSTs to the canonical oauth connect route", async () => {
    mockPost.mockResolvedValue({ authorization_url: "https://x" });
    const { result } = renderHook(() => useInitiateOAuthForSa(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({ saId: "sa-1", providerId: "github" });
    expect(mockPost).toHaveBeenCalledWith(
      "/admin/service-accounts/sa-1/providers/github/connect/oauth",
    );
  });

  it("useInitiateDeviceCodeForSa POSTs to the device-code initiate route", async () => {
    mockPost.mockResolvedValue({ device_code: "d" });
    const { result } = renderHook(() => useInitiateDeviceCodeForSa(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({ saId: "sa-1", providerId: "openai" });
    expect(mockPost).toHaveBeenCalledWith(
      "/admin/service-accounts/sa-1/providers/openai/connect/device-code/initiate",
    );
  });

  it("usePollDeviceCodeForSa POSTs the state to the poll route", async () => {
    mockPost.mockResolvedValue({ status: "pending" });
    const { result } = renderHook(() => usePollDeviceCodeForSa(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({
      saId: "sa-1",
      providerId: "openai",
      state: "st-1",
    });
    expect(mockPost).toHaveBeenCalledWith(
      "/admin/service-accounts/sa-1/providers/openai/connect/device-code/poll",
      { state: "st-1" },
    );
  });
});

describe("SA service connection hooks", () => {
  it("useSaConnections unwraps `connections` and gates on saId", async () => {
    mockGet.mockResolvedValue({ connections: [{ service_id: "svc-1" }] });
    const idle = renderHook(() => useSaConnections(""), {
      wrapper: createWrapper(),
    });
    expect(idle.result.current.fetchStatus).toBe("idle");
    expect(mockGet).not.toHaveBeenCalled();

    const active = renderHook(() => useSaConnections("sa-1"), {
      wrapper: createWrapper(),
    });
    await waitFor(() => expect(active.result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith(
      "/admin/service-accounts/sa-1/connections",
    );
    expect(active.result.current.data).toEqual([{ service_id: "svc-1" }]);
  });

  it("useConnectSaService renames credentialLabel->credential_label", async () => {
    mockPost.mockResolvedValue({ status: "connected" });
    const { result } = renderHook(() => useConnectSaService(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({
      saId: "sa-1",
      serviceId: "svc-1",
      credential: "tok",
      credentialLabel: "Prod key",
    });
    expect(mockPost).toHaveBeenCalledWith(
      "/admin/service-accounts/sa-1/connections/svc-1",
      { credential: "tok", credential_label: "Prod key" },
    );
  });

  it("useUpdateSaConnectionCredential PUTs to the credential sub-resource", async () => {
    mockPut.mockResolvedValue({ status: "updated" });
    const { result } = renderHook(() => useUpdateSaConnectionCredential(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({
      saId: "sa-1",
      serviceId: "svc-1",
      credential: "tok2",
      credentialLabel: "Rotated",
    });
    expect(mockPut).toHaveBeenCalledWith(
      "/admin/service-accounts/sa-1/connections/svc-1/credential",
      { credential: "tok2", credential_label: "Rotated" },
    );
  });

  it("useDisconnectSaService DELETEs the connection", async () => {
    mockDelete.mockResolvedValue({ status: "disconnected" });
    const { result } = renderHook(() => useDisconnectSaService(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({ saId: "sa-1", serviceId: "svc-1" });
    expect(mockDelete).toHaveBeenCalledWith(
      "/admin/service-accounts/sa-1/connections/svc-1",
    );
  });
});
