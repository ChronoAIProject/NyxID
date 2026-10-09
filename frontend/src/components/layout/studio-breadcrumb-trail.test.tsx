import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  RouterProvider,
} from "@tanstack/react-router";
import { describe, expect, it } from "vitest";
import { router } from "@/router";
import { buildStudioBreadcrumbs } from "@/lib/studio-breadcrumbs";
import { StudioBreadcrumbTrail } from "./studio-breadcrumb-trail";

async function renderTrail(path: string, search: Record<string, unknown>) {
  const crumbs = buildStudioBreadcrumbs(path, {}, search);
  const root = createRootRoute({
    component: () => <StudioBreadcrumbTrail crumbs={crumbs} />,
  });
  const paths = [
    "/settings",
    "/settings/consents",
    "/keys",
    "/keys/$keyId",
    "/orgs",
    "/orgs/$orgId",
    "/billing",
    "/admin/usage",
    "/admin/invite-codes",
    "/admin/credits",
    "/integration-guide",
  ];
  const routes = paths.map((fullPath) => {
    const source = Object.values(router.routesById).find(
      (route) => route.fullPath === fullPath,
    )!;
    return createRoute({
      getParentRoute: () => root,
      path: fullPath,
      validateSearch: source.options.validateSearch,
    });
  });
  const testRouter = createRouter({
    routeTree: root.addChildren(routes),
    history: createMemoryHistory({
      initialEntries: ["/?action=create-key&user_code=ABCD-EFGH"],
    }),
  });
  await testRouter.load();
  render(<RouterProvider router={testRouter} />);
  return testRouter;
}

describe("Studio breadcrumb links", () => {
  it.each([
    [
      "/settings",
      { tab: "display" },
      "Account Settings",
      "/settings",
      "tab",
      "profile",
    ],
    [
      "/settings/consents",
      { tab: "authorizations" },
      "Access & Authorizations",
      "/settings/consents",
      "tab",
      "apps",
    ],
    [
      "/integration-guide",
      { tab: "raw" },
      "Integration & SDK Guide",
      "/integration-guide",
      "tab",
      "react",
    ],
    ["/keys/api-key/key-1", {}, "Agent Keys", "/keys", "tab", "nyxid"],
    [
      "/keys/key-1",
      { tab: "advanced" },
      "External Services",
      "/keys",
      "tab",
      "services",
    ],
    [
      "/keys/key-1",
      { tab: "advanced" },
      "Credential",
      "/keys/key-1",
      "tab",
      "overview",
    ],
    [
      "/orgs/org-1/service-accounts/sa-1",
      {},
      "Service Accounts",
      "/orgs/org-1",
      "tab",
      "service-accounts",
    ],
    [
      "/orgs/org-1/developer-apps/app-1",
      {},
      "Developer Apps",
      "/orgs/org-1",
      "tab",
      "developer-apps",
    ],
    ["/settings/devices/bind", {}, "Security", "/settings", "tab", "security"],
    [
      "/billing",
      { tab: "usage" },
      "Billing & Usage",
      "/billing",
      "tab",
      "billing",
    ],
    [
      "/admin/usage",
      { tab: "list" },
      "Usage",
      "/admin/usage",
      "tab",
      "dashboard",
    ],
    [
      "/admin/invite-codes",
      { view: "users" },
      "Invite Codes",
      "/admin/invite-codes",
      "view",
      "codes",
    ],
    [
      "/admin/credits",
      { tab: "schedules" },
      "Credits",
      "/admin/credits",
      "tab",
      "grants",
    ],
  ] as const)(
    "navigates from %s via %s",
    async (path, search, label, target, key, value) => {
      const user = userEvent.setup();
      const testRouter = await renderTrail(path, search);
      const link = await screen.findByRole("link", { name: label });
      const href = new URL(link.getAttribute("href")!, "http://localhost");
      expect(href.pathname).toBe(target);
      expect([...href.searchParams]).toEqual([[key, value]]);

      await user.click(link);
      await waitFor(() =>
        expect(testRouter.state.location.pathname).toBe(target),
      );
      expect(testRouter.state.location.search).toMatchObject({ [key]: value });
      expect(testRouter.state.location.search).not.toHaveProperty("action");
      expect(testRouter.state.location.search).not.toHaveProperty("user_code");
      expect(
        screen.getByText(
          buildStudioBreadcrumbs(path, {}, search).at(-1)!.label,
        ),
      ).toHaveAttribute("aria-current", "page");
    },
  );
});
