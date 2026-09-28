import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactNode } from "react";
import type { KeyInfo } from "@/types/keys";

const { state } = vi.hoisted(() => ({
  state: {
    groupId: "catalog:openai",
    keys: [] as KeyInfo[],
    error: null as unknown,
  },
}));
vi.mock("@tanstack/react-router", () => ({
  useParams: () => ({ groupId: state.groupId }),
  Link: ({
    to,
    params,
    children,
    ...props
  }: {
    to: string;
    params?: { keyId?: string };
    "aria-label"?: string;
    children: ReactNode;
  }) => (
    <a
      aria-label={props["aria-label"]}
      href={params?.keyId ? `/keys/${params.keyId}` : to}
    >
      {children}
    </a>
  ),
}));
vi.mock("@/components/layout/dashboard-layout", () => ({
  useBreadcrumbLabel: vi.fn(),
}));
vi.mock("@/hooks/use-keys", () => ({
  useKeys: () => ({ data: state.keys, error: state.error, refetch: vi.fn() }),
  useCatalog: () => ({
    data: [
      {
        slug: "openai",
        name: "OpenAI",
        description: "Service description",
        documentation_url: "https://openai.example/docs",
      },
    ],
    refetch: vi.fn(),
  }),
}));
vi.mock("@/hooks/use-user-services", () => ({
  useUserServices: () => ({ data: [] }),
}));
vi.mock("@/hooks/use-nodes", () => ({ useNodes: () => ({ data: [] }) }));
vi.mock("@/hooks/use-service-insights", () => ({
  useServiceInsights: () => ({
    connections: new Map(),
    status: "unavailable",
    refresh: vi.fn(),
  }),
}));
vi.mock("@/components/dashboard/service-history", () => ({
  ServiceHistory: ({ serviceId }: { serviceId: string }) => (
    <div data-testid="service-history">{serviceId}</div>
  ),
  ServiceAuthorshipFooter: () => null,
}));
import { ServiceOverviewPage } from "./service-overview";

function key(id: string, overrides: Partial<KeyInfo> = {}): KeyInfo {
  return {
    id,
    label: id,
    slug: id,
    catalog_service_id: "openai",
    catalog_service_slug: "openai",
    catalog_service_name: "OpenAI",
    credential_source: { type: "personal" },
    service_type: "http",
    is_active: true,
    status: "active",
    auth_method: "bearer",
    credential_type: "api_key",
    api_key_id: id,
    node_id: null,
    auto_connected: false,
    created_at: "2026-01-01",
    granted_scopes: ["models:read", "chat:write"],
    ws_frame_injections: [],
    ...overrides,
  } as KeyInfo;
}
beforeEach(() => {
  state.groupId = "catalog:openai";
  state.error = null;
  state.keys = [
    key("Development"),
    key("Team", {
      source_app_name: "Provisioning app",
      last_used_at: "2026-09-01",
    }),
    key("Custom", { catalog_service_id: null }),
  ];
});
afterEach(cleanup);

describe("full service page", () => {
  it("contains all connections for the service with full information and configuration links", async () => {
    const user = userEvent.setup();
    render(<ServiceOverviewPage />);
    expect(screen.getByRole("heading", { name: "OpenAI" })).toBeVisible();
    expect(screen.getByText("2 connections")).toBeVisible();
    expect(
      screen.getByRole("link", {
        name: "Configure Development (Personal)",
      }),
    ).toHaveAttribute("href", "/keys/Development");
    expect(
      screen.queryByRole("link", { name: /Configure Custom/ }),
    ).not.toBeInTheDocument();
    expect(screen.queryByTestId("service-history")).not.toBeInTheDocument();
    await user.click(
      screen.getByRole("button", { name: "Details for Team (Personal)" }),
    );
    const info = screen.getByRole("table", { name: "OpenAI connections" });
    expect(within(info).getByText("models:read, chat:write")).toBeVisible();
    expect(within(info).getByText("Provisioning app")).toBeVisible();
    expect(within(info).getByText("Last caller")).toBeVisible();
    expect(
      within(info).getByRole("link", { name: "Configure Team (Personal)" }),
    ).toHaveAttribute("href", "/keys/Team");
    expect(
      screen.getByRole("link", { name: "Service documentation" }),
    ).toHaveAttribute("href", "https://openai.example/docs");
  });

  it.each(["member", "viewer"] as const)(
    "lets an org %s inspect metadata and history without showing private configuration",
    async (role) => {
      const user = userEvent.setup();
      state.keys = [
        key("Shared", {
          endpoint_url: "https://private.internal/v1",
          openapi_spec_url: "https://private.internal/spec",
          custom_user_agent: "private-client",
          credential_source: {
            type: "org",
            org_id: "org",
            org_name: "ChronoAI",
            role,
            allowed: role === "member",
          },
        }),
      ];
      render(<ServiceOverviewPage />);
      expect(screen.getByText("Editors only")).toBeVisible();
      expect(
        screen.getByRole("link", {
          name: "View Shared connection details (ChronoAI)",
        }),
      ).toHaveAttribute("href", "/keys/Shared");
      expect(
        screen.queryByRole("link", { name: /Configure/ }),
      ).not.toBeInTheDocument();
      await user.click(
        screen.getByRole("button", { name: "Details for Shared (ChronoAI)" }),
      );
      expect(document.body.innerHTML).not.toContain("private.internal");
      expect(document.body.innerHTML).not.toContain("private-client");
      expect(document.body.innerHTML).not.toContain("models:read");
      expect(screen.getByText("Connection ID")).toBeVisible();
      await user.click(
        screen.getByRole("button", { name: "History for Shared (ChronoAI)" }),
      );
      expect(screen.getByRole("tab", { name: "History" })).toHaveAttribute(
        "aria-selected",
        "true",
      );
      expect(screen.getByTestId("service-history")).toHaveTextContent("Shared");
    },
  );

  it("honors an explicit server denial even when cached provenance says admin", async () => {
    state.keys = [
      key("Restricted", {
        can_edit_configuration: false,
        endpoint_url: "https://private.internal",
      }),
    ];
    render(<ServiceOverviewPage />);
    expect(
      screen.queryByRole("link", { name: /Configure/ }),
    ).not.toBeInTheDocument();
    expect(document.body.innerHTML).not.toContain("private.internal");
    expect(
      screen.getByRole("button", { name: "History for Restricted (Personal)" }),
    ).toBeVisible();
  });

  it("loads history for the selected connection only", async () => {
    const user = userEvent.setup();
    render(<ServiceOverviewPage />);
    await user.click(screen.getByRole("tab", { name: "History" }));
    expect(screen.getByTestId("service-history")).toHaveTextContent(
      "Development",
    );
    await user.click(
      screen.getByRole("combobox", { name: "Connection history" }),
    );
    await user.click(screen.getByRole("option", { name: "Team · Personal" }));
    expect(screen.getByTestId("service-history")).toHaveTextContent("Team");
    await user.click(screen.getByRole("tab", { name: "Connections" }));
    expect(screen.queryByTestId("service-history")).not.toBeInTheDocument();
  });

  it("does not manufacture a group from the catalog or expose stale connections after an access failure", () => {
    state.groupId = "catalog:missing";
    const mounted = render(<ServiceOverviewPage />);
    expect(
      screen.getByRole("heading", { name: "Service unavailable" }),
    ).toBeVisible();
    state.groupId = "catalog:openai";
    state.error = new Error("No access");
    mounted.rerender(<ServiceOverviewPage />);
    expect(
      screen.getByText("Service information could not be loaded."),
    ).toBeVisible();
    expect(
      screen.queryByRole("link", {
        name: "Configure Development (Personal)",
      }),
    ).not.toBeInTheDocument();
  });
});
