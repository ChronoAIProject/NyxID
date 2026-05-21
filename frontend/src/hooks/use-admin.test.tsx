import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  useAdminAuditLog,
  useAdminUser,
  useAdminUsers,
  useAdminUserSessions,
  useCreateUser,
  useDeleteUser,
  useForcePasswordReset,
  useRevokeUserSessions,
  useSetUserRole,
  useSetUserStatus,
  useUpdateAdminUser,
  useVerifyUserEmail,
} from "./use-admin";

const { mockGet, mockPost, mockPut, mockPatch, mockDelete } = vi.hoisted(
  () => ({
    mockGet: vi.fn(),
    mockPost: vi.fn(),
    mockPut: vi.fn(),
    mockPatch: vi.fn(),
    mockDelete: vi.fn(),
  }),
);

vi.mock("@/lib/api-client", () => ({
  api: {
    get: mockGet,
    post: mockPost,
    put: mockPut,
    patch: mockPatch,
    delete: mockDelete,
  },
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

describe("admin list / detail queries", () => {
  it("useAdminUsers builds the page/per_page query and omits search when empty", async () => {
    mockGet.mockResolvedValue({ users: [], total: 0 });
    const { result } = renderHook(() => useAdminUsers(2, 25), {
      wrapper: wrapperFactory(),
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/admin/users?page=2&per_page=25");
  });

  it("useAdminUsers appends search when provided", async () => {
    mockGet.mockResolvedValue({ users: [], total: 0 });
    const { result } = renderHook(() => useAdminUsers(1, 10, "ann@x.com"), {
      wrapper: wrapperFactory(),
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith(
      "/admin/users?page=1&per_page=10&search=ann%40x.com",
    );
  });

  it("useAdminUser gates on userId length and fetches by id", async () => {
    mockGet.mockResolvedValue({ id: "u1" });
    const idle = renderHook(() => useAdminUser(""), {
      wrapper: wrapperFactory(),
    });
    expect(idle.result.current.fetchStatus).toBe("idle");
    expect(mockGet).not.toHaveBeenCalled();

    const active = renderHook(() => useAdminUser("u1"), {
      wrapper: wrapperFactory(),
    });
    await waitFor(() => expect(active.result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/admin/users/u1");
  });

  it("useAdminUserSessions gates on userId and fetches the sessions sub-resource", async () => {
    mockGet.mockResolvedValue({ sessions: [] });
    const idle = renderHook(() => useAdminUserSessions(""), {
      wrapper: wrapperFactory(),
    });
    expect(idle.result.current.fetchStatus).toBe("idle");
    expect(mockGet).not.toHaveBeenCalled();

    const active = renderHook(() => useAdminUserSessions("u1"), {
      wrapper: wrapperFactory(),
    });
    await waitFor(() => expect(active.result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/admin/users/u1/sessions");
  });

  it("useAdminAuditLog adds user_id / api_key_id filters when present", async () => {
    mockGet.mockResolvedValue({ entries: [], total: 0 });
    const { result } = renderHook(
      () => useAdminAuditLog(1, 50, { userId: "u1", apiKeyId: "key-2" }),
      { wrapper: wrapperFactory() },
    );
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith(
      "/admin/audit-log?page=1&per_page=50&user_id=u1&api_key_id=key-2",
    );
  });

  it("useAdminAuditLog omits filters when none are supplied", async () => {
    mockGet.mockResolvedValue({ entries: [], total: 0 });
    const { result } = renderHook(() => useAdminAuditLog(1, 50), {
      wrapper: wrapperFactory(),
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/admin/audit-log?page=1&per_page=50");
  });
});

describe("admin user mutations", () => {
  it("useCreateUser POSTs the create body to /admin/users", async () => {
    mockPost.mockResolvedValue({ id: "u1" });
    const { result } = renderHook(() => useCreateUser(), {
      wrapper: wrapperFactory(),
    });
    await result.current.mutateAsync({
      email: "new@x.com",
      password: "pw",
    } as never);
    expect(mockPost).toHaveBeenCalledWith("/admin/users", {
      email: "new@x.com",
      password: "pw",
    });
  });

  it("useUpdateAdminUser PUTs the data body to the user resource", async () => {
    mockPut.mockResolvedValue({ id: "u1" });
    const { result } = renderHook(() => useUpdateAdminUser(), {
      wrapper: wrapperFactory(),
    });
    await result.current.mutateAsync({
      userId: "u1",
      data: { display_name: "Renamed" } as never,
    });
    expect(mockPut).toHaveBeenCalledWith("/admin/users/u1", {
      display_name: "Renamed",
    });
  });

  it("useSetUserRole PATCHes the role to the role sub-resource", async () => {
    mockPatch.mockResolvedValue({ role: "admin" });
    const { result } = renderHook(() => useSetUserRole(), {
      wrapper: wrapperFactory(),
    });
    await result.current.mutateAsync({ userId: "u1", role: "admin" });
    expect(mockPatch).toHaveBeenCalledWith("/admin/users/u1/role", {
      role: "admin",
    });
  });

  it("useSetUserStatus maps isActive -> is_active in the PATCH body", async () => {
    mockPatch.mockResolvedValue({ is_active: false });
    const { result } = renderHook(() => useSetUserStatus(), {
      wrapper: wrapperFactory(),
    });
    await result.current.mutateAsync({ userId: "u1", isActive: false });
    expect(mockPatch).toHaveBeenCalledWith("/admin/users/u1/status", {
      is_active: false,
    });
  });

  it("useForcePasswordReset POSTs to the reset-password sub-resource", async () => {
    mockPost.mockResolvedValue({ message: "ok" });
    const { result } = renderHook(() => useForcePasswordReset(), {
      wrapper: wrapperFactory(),
    });
    await result.current.mutateAsync("u1");
    expect(mockPost).toHaveBeenCalledWith("/admin/users/u1/reset-password");
  });

  it("useDeleteUser DELETEs the user resource", async () => {
    mockDelete.mockResolvedValue({ message: "deleted" });
    const { result } = renderHook(() => useDeleteUser(), {
      wrapper: wrapperFactory(),
    });
    await result.current.mutateAsync("u1");
    expect(mockDelete).toHaveBeenCalledWith("/admin/users/u1");
  });

  it("useVerifyUserEmail PATCHes the verify-email sub-resource", async () => {
    mockPatch.mockResolvedValue({ message: "verified" });
    const { result } = renderHook(() => useVerifyUserEmail(), {
      wrapper: wrapperFactory(),
    });
    await result.current.mutateAsync("u1");
    expect(mockPatch).toHaveBeenCalledWith("/admin/users/u1/verify-email");
  });

  it("useRevokeUserSessions DELETEs the sessions sub-resource", async () => {
    mockDelete.mockResolvedValue({ revoked: 3 });
    const { result } = renderHook(() => useRevokeUserSessions(), {
      wrapper: wrapperFactory(),
    });
    await result.current.mutateAsync("u1");
    expect(mockDelete).toHaveBeenCalledWith("/admin/users/u1/sessions");
  });
});
