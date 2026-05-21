import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { PropsWithChildren } from "react";
import type {
  CreateServicePayload,
  UpdateServicePayload,
} from "@/types/api";
import {
  useConnections,
  useConnectService,
  useCreateService,
  useDeleteService,
  useDisconnectService,
  useOidcCredentials,
  useRegenerateOidcSecret,
  useService,
  useServices,
  useTestSshConnection,
  useUpdateCredential,
  useUpdateRedirectUris,
  useUpdateService,
  useUpdateSshAuthMode,
} from "./use-services";

const { mockDelete, mockGet, mockPatch, mockPost, mockPut } = vi.hoisted(
  () => ({
    mockDelete: vi.fn(),
    mockGet: vi.fn(),
    mockPatch: vi.fn(),
    mockPost: vi.fn(),
    mockPut: vi.fn(),
  }),
);

vi.mock("@/lib/api-client", () => ({
  api: {
    delete: mockDelete,
    get: mockGet,
    patch: mockPatch,
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

describe("SSH service hooks", () => {
  beforeEach(() => {
    mockPatch.mockReset();
    mockPost.mockReset();
  });

  it("patches the user-service SSH auth mode", async () => {
    mockPatch.mockResolvedValue(undefined);

    const { result } = renderHook(() => useUpdateSshAuthMode(), {
      wrapper: createWrapper(),
    });

    await result.current.mutateAsync({
      userServiceId: "usvc-1",
      mode: "node_key",
    });

    expect(mockPatch).toHaveBeenCalledWith(
      "/user-services/usvc-1/ssh-auth-mode",
      { mode: "node_key" },
    );
  });

  it("uses the node-key exec path for Test connection", async () => {
    mockPost.mockResolvedValue({ exit_code: 0 });

    const { result } = renderHook(() => useTestSshConnection(), {
      wrapper: createWrapper(),
    });

    await result.current.mutateAsync({
      serviceId: "svc-routeros",
      principal: "nyxid-ro",
    });

    expect(mockPost).toHaveBeenCalledWith("/ssh/svc-routeros/exec", {
      principal: "nyxid-ro",
      command: "true",
      timeout_secs: 10,
    });
  });
});

describe("service catalog queries", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("useServices unwraps the `services` array", async () => {
    mockGet.mockResolvedValue({ services: [{ id: "svc-1" }] });
    const { result } = renderHook(() => useServices(), {
      wrapper: createWrapper(),
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/services");
    expect(result.current.data).toEqual([{ id: "svc-1" }]);
  });

  it("useService fetches a single service and stays idle for an empty id", async () => {
    mockGet.mockResolvedValue({ id: "svc-1" });
    const idle = renderHook(() => useService(""), {
      wrapper: createWrapper(),
    });
    expect(idle.result.current.fetchStatus).toBe("idle");
    expect(mockGet).not.toHaveBeenCalled();

    const active = renderHook(() => useService("svc-1"), {
      wrapper: createWrapper(),
    });
    await waitFor(() => expect(active.result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/services/svc-1");
  });

  it("useOidcCredentials gates on both enabled flag and serviceId", async () => {
    mockGet.mockResolvedValue({ client_id: "cid" });

    // Disabled by the `enabled` flag even with a valid id.
    const off = renderHook(() => useOidcCredentials("svc-1", false), {
      wrapper: createWrapper(),
    });
    expect(off.result.current.fetchStatus).toBe("idle");

    // Enabled but empty id stays idle.
    const noId = renderHook(() => useOidcCredentials("", true), {
      wrapper: createWrapper(),
    });
    expect(noId.result.current.fetchStatus).toBe("idle");
    expect(mockGet).not.toHaveBeenCalled();

    const on = renderHook(() => useOidcCredentials("svc-1", true), {
      wrapper: createWrapper(),
    });
    await waitFor(() => expect(on.result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/services/svc-1/oidc-credentials");
  });

  it("useConnections unwraps the `connections` array", async () => {
    mockGet.mockResolvedValue({ connections: [{ service_id: "svc-1" }] });
    const { result } = renderHook(() => useConnections(), {
      wrapper: createWrapper(),
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/connections");
    expect(result.current.data).toEqual([{ service_id: "svc-1" }]);
  });
});

describe("service CRUD mutations", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("useCreateService POSTs to /services with the payload", async () => {
    mockPost.mockResolvedValue({ id: "svc-1" });
    const { result } = renderHook(() => useCreateService(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({
      name: "Acme",
      slug: "acme",
      base_url: "https://acme.test",
    } as unknown as CreateServicePayload);
    expect(mockPost).toHaveBeenCalledWith("/services", {
      name: "Acme",
      slug: "acme",
      base_url: "https://acme.test",
    });
  });

  it("useUpdateService PUTs to the specific service", async () => {
    mockPut.mockResolvedValue({ id: "svc-1" });
    const { result } = renderHook(() => useUpdateService(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({
      serviceId: "svc-1",
      data: { name: "Renamed" } as unknown as UpdateServicePayload,
    });
    expect(mockPut).toHaveBeenCalledWith("/services/svc-1", {
      name: "Renamed",
    });
  });

  it("useDeleteService DELETEs the specific service", async () => {
    mockDelete.mockResolvedValue(undefined);
    const { result } = renderHook(() => useDeleteService(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync("svc-1");
    expect(mockDelete).toHaveBeenCalledWith("/services/svc-1");
  });
});

describe("OIDC credential mutations", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("useUpdateRedirectUris wraps the array under redirect_uris", async () => {
    mockPut.mockResolvedValue({ redirect_uris: ["https://a", "https://b"] });
    const { result } = renderHook(() => useUpdateRedirectUris(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({
      serviceId: "svc-1",
      redirectUris: ["https://a", "https://b"],
    });
    expect(mockPut).toHaveBeenCalledWith("/services/svc-1/redirect-uris", {
      redirect_uris: ["https://a", "https://b"],
    });
  });

  it("useRegenerateOidcSecret POSTs to the regenerate-secret endpoint", async () => {
    mockPost.mockResolvedValue({ client_secret: "new-secret" });
    const { result } = renderHook(() => useRegenerateOidcSecret(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync("svc-1");
    expect(mockPost).toHaveBeenCalledWith("/services/svc-1/regenerate-secret");
  });
});

describe("connection mutations", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("useConnectService renames credential + credentialLabel into snake_case body", async () => {
    mockPost.mockResolvedValue({
      service_id: "svc-1",
      service_name: "Acme",
      connected_at: "now",
    });
    const { result } = renderHook(() => useConnectService(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({
      serviceId: "svc-1",
      credential: "sk-x",
      credentialLabel: "My Key",
    });
    expect(mockPost).toHaveBeenCalledWith("/connections/svc-1", {
      credential: "sk-x",
      credential_label: "My Key",
    });
  });

  it("useUpdateCredential PUTs the renamed body to the credential resource", async () => {
    mockPut.mockResolvedValue(undefined);
    const { result } = renderHook(() => useUpdateCredential(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({
      serviceId: "svc-1",
      credential: "sk-y",
      credentialLabel: "Rotated",
    });
    expect(mockPut).toHaveBeenCalledWith("/connections/svc-1/credential", {
      credential: "sk-y",
      credential_label: "Rotated",
    });
  });

  it("useDisconnectService DELETEs the connection", async () => {
    mockDelete.mockResolvedValue(undefined);
    const { result } = renderHook(() => useDisconnectService(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync("svc-1");
    expect(mockDelete).toHaveBeenCalledWith("/connections/svc-1");
  });
});
