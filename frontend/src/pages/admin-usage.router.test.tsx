import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createMemoryHistory, createRouter, RouterProvider } from "@tanstack/react-router";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import { router as appRouter } from "@/router";
import { api } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import { usageFixture } from "@/test/admin-usage-fixture";

// Keep the production route tree, guards, search validation, lazy page, query
// hook and router hooks. Only unrelated dashboard chrome and HTTP are replaced.
vi.mock("@/components/layout/dashboard-layout", async () => {
  const { Outlet } = await import("@tanstack/react-router");
  return { DashboardLayout: () => <Outlet /> };
});

const initialAuth = useAuthStore.getState();
afterEach(() => {
  useAuthStore.setState(initialAuth);
  vi.restoreAllMocks();
});

it("reads usage controls from a real admin route and rewrites its URL", async () => {
  useAuthStore.setState({
    isAuthenticated: true,
    isLoading: false,
    user: {
      id: "11111111-1111-4111-8111-111111111111",
      email: "admin@example.test",
      display_name: "Admin",
      avatar_url: null,
      email_verified: true,
      mfa_enabled: false,
      is_admin: true,
      is_active: true,
      created_at: "2026-09-19T00:00:00Z",
    },
  });
  const get = vi.spyOn(api, "get").mockResolvedValue(usageFixture());
  const history = createMemoryHistory({
    initialEntries: ["/admin/usage?period=7d&sort=cost"],
  });
  const router = createRouter({ routeTree: appRouter.routeTree, history });
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const view = render(
    <QueryClientProvider client={client}>
      <RouterProvider router={router} />
    </QueryClientProvider>,
  );
  await act(() => router.load());
  expect(await screen.findByRole("combobox", { name: "Usage period" }))
    .toHaveTextContent("Last 7 days");
  expect(await screen.findByRole("combobox", { name: "Ranking sort" }))
    .toHaveTextContent("Gross cost");
  expect(get).toHaveBeenCalledWith(expect.stringContaining("period=7d&sort=cost"));

  await userEvent.click(screen.getByRole("combobox", { name: "Usage period" }));
  await userEvent.click(screen.getByRole("option", { name: "Last 30 days" }));
  await waitFor(() => {
    expect(history.location.pathname).toBe("/admin/usage");
    const search = new URLSearchParams(history.location.search);
    expect(search.get("period")).toBe("30d");
    expect(search.get("sort")).toBe("cost");
    expect(screen.getByRole("combobox", { name: "Usage period" }))
      .toHaveTextContent("Last 30 days");
  });
  expect(get).toHaveBeenCalledWith(expect.stringContaining("period=30d&sort=cost"));
  view.unmount();
  client.clear();
});
