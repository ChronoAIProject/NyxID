import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { PropsWithChildren } from "react";
import type {
  CreateRegistrationTokenFormData,
  PushNodeCredentialFormData,
  TransferNodeFormData,
} from "@/schemas/nodes";
import {
  useCancelNodePendingCredential,
  useCreateRegistrationToken,
  useDeleteNode,
  useMyNodeBindings,
  useNode,
  useNodeAdmins,
  useNodePendingCredentials,
  useNodes,
  usePushNodeCredential,
  useRotateNodeToken,
  useTransferNode,
} from "./use-nodes";

const { mockDelete, mockGet, mockPost } = vi.hoisted(() => ({
  mockDelete: vi.fn(),
  mockGet: vi.fn(),
  mockPost: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: {
    delete: mockDelete,
    get: mockGet,
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

describe("node queries", () => {
  it("useMyNodeBindings unwraps `service_ids`", async () => {
    mockGet.mockResolvedValue({ service_ids: ["svc-1", "svc-2"] });
    const { result } = renderHook(() => useMyNodeBindings(), {
      wrapper: createWrapper(),
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/nodes/my-bindings");
    expect(result.current.data).toEqual(["svc-1", "svc-2"]);
  });

  it("useNodes unwraps the `nodes` array", async () => {
    mockGet.mockResolvedValue({ nodes: [{ id: "node-1" }] });
    const { result } = renderHook(() => useNodes(), {
      wrapper: createWrapper(),
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/nodes");
    expect(result.current.data).toEqual([{ id: "node-1" }]);
  });

  it("useNode fetches by id and stays idle for an empty id", async () => {
    mockGet.mockResolvedValue({ id: "node-1" });
    const idle = renderHook(() => useNode(""), { wrapper: createWrapper() });
    expect(idle.result.current.fetchStatus).toBe("idle");
    expect(mockGet).not.toHaveBeenCalled();

    const active = renderHook(() => useNode("node-1"), {
      wrapper: createWrapper(),
    });
    await waitFor(() => expect(active.result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/nodes/node-1");
  });

  it("useNodeAdmins unwraps `admins` and gates on nodeId", async () => {
    mockGet.mockResolvedValue({ admins: [{ user_id: "u1" }] });
    const idle = renderHook(() => useNodeAdmins(""), {
      wrapper: createWrapper(),
    });
    expect(idle.result.current.fetchStatus).toBe("idle");
    expect(mockGet).not.toHaveBeenCalled();

    const active = renderHook(() => useNodeAdmins("node-1"), {
      wrapper: createWrapper(),
    });
    await waitFor(() => expect(active.result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/nodes/node-1/admins");
    expect(active.result.current.data).toEqual([{ user_id: "u1" }]);
  });

  it("useNodePendingCredentials unwraps `pending_credentials` and gates on enabled+nodeId", async () => {
    mockGet.mockResolvedValue({ pending_credentials: [{ id: "pc-1" }] });

    // Disabled by the enabled flag.
    const off = renderHook(() => useNodePendingCredentials("node-1", false), {
      wrapper: createWrapper(),
    });
    expect(off.result.current.fetchStatus).toBe("idle");

    // Empty id stays idle.
    const noId = renderHook(() => useNodePendingCredentials("", true), {
      wrapper: createWrapper(),
    });
    expect(noId.result.current.fetchStatus).toBe("idle");
    expect(mockGet).not.toHaveBeenCalled();

    const on = renderHook(() => useNodePendingCredentials("node-1"), {
      wrapper: createWrapper(),
    });
    await waitFor(() => expect(on.result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/nodes/node-1/credentials/pending");
    expect(on.result.current.data).toEqual([{ id: "pc-1" }]);
  });
});

describe("node mutations", () => {
  it("useCreateRegistrationToken POSTs the form data to /nodes/register-token", async () => {
    mockPost.mockResolvedValue({ token: "nyx_nreg_x" });
    const { result } = renderHook(() => useCreateRegistrationToken(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({
      label: "laptop",
    } as unknown as CreateRegistrationTokenFormData);
    expect(mockPost).toHaveBeenCalledWith("/nodes/register-token", {
      label: "laptop",
    });
  });

  it("useDeleteNode DELETEs the specific node", async () => {
    mockDelete.mockResolvedValue(undefined);
    const { result } = renderHook(() => useDeleteNode(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync("node-1");
    expect(mockDelete).toHaveBeenCalledWith("/nodes/node-1");
  });

  it("useRotateNodeToken POSTs to the rotate-token endpoint", async () => {
    mockPost.mockResolvedValue({ token: "nyx_nauth_x" });
    const { result } = renderHook(() => useRotateNodeToken(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync("node-1");
    expect(mockPost).toHaveBeenCalledWith("/nodes/node-1/rotate-token");
  });

  it("useTransferNode POSTs the transfer payload to the node's transfer endpoint", async () => {
    mockPost.mockResolvedValue({ node_id: "node-1" });
    const { result } = renderHook(() => useTransferNode(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({
      nodeId: "node-1",
      data: { new_owner_id: "org-2" } as unknown as TransferNodeFormData,
    });
    expect(mockPost).toHaveBeenCalledWith("/nodes/node-1/transfer", {
      new_owner_id: "org-2",
    });
  });

  it("usePushNodeCredential POSTs to the bound node's credentials/push endpoint", async () => {
    mockPost.mockResolvedValue({ id: "pc-1" });
    const { result } = renderHook(() => usePushNodeCredential("node-1"), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({
      service_slug: "openai",
    } as unknown as PushNodeCredentialFormData);
    expect(mockPost).toHaveBeenCalledWith("/nodes/node-1/credentials/push", {
      service_slug: "openai",
    });
  });

  it("useCancelNodePendingCredential DELETEs the pending credential by id", async () => {
    mockDelete.mockResolvedValue(undefined);
    const { result } = renderHook(
      () => useCancelNodePendingCredential("node-1"),
      { wrapper: createWrapper() },
    );
    await result.current.mutateAsync("pc-1");
    expect(mockDelete).toHaveBeenCalledWith(
      "/nodes/node-1/credentials/pending/pc-1",
    );
  });
});
