import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  useAddGroupMember,
  useAssignRole,
  useBulkAssignRole,
  useCreateGroup,
  useCreateRole,
  useDeleteGroup,
  useDeleteRole,
  useGroup,
  useGroupMembers,
  useGroups,
  useRemoveGroupMember,
  useRevokeRole,
  useRole,
  useRoles,
  useUpdateGroup,
  useUpdateRole,
  useUserGroups,
  useUserRoles,
} from "./use-rbac";

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

describe("role queries", () => {
  it("useRoles GETs /admin/roles and returns the raw RoleListResponse", async () => {
    const payload = { roles: [{ id: "role-1", name: "admin" }] };
    mockGet.mockResolvedValue(payload);
    const { result } = renderHook(() => useRoles(), { wrapper: createWrapper() });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/admin/roles");
    expect(result.current.data).toBe(payload);
  });

  it("useRole fetches by id and stays idle for an empty id", async () => {
    mockGet.mockResolvedValue({ id: "role-1" });
    const idle = renderHook(() => useRole(""), { wrapper: createWrapper() });
    expect(idle.result.current.fetchStatus).toBe("idle");
    expect(mockGet).not.toHaveBeenCalled();

    const active = renderHook(() => useRole("role-1"), {
      wrapper: createWrapper(),
    });
    await waitFor(() => expect(active.result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/admin/roles/role-1");
  });
});

describe("role mutations", () => {
  it("useCreateRole POSTs the body to /admin/roles", async () => {
    mockPost.mockResolvedValue({ id: "role-1" });
    const { result } = renderHook(() => useCreateRole(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({
      name: "editor",
      description: "Editors",
    } as never);
    expect(mockPost).toHaveBeenCalledWith("/admin/roles", {
      name: "editor",
      description: "Editors",
    });
  });

  it("useUpdateRole PUTs to /admin/roles/{roleId} with the data", async () => {
    mockPut.mockResolvedValue({ id: "role-1" });
    const { result } = renderHook(() => useUpdateRole(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({
      roleId: "role-1",
      data: { name: "renamed" } as never,
    });
    expect(mockPut).toHaveBeenCalledWith("/admin/roles/role-1", {
      name: "renamed",
    });
  });

  it("useDeleteRole DELETEs /admin/roles/{roleId}", async () => {
    mockDelete.mockResolvedValue(undefined);
    const { result } = renderHook(() => useDeleteRole(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync("role-1");
    expect(mockDelete).toHaveBeenCalledWith("/admin/roles/role-1");
  });
});

describe("role assignment", () => {
  it("useUserRoles GETs /admin/users/{userId}/roles and gates on userId", async () => {
    mockGet.mockResolvedValue({ roles: [] });
    const idle = renderHook(() => useUserRoles(""), {
      wrapper: createWrapper(),
    });
    expect(idle.result.current.fetchStatus).toBe("idle");
    expect(mockGet).not.toHaveBeenCalled();

    const active = renderHook(() => useUserRoles("user-1"), {
      wrapper: createWrapper(),
    });
    await waitFor(() => expect(active.result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/admin/users/user-1/roles");
  });

  it("useAssignRole POSTs to the nested user/role endpoint (no body)", async () => {
    mockPost.mockResolvedValue({ assigned: true });
    const { result } = renderHook(() => useAssignRole(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({ userId: "user-1", roleId: "role-1" });
    expect(mockPost).toHaveBeenCalledWith("/admin/users/user-1/roles/role-1");
  });

  it("useRevokeRole DELETEs the nested user/role endpoint", async () => {
    mockDelete.mockResolvedValue({ assigned: false });
    const { result } = renderHook(() => useRevokeRole(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({ userId: "user-1", roleId: "role-1" });
    expect(mockDelete).toHaveBeenCalledWith("/admin/users/user-1/roles/role-1");
  });

  it("useBulkAssignRole POSTs to /admin/roles/{roleId}/assign-bulk with the data", async () => {
    mockPost.mockResolvedValue({ assigned_count: 2 });
    const { result } = renderHook(() => useBulkAssignRole(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({
      roleId: "role-1",
      data: { user_ids: ["u1", "u2"] } as never,
    });
    expect(mockPost).toHaveBeenCalledWith("/admin/roles/role-1/assign-bulk", {
      user_ids: ["u1", "u2"],
    });
  });
});

describe("group queries", () => {
  it("useGroups GETs /admin/groups", async () => {
    mockGet.mockResolvedValue({ groups: [{ id: "g1" }] });
    const { result } = renderHook(() => useGroups(), {
      wrapper: createWrapper(),
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/admin/groups");
  });

  it("useGroup fetches by id and stays idle for an empty id", async () => {
    mockGet.mockResolvedValue({ id: "g1" });
    const idle = renderHook(() => useGroup(""), { wrapper: createWrapper() });
    expect(idle.result.current.fetchStatus).toBe("idle");
    expect(mockGet).not.toHaveBeenCalled();

    const active = renderHook(() => useGroup("g1"), {
      wrapper: createWrapper(),
    });
    await waitFor(() => expect(active.result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/admin/groups/g1");
  });

  it("useGroupMembers GETs the members sub-resource and gates on groupId", async () => {
    mockGet.mockResolvedValue({ members: [] });
    const idle = renderHook(() => useGroupMembers(""), {
      wrapper: createWrapper(),
    });
    expect(idle.result.current.fetchStatus).toBe("idle");
    expect(mockGet).not.toHaveBeenCalled();

    const active = renderHook(() => useGroupMembers("g1"), {
      wrapper: createWrapper(),
    });
    await waitFor(() => expect(active.result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/admin/groups/g1/members");
  });

  it("useUserGroups GETs /admin/users/{userId}/groups and gates on userId", async () => {
    mockGet.mockResolvedValue({ groups: [] });
    const idle = renderHook(() => useUserGroups(""), {
      wrapper: createWrapper(),
    });
    expect(idle.result.current.fetchStatus).toBe("idle");
    expect(mockGet).not.toHaveBeenCalled();

    const active = renderHook(() => useUserGroups("user-1"), {
      wrapper: createWrapper(),
    });
    await waitFor(() => expect(active.result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/admin/users/user-1/groups");
  });
});

describe("group mutations", () => {
  it("useCreateGroup POSTs the body to /admin/groups", async () => {
    mockPost.mockResolvedValue({ id: "g1" });
    const { result } = renderHook(() => useCreateGroup(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({ name: "team" } as never);
    expect(mockPost).toHaveBeenCalledWith("/admin/groups", { name: "team" });
  });

  it("useUpdateGroup PUTs to /admin/groups/{groupId} with the data", async () => {
    mockPut.mockResolvedValue({ id: "g1" });
    const { result } = renderHook(() => useUpdateGroup(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({
      groupId: "g1",
      data: { name: "renamed" } as never,
    });
    expect(mockPut).toHaveBeenCalledWith("/admin/groups/g1", {
      name: "renamed",
    });
  });

  it("useDeleteGroup DELETEs /admin/groups/{groupId}", async () => {
    mockDelete.mockResolvedValue(undefined);
    const { result } = renderHook(() => useDeleteGroup(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync("g1");
    expect(mockDelete).toHaveBeenCalledWith("/admin/groups/g1");
  });

  it("useAddGroupMember POSTs to the nested group/member endpoint (no body)", async () => {
    mockPost.mockResolvedValue({ added: true });
    const { result } = renderHook(() => useAddGroupMember(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({ groupId: "g1", userId: "user-1" });
    expect(mockPost).toHaveBeenCalledWith("/admin/groups/g1/members/user-1");
  });

  it("useRemoveGroupMember DELETEs the nested group/member endpoint", async () => {
    mockDelete.mockResolvedValue({ added: false });
    const { result } = renderHook(() => useRemoveGroupMember(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({ groupId: "g1", userId: "user-1" });
    expect(mockDelete).toHaveBeenCalledWith("/admin/groups/g1/members/user-1");
  });
});
