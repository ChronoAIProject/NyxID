import { describe, expect, it } from "vitest";
import { router } from "@/router";
import { buildStudioBreadcrumbs } from "./studio-breadcrumbs";

const REDIRECTS = new Set([
  "/api-keys",
  "/connections",
  "/settings/devices/onboard",
  "/guide",
  "/settings/authorizations",
  "/admin/analytics",
]);

const studioRoutes = Object.values(router.routesById).filter(
  (route) =>
    route.id.startsWith("/dashboard/") && !REDIRECTS.has(route.fullPath),
);

describe("Studio breadcrumbs", () => {
  it.each(studioRoutes.map((route) => [route.fullPath] as const))(
    "covers the mounted route %s with valid parent links",
    (fullPath) => {
      const path = fullPath.replace(/\$[^/]+/g, "entity-1");
      const crumbs = buildStudioBreadcrumbs(path);
      expect(crumbs.length).toBeGreaterThan(0);
      expect(crumbs.at(-1)?.to).toBeUndefined();
      for (const crumb of crumbs) {
        expect(crumb.label).not.toContain("entity-1");
        if (crumb.to) {
          expect(
            router.getMatchedRoutes(crumb.to).foundRoute,
            crumb.to,
          ).toBeDefined();
        }
      }
    },
  );

  it("checks every Studio route instead of an empty route list", () => {
    expect(studioRoutes.length).toBeGreaterThan(50);
  });

  it("returns organization children to the correct tab", () => {
    const path = "/orgs/org-1/service-accounts/worker-1";
    expect(
      buildStudioBreadcrumbs(path, {
        "/orgs/org-1": "Engineering",
        [path]: "Deploy worker",
      }),
    ).toEqual([
      { label: "Organizations", to: "/orgs" },
      { label: "Engineering", to: "/orgs/org-1", search: { tab: "members" } },
      {
        label: "Service Accounts",
        to: "/orgs/org-1",
        search: { tab: "service-accounts" },
      },
      { label: "Deploy worker" },
    ]);
    expect(
      buildStudioBreadcrumbs("/orgs/org-1/developer-apps/app-1")[2],
    ).toMatchObject({
      to: "/orgs/org-1",
      search: { tab: "developer-apps" },
    });
    const parent = buildStudioBreadcrumbs(path)[2]!;
    const location = router.buildLocation({
      to: "/orgs/$orgId",
      params: { orgId: "org-1" },
      search: { tab: parent.search?.tab },
    });
    expect(location.pathname).toBe(parent.to);
    expect(location.pathname).toBe("/orgs/org-1");
    expect(location.search).toMatchObject({ tab: "service-accounts" });
  });

  it("returns a service overview to the AI Services tab with its name", () => {
    expect(
      buildStudioBreadcrumbs("/keys/services/catalog:cat-1", {
        "/keys/services/catalog:cat-1": "OpenAI",
      }),
    ).toEqual([
      { label: expect.any(String), to: "/keys", search: expect.any(Object) },
      { label: expect.any(String), to: "/keys", search: { tab: "services" } },
      { label: "OpenAI" },
    ]);
  });

  it("returns agent keys to the Agent Keys tab", () => {
    expect(buildStudioBreadcrumbs("/keys/api-key/key-1")[1]).toEqual({
      label: "Agent Keys",
      to: "/keys",
      search: { tab: "nyxid" },
    });
    const parent = buildStudioBreadcrumbs("/keys/api-key/key-1")[1]!;
    const location = router.buildLocation({
      to: "/keys",
      search: { tab: parent.search?.tab },
    });
    expect(location.pathname).toBe(parent.to);
    expect(location.pathname).toBe("/keys");
    expect(location.search).toMatchObject({ tab: "nyxid" });
  });

  it.each(["services", "providers"])(
    "keeps a named parent when editing %s",
    (section) => {
      const path = `/${section}/item-1/edit`;
      const crumbs = buildStudioBreadcrumbs(path, { [path]: "Production" });
      expect(crumbs[1]).toEqual({
        label: "Production",
        to: `/${section}/item-1`,
      });
      expect(crumbs.at(-1)).toEqual({ label: "Edit" });
    },
  );

  it("keeps the channel bot parent on conversation pages", () => {
    expect(
      buildStudioBreadcrumbs("/channel-bots/bot-1/conversations/chat-1", {
        "/channel-bots/bot-1": "Support",
      }),
    ).toEqual([
      { label: "Channel Bots", to: "/channel-bots" },
      { label: "Support", to: "/channel-bots/bot-1" },
      { label: "Messages" },
    ]);
  });

  it("does not reuse a different entity's label or expose its identifier", () => {
    expect(
      buildStudioBreadcrumbs("/keys/new-key", {
        "/keys/old-key": "Old credential",
      }),
    ).toEqual([
      {
        label: "Services & Credentials",
        to: "/keys",
        search: { tab: "services" },
      },
      { label: "External Services", to: "/keys", search: { tab: "services" } },
      { label: "Credential", to: "/keys/new-key", search: { tab: "overview" } },
      { label: "Overview" },
    ]);
  });

  it.each([
    ["/settings", "profile", "Profile"],
    ["/settings", "security", "Security"],
    ["/settings", "sessions", "Sessions"],
    ["/settings", "mcp", "MCP"],
    ["/settings", "display", "Display"],
    ["/settings", "privacy", "Privacy"],
    ["/settings/consents", "apps", "Authorized Apps"],
    ["/settings/consents", "authorizations", "Authorizations"],
    ["/keys", "services", "External Services"],
    ["/keys", "pools", "Service Pools"],
    ["/keys", "nyxid", "Agent Keys"],
    ["/keys/key-1", "overview", "Overview"],
    ["/keys/key-1", "advanced", "Advanced"],
    ["/keys/key-1", "history", "History"],
    ["/orgs/org-1", "members", "Members"],
    ["/orgs/org-1", "role-permissions", "Role permissions"],
    ["/orgs/org-1", "invites", "Invites"],
    ["/orgs/org-1", "approvals", "Approvals"],
    ["/orgs/org-1", "service-accounts", "Service Accounts"],
    ["/orgs/org-1", "developer-apps", "Developer Apps"],
    ["/orgs/org-1", "settings", "Settings"],
    ["/integration-guide", "react", "React SDK"],
    ["/integration-guide", "core", "Core SDK"],
    ["/integration-guide", "raw", "Raw API"],
    ["/billing", "billing", "Billing"],
    ["/billing", "usage", "Usage"],
    ["/admin/usage", "dashboard", "Dashboard"],
    ["/admin/usage", "list", "List"],
    ["/admin/credits", "grants", "Credit grants"],
    ["/admin/credits", "allowances", "Free allowances"],
    ["/admin/credits", "schedules", "Schedules"],
  ])("shows the selected subsection at %s?tab=%s", (path, tab, label) => {
    const crumbs = buildStudioBreadcrumbs(path, {}, { tab });
    expect(crumbs.at(-1)).toEqual({ label });
    expect(crumbs.slice(0, -1).every((crumb) => crumb.to)).toBe(true);
  });

  it.each([
    ["codes", "Codes"],
    ["users", "By User"],
  ])("uses view=%s for invite codes", (view, label) => {
    expect(
      buildStudioBreadcrumbs("/admin/invite-codes", {}, { view }).at(-1),
    ).toEqual({ label });
  });

  it.each([undefined, "unknown", ["display"], 42, null])(
    "uses the page default for invalid tab %s",
    (tab) => {
      expect(buildStudioBreadcrumbs("/settings", {}, { tab }).at(-1)).toEqual({
        label: "Profile",
      });
    },
  );

  it("uses the visible credential tab when a requested tab is unavailable", () => {
    expect(
      buildStudioBreadcrumbs(
        "/keys/key-1",
        {},
        { tab: "advanced" },
        "overview",
      ).at(-1),
    ).toEqual({ label: "Overview" });
  });

  it("returns device binding to Security without forwarding a user code", () => {
    expect(
      buildStudioBreadcrumbs(
        "/settings/devices/bind",
        {},
        { user_code: "ABCD-EFGH" },
      ),
    ).toEqual([
      {
        label: "Account Settings",
        to: "/settings",
        search: { tab: "profile" },
      },
      { label: "Security", to: "/settings", search: { tab: "security" } },
      { label: "Bind device" },
    ]);
  });

  it("links channel setup and connection status to their accessible parents", () => {
    expect(buildStudioBreadcrumbs("/channel-bots/connect")).toEqual([
      { label: "Channel Bots", to: "/channel-bots" },
      { label: "Setup links" },
    ]);
    expect(buildStudioBreadcrumbs("/providers/callback")[0]).toEqual({
      label: "Services & Credentials",
      to: "/keys",
      search: { tab: "services" },
    });
  });

  it("names both selected AI setup contexts without exposing invalid query values", () => {
    expect(
      buildStudioBreadcrumbs(
        "/ai-setup",
        {},
        { skill: "codex", tool: "cursor" },
      ).at(-1),
    ).toEqual({ label: "Codex skills · Cursor MCP" });
    expect(
      buildStudioBreadcrumbs("/ai-setup", {}, { skill: "unknown", tool: [] }),
    ).toEqual([{ label: "AI Setup Guide" }]);
  });
});
