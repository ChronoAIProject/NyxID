import type { ReactNode } from "react";
import { render } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  Outlet,
  RouterProvider,
} from "@tanstack/react-router";

export async function renderCrmRoute(
  element: ReactNode,
  entry = "/assistant/crm",
) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  const root = createRootRoute({ component: Outlet });
  const route = createRoute({
    getParentRoute: () => root,
    path: "/assistant/crm",
    validateSearch: (search: Record<string, unknown>) => search,
    component: () => element,
  });
  const history = createMemoryHistory({ initialEntries: [entry] });
  const router = createRouter({
    routeTree: root.addChildren([route]),
    history,
  });
  await router.load();
  const rendered = render(
    <QueryClientProvider client={client}>
      <RouterProvider router={router} />
    </QueryClientProvider>,
  );
  return { ...rendered, router, history, client };
}
