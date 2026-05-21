import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { PropsWithChildren } from "react";
import {
  useAddMember,
  useOrgMembers,
  useRemoveMember,
  useUpdateMember,
} from "./use-org-members";

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

describe("useOrgMembers", () => {
  it("unwraps the `members` array and gates on orgId", async () => {
    mockGet.mockResolvedValue({ members: [{ id: "m1" }] });

    const idle = renderHook(() => useOrgMembers(""), {
      wrapper: createWrapper(),
    });
    expect(idle.result.current.fetchStatus).toBe("idle");
    expect(mockGet).not.toHaveBeenCalled();

    const active = renderHook(() => useOrgMembers("org-1"), {
      wrapper: createWrapper(),
    });
    await waitFor(() => expect(active.result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/orgs/org-1/members");
    expect(active.result.current.data).toEqual([{ id: "m1" }]);
  });
});

describe("org member mutations", () => {
  it("useAddMember POSTs the body to the org members collection", async () => {
    mockPost.mockResolvedValue({ id: "m1" });
    const { result } = renderHook(() => useAddMember(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({
      orgId: "org-1",
      body: { user_id: "u-1", role: "member" },
    });
    expect(mockPost).toHaveBeenCalledWith("/orgs/org-1/members", {
      user_id: "u-1",
      role: "member",
    });
  });

  it("useUpdateMember PATCHes the specific member with the body", async () => {
    mockPatch.mockResolvedValue({ id: "m1", role: "admin" });
    const { result } = renderHook(() => useUpdateMember(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({
      orgId: "org-1",
      memberId: "m1",
      body: { role: "admin" },
    });
    expect(mockPatch).toHaveBeenCalledWith("/orgs/org-1/members/m1", {
      role: "admin",
    });
  });

  it("useRemoveMember DELETEs the specific member", async () => {
    mockDelete.mockResolvedValue(undefined);
    const { result } = renderHook(() => useRemoveMember(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({ orgId: "org-1", memberId: "m1" });
    expect(mockDelete).toHaveBeenCalledWith("/orgs/org-1/members/m1");
  });
});
