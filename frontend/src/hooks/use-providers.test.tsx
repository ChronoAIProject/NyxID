import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { PropsWithChildren } from "react";
import type { TelegramLoginData } from "@/types/api";
import {
  useConnectApiKey,
  useConnectTelegramWidget,
  useCreateProvider,
  useDeleteProvider,
  useDeleteProviderCredentials,
  useDisconnectProvider,
  useInitiateDeviceCode,
  useInitiateOAuth,
  useMyProviderTokens,
  usePollDeviceCode,
  useProvider,
  useProviders,
  useRefreshProviderToken,
  useServiceRequirements,
  useSetProviderCredentials,
  useUpdateProvider,
} from "./use-providers";

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

describe("useConnectTelegramWidget", () => {
  beforeEach(() => {
    mockDelete.mockReset();
    mockGet.mockReset();
    mockPost.mockReset();
  });

  it("coerces Telegram widget numeric fields before posting", async () => {
    mockPost.mockResolvedValue({
      status: "connected",
      message: "Telegram identity verified and stored",
    });

    const { result } = renderHook(() => useConnectTelegramWidget(), {
      wrapper: createWrapper(),
    });

    await result.current.mutateAsync({
      providerId: "provider-telegram",
      data: {
        id: "12345",
        first_name: "Nyx",
        auth_date: "1742518400",
        hash: "a".repeat(64),
      } as unknown as TelegramLoginData,
    });

    expect(mockPost).toHaveBeenCalledWith(
      "/providers/provider-telegram/connect/telegram/callback",
      {
        id: 12345,
        first_name: "Nyx",
        auth_date: 1742518400,
        hash: "a".repeat(64),
      },
    );
  });

  it("rejects malformed Telegram widget payloads before posting", async () => {
    const { result } = renderHook(() => useConnectTelegramWidget(), {
      wrapper: createWrapper(),
    });

    await expect(
      result.current.mutateAsync({
        providerId: "provider-telegram",
        data: {
          id: 12345,
          first_name: "Nyx",
          auth_date: 1742518400,
          hash: "deadbeef",
        } as TelegramLoginData,
      }),
    ).rejects.toThrow("Invalid Telegram login hash");

    expect(mockPost).not.toHaveBeenCalled();
  });
});

describe("provider token scope hooks", () => {
  beforeEach(() => {
    mockDelete.mockReset();
    mockGet.mockReset();
    mockPost.mockReset();
  });

  it("appends target_org_id when listing provider tokens for an org", async () => {
    mockGet.mockResolvedValue({ tokens: [] });

    const { result } = renderHook(
      () => useMyProviderTokens({ targetOrgId: "org-1" }),
      { wrapper: createWrapper() },
    );

    await waitFor(() => {
      expect(result.current.isSuccess).toBe(true);
    });
    expect(mockGet).toHaveBeenCalledWith(
      "/providers/my-tokens?target_org_id=org-1",
    );
  });

  it("appends target_org_id when disconnecting an org provider token", async () => {
    mockDelete.mockResolvedValue({
      status: "disconnected",
      message: "Provider disconnected",
    });

    const { result } = renderHook(() => useDisconnectProvider(), {
      wrapper: createWrapper(),
    });

    await result.current.mutateAsync({
      providerId: "provider-1",
      targetOrgId: "org-1",
    });

    expect(mockDelete).toHaveBeenCalledWith(
      "/providers/provider-1/disconnect?target_org_id=org-1",
    );
  });

  it("omits the suffix for personal disconnect and refresh", async () => {
    mockDelete.mockResolvedValue({ status: "disconnected" });
    mockPost.mockResolvedValue({ status: "refreshed" });

    const disconnect = renderHook(() => useDisconnectProvider(), {
      wrapper: createWrapper(),
    });
    await disconnect.result.current.mutateAsync({ providerId: "p1" });
    expect(mockDelete).toHaveBeenCalledWith("/providers/p1/disconnect");

    const refresh = renderHook(() => useRefreshProviderToken(), {
      wrapper: createWrapper(),
    });
    await refresh.result.current.mutateAsync({ providerId: "p1" });
    expect(mockPost).toHaveBeenCalledWith("/providers/p1/refresh");
  });
});

describe("list + detail queries", () => {
  beforeEach(() => {
    mockGet.mockReset();
  });

  it("useProviders unwraps the `providers` array", async () => {
    mockGet.mockResolvedValue({ providers: [{ id: "p1" }] });
    const { result } = renderHook(() => useProviders(), {
      wrapper: createWrapper(),
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/providers");
    expect(result.current.data).toEqual([{ id: "p1" }]);
  });

  it("useProvider fetches by id and stays idle for an empty id", async () => {
    mockGet.mockResolvedValue({ id: "p1" });
    const idle = renderHook(() => useProvider(""), {
      wrapper: createWrapper(),
    });
    expect(idle.result.current.fetchStatus).toBe("idle");
    expect(mockGet).not.toHaveBeenCalled();

    const active = renderHook(() => useProvider("p1"), {
      wrapper: createWrapper(),
    });
    await waitFor(() => expect(active.result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/providers/p1");
  });

  it("useServiceRequirements unwraps `requirements` and gates on serviceId", async () => {
    mockGet.mockResolvedValue({ requirements: [{ provider_slug: "openai" }] });
    const { result } = renderHook(() => useServiceRequirements("svc-1"), {
      wrapper: createWrapper(),
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/services/svc-1/requirements");
    expect(result.current.data).toEqual([{ provider_slug: "openai" }]);
  });
});

describe("useConnectApiKey", () => {
  beforeEach(() => {
    mockPost.mockReset();
  });

  it("sends the gateway URL when provided", async () => {
    mockPost.mockResolvedValue({ status: "connected" });
    const { result } = renderHook(() => useConnectApiKey(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({
      providerId: "openclaw",
      apiKey: "sk-x",
      label: "My Gateway",
      gatewayUrl: "https://gw.example.com",
    });
    expect(mockPost).toHaveBeenCalledWith("/providers/openclaw/connect/api-key", {
      api_key: "sk-x",
      label: "My Gateway",
      gateway_url: "https://gw.example.com",
    });
  });

  it("coerces an empty gateway URL to undefined", async () => {
    mockPost.mockResolvedValue({ status: "connected" });
    const { result } = renderHook(() => useConnectApiKey(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({
      providerId: "openai",
      apiKey: "sk-x",
      gatewayUrl: "",
    });
    expect(mockPost).toHaveBeenCalledWith("/providers/openai/connect/api-key", {
      api_key: "sk-x",
      label: undefined,
      gateway_url: undefined,
    });
  });
});

describe("useInitiateOAuth query building", () => {
  beforeEach(() => {
    mockGet.mockReset();
  });

  it("hits the bare endpoint when given a plain provider id string", async () => {
    mockGet.mockResolvedValue({ authorization_url: "https://x" });
    const { result } = renderHook(() => useInitiateOAuth(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync("github");
    expect(mockGet).toHaveBeenCalledWith("/providers/github/connect/oauth");
  });

  it("builds redirect_path, comma-joined scope, target_org_id and key_id", async () => {
    mockGet.mockResolvedValue({ authorization_url: "https://x" });
    const { result } = renderHook(() => useInitiateOAuth(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({
      providerId: "github",
      redirectPath: "/keys",
      additionalScopes: ["repo", "read:org"],
      targetOrgId: "org-1",
      keyId: "key-9",
    });
    expect(mockGet).toHaveBeenCalledWith(
      "/providers/github/connect/oauth?redirect_path=%2Fkeys&scope=repo%2Cread%3Aorg&target_org_id=org-1&key_id=key-9",
    );
  });
});

describe("useInitiateDeviceCode query building", () => {
  beforeEach(() => {
    mockPost.mockReset();
  });

  it("posts to the bare initiate endpoint for a plain id", async () => {
    mockPost.mockResolvedValue({ device_code: "d" });
    const { result } = renderHook(() => useInitiateDeviceCode(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync("openai");
    expect(mockPost).toHaveBeenCalledWith(
      "/providers/openai/connect/device-code/initiate",
    );
  });

  it("appends scope and target_org_id when provided", async () => {
    mockPost.mockResolvedValue({ device_code: "d" });
    const { result } = renderHook(() => useInitiateDeviceCode(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({
      providerId: "openai",
      additionalScopes: ["a", "b"],
      targetOrgId: "org-1",
    });
    expect(mockPost).toHaveBeenCalledWith(
      "/providers/openai/connect/device-code/initiate?scope=a%2Cb&target_org_id=org-1",
    );
  });
});

describe("usePollDeviceCode", () => {
  beforeEach(() => {
    mockPost.mockReset();
  });

  it("posts the polling state to the poll endpoint", async () => {
    mockPost.mockResolvedValue({ status: "pending" });
    const { result } = renderHook(() => usePollDeviceCode(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({ providerId: "openai", state: "st-1" });
    expect(mockPost).toHaveBeenCalledWith(
      "/providers/openai/connect/device-code/poll",
      { state: "st-1" },
    );
  });
});

describe("provider credentials + admin CRUD", () => {
  beforeEach(() => {
    mockGet.mockReset();
    mockPost.mockReset();
    mockPut.mockReset();
    mockDelete.mockReset();
  });

  it("useSetProviderCredentials PUTs the client id/secret/label", async () => {
    mockPut.mockResolvedValue({ client_id: "cid" });
    const { result } = renderHook(() => useSetProviderCredentials(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({
      providerId: "p1",
      client_id: "cid",
      client_secret: "csecret",
      label: "App",
    });
    expect(mockPut).toHaveBeenCalledWith("/providers/p1/credentials", {
      client_id: "cid",
      client_secret: "csecret",
      label: "App",
    });
  });

  it("useDeleteProviderCredentials DELETEs the credentials resource", async () => {
    mockDelete.mockResolvedValue({ message: "deleted" });
    const { result } = renderHook(() => useDeleteProviderCredentials(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync("p1");
    expect(mockDelete).toHaveBeenCalledWith("/providers/p1/credentials");
  });

  it("useCreateProvider POSTs the new provider config", async () => {
    mockPost.mockResolvedValue({ id: "p1" });
    const { result } = renderHook(() => useCreateProvider(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({
      name: "Acme",
      slug: "acme",
      provider_type: "api_key",
    });
    expect(mockPost).toHaveBeenCalledWith("/providers", {
      name: "Acme",
      slug: "acme",
      provider_type: "api_key",
    });
  });

  it("useUpdateProvider PUTs to the specific provider", async () => {
    mockPut.mockResolvedValue({ id: "p1" });
    const { result } = renderHook(() => useUpdateProvider("p1"), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({ name: "Renamed", is_active: false });
    expect(mockPut).toHaveBeenCalledWith("/providers/p1", {
      name: "Renamed",
      is_active: false,
    });
  });

  it("useDeleteProvider DELETEs the specific provider", async () => {
    mockDelete.mockResolvedValue({ message: "deleted" });
    const { result } = renderHook(() => useDeleteProvider(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync("p1");
    expect(mockDelete).toHaveBeenCalledWith("/providers/p1");
  });
});
