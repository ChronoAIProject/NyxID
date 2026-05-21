import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  useCancelInvite,
  useCreateInvite,
  useOrgInvites,
} from "./use-org-invites";

const { mockGet, mockPost, mockDelete } = vi.hoisted(() => ({
  mockGet: vi.fn(),
  mockPost: vi.fn(),
  mockDelete: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: { get: mockGet, post: mockPost, delete: mockDelete },
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

describe("useOrgInvites", () => {
  it("unwraps the `invites` array for a given org", async () => {
    mockGet.mockResolvedValue({ invites: [{ id: "inv-1" }] });
    const { result } = renderHook(() => useOrgInvites("org-1"), {
      wrapper: wrapperFactory(),
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/orgs/org-1/invites");
    expect(result.current.data).toEqual([{ id: "inv-1" }]);
  });

  it("is disabled when the org id is empty", () => {
    const { result } = renderHook(() => useOrgInvites(""), {
      wrapper: wrapperFactory(),
    });
    expect(result.current.fetchStatus).toBe("idle");
    expect(mockGet).not.toHaveBeenCalled();
  });
});

describe("useCreateInvite", () => {
  it("POSTs the invite body to the org's invites collection", async () => {
    mockPost.mockResolvedValue({ id: "inv-1", nonce: "ORGINV-X" });
    const { result } = renderHook(() => useCreateInvite(), {
      wrapper: wrapperFactory(),
    });
    await result.current.mutateAsync({
      orgId: "org-1",
      body: { role: "member", allowed_service_ids: ["svc-1"] },
    });
    expect(mockPost).toHaveBeenCalledWith("/orgs/org-1/invites", {
      role: "member",
      allowed_service_ids: ["svc-1"],
    });
  });
});

describe("useCancelInvite", () => {
  it("DELETEs the specific invite under the org", async () => {
    mockDelete.mockResolvedValue(undefined);
    const { result } = renderHook(() => useCancelInvite(), {
      wrapper: wrapperFactory(),
    });
    await result.current.mutateAsync({ orgId: "org-1", inviteId: "inv-1" });
    expect(mockDelete).toHaveBeenCalledWith("/orgs/org-1/invites/inv-1");
  });
});
