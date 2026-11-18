import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import { QueryClientProvider } from "@tanstack/react-query";
import { createMemoryHistory, createRouter, RouterProvider } from "@tanstack/react-router";
import { afterEach, expect, it, vi } from "vitest";
import { router as appRouter } from "@/router";
import { api } from "@/lib/api-client";
import { createAccountQueryScope } from "@/lib/account-query-cache";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";

vi.mock("@/components/layout/dashboard-layout", async () => {
  const { Outlet } = await import("@tanstack/react-router");
  return { DashboardLayout: () => <Outlet /> };
});
vi.mock("@/components/assistant/cma-session-card", () => ({ FloatingCmaSessions: () => null }));
vi.mock("@/pages/dashboard", () => ({ DashboardPage: () => <p>Member dashboard</p> }));
const admin = { id: "A", email: "admin@example.test", is_admin: true } as User;
const member = { id: "B", email: "member@example.test", is_admin: false } as User;
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
it("admits a switched member exactly like a fresh navigation, before admin children dispatch", async () => {
  useAuthStore.getState().setUser(member);
  useAuthStore.setState({ isLoading: false });
  const fresh = createRouter({ routeTree: appRouter.routeTree, history: createMemoryHistory({ initialEntries: ["/admin/users"] }) });
  await fresh.load();
  await vi.waitFor(() => expect(fresh.state.location.pathname).toBe("/dashboard"));
  useAuthStore.getState().setUser(admin);
  const scope = createAccountQueryScope({ defaultOptions: { queries: { retry: false } } });
  const calls: Array<{ actor?: string; path: string }> = [];
  vi.spyOn(api, "get").mockImplementation(async (path) => {
    calls.push({ actor: useAuthStore.getState().user?.id, path });
    return { users: [], total: 0, page: 1, per_page: 20 };
  });
  const router = createRouter({ routeTree: appRouter.routeTree, history: createMemoryHistory({ initialEntries: ["/admin/users"] }) });
  function App() {
    useAuthStore((state) => state.user);
    return <QueryClientProvider client={scope.getClient()}><RouterProvider router={router} /></QueryClientProvider>;
  }
  const view = render(<App />);
  try {
    await act(() => router.load());
    await screen.findByRole("heading", { name: "User Management" });
    expect(calls.some((call) => call.actor === "A" && call.path.startsWith("/admin/users?"))).toBe(true);
    const search = screen.getByPlaceholderText(/Search by email/i);
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(Response.json(admin)));
    await act(() => useAuthStore.getState().verifySession());
    expect(screen.getByPlaceholderText(/Search by email/i)).toBe(search);
    expect(router.state.location.pathname).toBe("/admin/users");
    act(() => useAuthStore.getState().setUser(member));
    await waitFor(() => expect(router.state.location.pathname).toBe(fresh.state.location.pathname));
    expect(calls.filter((call) => call.actor === "B" && call.path.startsWith("/admin/users"))).toEqual([]);
    expect(screen.queryByRole("heading", { name: "User Management" })).toBeNull();
  } finally { view.unmount(); scope.dispose(); }
});
