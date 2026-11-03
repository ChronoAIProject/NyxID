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
  apiClient: mockGet,
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

  it("useAdminAuditLog forwards every applied table control", async () => {
    mockGet.mockResolvedValue({ entries: [], total: 0 });
    const { result } = renderHook(
      () =>
        useAdminAuditLog({
          page: 2,
          per_page: 50,
          sort: "event_type",
          user_id: "exact-user",
          api_key_id: "exact-key",
          search: "mfa",
          search_filters: '[{"field":"user_id","values":["u1"]}]',
          custom_filters: '{"event_type":["login"]}',
          event_type: "admin.user.created",
          status: "4xx,5xx",
          actor: "agent",
          created_from: "2026-07-01",
          created_to: "2026-07-13",
        }),
      { wrapper: wrapperFactory() },
    );
    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    const url = new URL(
      `https://example.test${(mockGet.mock.calls[0] as [string])[0]}`,
    );
    expect(Object.fromEntries(url.searchParams)).toEqual({
      page: "2",
      per_page: "50",
      sort: "event_type",
      include_total: "false",
      user_id: "exact-user",
      api_key_id: "exact-key",
      search: "mfa",
      search_filters: '[{"field":"user_id","values":["u1"]}]',
      custom_filters: '{"event_type":["login"]}',
      event_type: "admin.user.created",
      status: "4xx,5xx",
      actor: "agent",
      created_from: "2026-07-01",
      created_to: "2026-07-13",
    });
  });

  it("useAdminAuditLog omits filters when none are supplied", async () => {
    mockGet.mockResolvedValue({ entries: [], total: 0 });
    const { result } = renderHook(
      () => useAdminAuditLog({ page: 1, per_page: 25, sort: "-created_at" }),
      { wrapper: wrapperFactory() },
    );
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith(
      "/admin/audit-log?page=1&per_page=25&sort=-created_at&include_total=false",
      { signal: expect.any(AbortSignal) },
    );
    expect(mockGet).toHaveBeenCalledTimes(1);
  });

  it("uses an older backend's total without requesting the new count route", async () => {
    mockGet.mockResolvedValue({
      entries: [{ id: "legacy-entry" }],
      total: 75,
      page: 2,
      per_page: 25,
    });
    const { result } = renderHook(
      () =>
        useAdminAuditLog({
          page: 2,
          per_page: 25,
          sort: "-created_at",
          user_id: "exact-user",
          search_filters: '[{"field":"event_type","values":["login"]}]',
        }),
      { wrapper: wrapperFactory() },
    );
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(result.current.total).toBe(75);
    expect(result.current.data?.entries).toEqual([{ id: "legacy-entry" }]);
    expect(result.current.totalError).toBeNull();
    expect(mockGet).toHaveBeenCalledTimes(1);
    const query = new URL(`https://example.test${mockGet.mock.calls[0]![0]}`)
      .searchParams;
    expect(query.get("user_id")).toBe("exact-user");
    expect(query.get("search_filters")).toBe(
      '[{"field":"event_type","values":["login"]}]',
    );
  });

  it("shows rows before the total and reuses that total across pages", async () => {
    let resolveCount!: (value: { total: number }) => void;
    const count = new Promise<{ total: number }>((resolve) => {
      resolveCount = resolve;
    });
    mockGet.mockImplementation((path: string) => {
      if (path.startsWith("/admin/audit-log/count?")) return count;
      const query = new URL(`https://example.test${path}`).searchParams;
      return Promise.resolve({
        entries: [{ id: "audit-1" }],
        has_more: true,
        page: Number(query.get("page")),
        per_page: 25,
      });
    });
    const { result, rerender } = renderHook(
      ({ page, sort }) =>
        useAdminAuditLog({ page, per_page: 25, sort, user_id: "exact-user" }),
      {
        wrapper: wrapperFactory(),
        initialProps: { page: 1, sort: "-created_at" as const },
      },
    );
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(result.current.data?.entries).toEqual([{ id: "audit-1" }]);
    expect(result.current.total).toBeUndefined();
    await waitFor(() =>
      expect(mockGet).toHaveBeenCalledWith(
        "/admin/audit-log/count?user_id=exact-user",
        { signal: expect.any(AbortSignal) },
      ),
    );
    resolveCount({ total: 100 });
    await waitFor(() => expect(result.current.total).toBe(100));
    rerender({ page: 2, sort: "-created_at" });
    await waitFor(() => expect(result.current.data?.page).toBe(2));
    expect(
      mockGet.mock.calls.filter(([path]) =>
        (path as string).startsWith("/admin/audit-log/count?"),
      ),
    ).toHaveLength(1);
  });

  it("keeps a successful page usable if its independent total fails", async () => {
    mockGet.mockImplementation((path: string) =>
      path.startsWith("/admin/audit-log/count?")
        ? Promise.reject(new Error("count unavailable"))
        : Promise.resolve({ entries: [{ id: "audit-1" }], has_more: true }),
    );
    const { result } = renderHook(
      () =>
        useAdminAuditLog({
          page: 1,
          per_page: 25,
          sort: "-created_at",
          search: "proxy",
        }),
      { wrapper: wrapperFactory() },
    );
    await waitFor(() =>
      expect(result.current.totalError?.message).toBe("count unavailable"),
    );
    expect(result.current.isSuccess).toBe(true);
    expect(result.current.error).toBeNull();
    expect(result.current.data?.has_more).toBe(true);
  });

  it("keeps rows and refresh usable if an older replica lacks the count route", async () => {
    let legacyResponse = false;
    const missingRoute = Object.assign(new Error("Not Found"), { status: 404 });
    mockGet.mockImplementation((path: string) => {
      if (path.startsWith("/admin/audit-log/count?")) {
        return Promise.reject(missingRoute);
      }
      return Promise.resolve({
        entries: [{ id: "audit-1" }],
        has_more: !legacyResponse,
        ...(legacyResponse ? { total: 1 } : {}),
      });
    });
    const { result } = renderHook(
      () =>
        useAdminAuditLog({
          page: 1,
          per_page: 25,
          sort: "-created_at",
          search: "proxy",
        }),
      { wrapper: wrapperFactory() },
    );
    await waitFor(() => expect(result.current.totalError).toBe(missingRoute));
    expect(result.current.isSuccess).toBe(true);
    expect(result.current.error).toBeNull();
    expect(result.current.data?.entries).toEqual([{ id: "audit-1" }]);
    expect(result.current.data?.has_more).toBe(true);

    legacyResponse = true;
    await result.current.refetch();
    await waitFor(() => expect(result.current.total).toBe(1));
    expect(result.current.totalError).toBeNull();
    expect(result.current.isSuccess).toBe(true);
  });

  it("cancels a superseded page request and an unmounted count request", async () => {
    mockGet.mockImplementation(() => new Promise(() => {}));
    const pending = renderHook(
      ({ search }) =>
        useAdminAuditLog({
          page: 1,
          per_page: 25,
          sort: "-created_at",
          search,
        }),
      { wrapper: wrapperFactory(), initialProps: { search: "old" } },
    );
    const pageSignal = (
      mockGet.mock.calls.at(-1)![1] as { signal: AbortSignal }
    ).signal;
    pending.rerender({ search: "new" });
    expect(pageSignal.aborted).toBe(true);
    pending.unmount();

    mockGet.mockImplementation((path: string) =>
      path.startsWith("/admin/audit-log/count?")
        ? new Promise(() => {})
        : Promise.resolve({ entries: [], has_more: true }),
    );
    const counted = renderHook(
      () =>
        useAdminAuditLog({
          page: 1,
          per_page: 25,
          sort: "-created_at",
          search: "proxy",
        }),
      { wrapper: wrapperFactory() },
    );
    await waitFor(() =>
      expect(mockGet.mock.calls.at(-1)![0] as string).toContain("/count?"),
    );
    const countSignal = (
      mockGet.mock.calls.at(-1)![1] as { signal: AbortSignal }
    ).signal;
    counted.unmount();
    expect(countSignal.aborted).toBe(true);
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
