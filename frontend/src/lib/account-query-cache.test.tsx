import { createMemoryHistory, createRootRoute, createRoute, createRouter, Outlet, RouterProvider } from "@tanstack/react-router";
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import { QueryClientProvider, useQuery, useQueryClient } from "@tanstack/react-query";
import { afterEach, expect, it, vi } from "vitest";
import { useEffect } from "react";
import { AccountBoundary } from "@/components/account-boundary";
import { useAuthStore } from "@/stores/auth-store";
import { createAccountQueryScope } from "./account-query-cache";
import type { User } from "@/types/api";

afterEach(cleanup);
it.each([null, "A"])("keeps observers and invalidators on the same client after %s -> B", async (from) => {
  useAuthStore.getState().setUser(from ? { id: from } as User : null);
  const scope = createAccountQueryScope({ defaultOptions: { queries: { retry: false } } });
  const initial = scope.getClient();
  const mounted = vi.fn();
  let count = 0;
  function Consumer() {
    const query = useQuery({ queryKey: ["still-mounted"], queryFn: async () => ++count });
    const client = useQueryClient();
    useEffect(() => { mounted(); }, []);
    return <button onClick={() => void client.invalidateQueries({ queryKey: ["still-mounted"] })}>{query.data}</button>;
  }
  const beforeLoad = vi.fn();
  const root = createRootRoute({ component: () => <AccountBoundary><Outlet /></AccountBoundary> });
  const route = createRoute({ getParentRoute: () => root, path: "/", beforeLoad, component: Consumer });
  const router = createRouter({ routeTree: root.addChildren([route]), history: createMemoryHistory({ initialEntries: ["/"] }) });
  function App() {
    useAuthStore((state) => state.user?.id);
    return <QueryClientProvider client={scope.getClient()}><RouterProvider router={router} /></QueryClientProvider>;
  }
  render(<App />);
  await screen.findByRole("button", { name: "1" });
  act(() => useAuthStore.getState().setUser({ id: "B" } as User));
  if (from) expect(scope.getClient()).not.toBe(initial);
  else expect(scope.getClient()).toBe(initial);
  await waitFor(() => expect(screen.getByRole("button")).not.toHaveTextContent("1"));
  const before = count;
  await act(() => scope.getClient().invalidateQueries({ queryKey: ["still-mounted"] }));
  await screen.findByRole("button", { name: String(before + 1) });
  expect(mounted).toHaveBeenCalledTimes(from ? 2 : 1);
  expect(beforeLoad).toHaveBeenCalledOnce();
  scope.dispose();
});
