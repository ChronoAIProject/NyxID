import { withAccountPanelSearch, validateSettingsSearch } from "@/lib/assistant/account-panel-search";
import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  RouterProvider,
} from "@tanstack/react-router";
import { describe, expect, it, vi } from "vitest";
import { TooltipProvider } from "@/components/ui/tooltip";
import { SETTINGS_TABS } from "@/lib/url-tabs";
import { SettingsPage } from "./settings";

vi.mock("@/hooks/use-auth", () => ({
  useUser: () => ({
    data: {
      display_name: "Reader",
      email: "reader@example.com",
      email_verified: true,
    },
  }),
  useMfaDisable: () => ({ mutateAsync: vi.fn() }),
  useRevokeSession: () => ({ mutateAsync: vi.fn() }),
}));
vi.mock("@/hooks/use-public-config", () => ({
  usePublicConfig: () => ({ data: { mcp_url: "https://nyxid.test/mcp" } }),
}));
vi.mock("@/components/auth/mfa-setup-dialog", () => ({
  MfaSetupDialog: () => null,
}));

async function open(
  route: "/settings" | "/assistant",
  tab = "profile",
) {
  const root = createRootRoute();
  const page = createRoute({
    path: route,
    getParentRoute: () => root,
    validateSearch: route === "/settings" ? validateSettingsSearch : withAccountPanelSearch((search) => ({ c: search.c, mock: search.mock })),
    component: () =>
      route === "/settings" ? <SettingsPage /> : <SettingsPage presentation="panel" />,
  });
  const history = createMemoryHistory({
    initialEntries: [route === "/settings" ? `${route}?tab=${tab}` : `${route}?c=chat&mock=1&panel=settings&panelTab=${tab}`],
  });
  const router = createRouter({ routeTree: root.addChildren([page]), history });
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: Infinity } },
  });
  client.setQueryData(["sessions"], []);
  render(
    <QueryClientProvider client={client}>
      <TooltipProvider>
        <RouterProvider router={router} />
      </TooltipProvider>
    </QueryClientProvider>,
  );
  await act(() => router.load());
  await screen.findByRole("tab", { name: "Profile" });
  return { router, history };
}

describe.each(["/settings", "/assistant"] as const)(
  "SettingsPage at %s",
  (route) => {
    it("replaces tab state on the supplied route for every account tab", async () => {
      const { router, history } = await open(route);
      for (const tab of SETTINGS_TABS) {
        const trigger = screen.getByRole("tab", {
          name:
            tab === "mcp" ? "MCP" : tab.charAt(0).toUpperCase() + tab.slice(1),
        });
        await userEvent.click(trigger);
        expect(router.state.location.pathname).toBe(route);
        expect(router.state.location.search).toEqual(route === "/settings" ? { tab } : { c: "chat", mock: 1, panel: "settings", panelTab: tab });
        expect(trigger).toHaveAttribute("data-state", "active");
        expect(history.length).toBe(1);
        if (tab === "sessions")
          expect(
            screen.getByRole("link", { name: "Generate terminal login code" }),
          ).toHaveAttribute("href", "/login/code");
        if (tab === "privacy")
          expect(
            screen.getByRole("link", { name: "privacy policy" }),
          ).toHaveAttribute("href", "/privacy");
      }
      if (route === "/settings") expect(screen.getByRole("heading", { name: "Account Settings" })).toHaveClass("text-22", "sm:text-28");
      else expect(screen.queryByRole("heading", { name: "Account Settings" })).not.toBeInTheDocument();
    });

    it("defaults invalid tab deep links to Profile", async () => {
      await open(route, "unknown");
      expect(screen.getByRole("tab", { name: "Profile" })).toHaveAttribute(
        "data-state",
        "active",
      );
    });
  },
);
