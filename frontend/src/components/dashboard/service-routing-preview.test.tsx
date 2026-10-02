import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  render as renderDom,
  screen,
  cleanup,
  fireEvent,
  act,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactNode } from "react";
import type { KeyInfo } from "@/types/keys";
import type { ServicePool } from "@/schemas/pools";
import type { ServiceInsight } from "@/schemas/service-insights";
import { configuredBilling } from "@/lib/service-insights-compat";
import { DEFAULT_SERVICE_FILTERS } from "@/schemas/service-view";
import { AddCtaButton } from "@/components/shared/add-cta-button";

function render(ui: ReactNode) {
  const client = new QueryClient({
    defaultOptions: { mutations: { retry: false }, queries: { retry: false } },
  });
  return renderDom(ui, {
    wrapper: ({ children }) => (
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    ),
  });
}

const { records, account, poolState, insightConnections } = vi.hoisted(() => ({
  insightConnections: new Map<string, ServiceInsight>(),
  account: { id: "user-a" },
  poolState: { data: [] as ServicePool[], error: null as unknown },
  records: [
    {
      id: "mine",
      label: "Personal account",
      slug: "openai-personal",
      endpoint_url: "https://api.openai.com",
      description: "My development connection",
      granted_scopes: ["models:read"],
      catalog_service_id: "openai-id",
      catalog_service_slug: "openai",
      catalog_service_name: "OpenAI",
      service_type: "http",
      is_active: true,
      status: "active",
      credential_type: "api_key",
      auth_method: "bearer",
      api_key_id: "credential-a",
      node_id: null,
      auto_connected: false,
      credential_source: { type: "personal" },
    },
    {
      id: "team",
      label: "Team account",
      slug: "openai-team",
      catalog_service_id: "openai-id",
      catalog_service_slug: "openai",
      catalog_service_name: "OpenAI",
      service_type: "http",
      is_active: true,
      status: "active",
      credential_type: "api_key",
      auth_method: "bearer",
      api_key_id: "credential-b",
      node_id: null,
      auto_connected: false,
      credential_source: {
        type: "org",
        org_id: "team-id",
        org_name: "Chrono",
        allowed: true,
        role: "member",
      },
    },
  ] as KeyInfo[],
}));

vi.mock("@tanstack/react-router", () => ({
  Link: ({
    children,
    params = {},
    to,
    search,
    ...props
  }: {
    children: ReactNode;
    params: { keyId?: string; groupId?: string };
    to: string;
    search?: Record<string, string | undefined>;
    "aria-label"?: string;
  }) => (
    <a
      aria-label={props["aria-label"]}
      href={
        params.groupId
          ? `/keys/services/${params.groupId}`
          : params.keyId
            ? `/keys/${params.keyId}`
            : `${to}${search ? `?${new URLSearchParams(Object.entries(search).filter((entry): entry is [string, string] => entry[1] !== undefined))}` : ""}`
      }
    >
      {children}
    </a>
  ),
}));
vi.mock("@/hooks/use-keys", () => ({
  useKeys: () => ({ data: records, refetch: vi.fn() }),
  useCatalog: () => ({ data: [], refetch: vi.fn() }),
}));
vi.mock("@/hooks/use-user-services", () => ({
  useUserServices: () => ({ data: [], refetch: vi.fn() }),
}));
vi.mock("@/hooks/use-pools", () => ({
  useServicePools: () => ({ ...poolState, refetch: vi.fn() }),
  usePoolHealth: () => ({
    data: { candidates: [] },
    isError: false,
    isLoading: false,
  }),
}));
vi.mock("@/hooks/use-service-routing-pools", () => ({
  useServiceRoutingPools: () => ({
    pools: poolState.error ? [] : poolState.data,
    loading: false,
    incomplete: !!poolState.error,
  }),
}));
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (selector: (state: { user: { id: string } }) => unknown) =>
    selector({ user: account }),
}));

import ServiceRoutingPreview from "./service-routing-preview";
import { useServiceCardView } from "@/stores/service-card-view-store";

vi.mock("@/hooks/use-service-insights", () => ({
  useServiceInsights: () => ({
    connections: insightConnections,
    status: insightConnections.size ? "ready" : "unavailable",
    refresh: vi.fn(),
  }),
}));
vi.mock("@/hooks/use-nodes", () => ({ useNodes: () => ({ data: [] }) }));
function preview() {
  return <ServiceRoutingPreview />;
}
function pool(id: string, members = ["mine", "team"]): ServicePool {
  return {
    id,
    name: id,
    slug: id.toLowerCase(),
    user_id: "user-a",
    strategy: "round_robin",
    members: members.map((user_service_id) => ({
      user_service_id,
      weight: 1,
      enabled: true,
    })),
    rr_counter: 0,
    is_active: true,
    created_at: "2026-01-01",
    updated_at: "2026-01-01",
  };
}

beforeEach(() => {
  useServiceCardView.setState({
    accountId: undefined,
    expanded: [],
    filters: undefined,
  });
  localStorage.clear();
  account.id = "user-a";
  poolState.data = [];
  poolState.error = null;
  insightConnections.clear();
});
afterEach(() => {
  cleanup();
  records.splice(2);
});

describe("live grouped services", () => {
  it("counts all configured charges including disabled connections and opens the billable member", async () => {
    for (let i = 0; i < 4; i++)
      records.push({
        ...records[0]!,
        id: `extra-${i}`,
        label: `App ${i}`,
        slug: `app-${i}`,
        is_active: i !== 3,
      });
    for (const connection of records)
      insightConnections.set(connection.id, {
        service_id: connection.id,
        billing: {
          ...configuredBilling(connection),
          credit_billing_configured: connection.id === "extra-3",
        },
        usage: null,
      });
    render(preview());
    expect(screen.getByText("1 of 6 connections billable")).toBeVisible();
    expect(screen.getByText("1 disabled")).toBeVisible();
    await userEvent.click(
      screen.getByRole("button", { name: "Show billing for OpenAI" }),
    );
    expect(
      screen.getByRole("region", { name: "Billing for App 3" }),
    ).toBeVisible();
    expect(screen.getByText("NyxID usage charges configured")).toBeVisible();
    expect(screen.getByText("Billable · NyxID usage")).toBeVisible();
  });
  it("marks partial billability as a lower bound instead of counting missing billing as free", () => {
    insightConnections.set("mine", {
      service_id: "mine",
      billing: {
        ...configuredBilling(records[0]!),
        credit_billing_configured: true,
      },
      usage: null,
    });
    render(preview());
    expect(screen.getByText("1+ of 2 connections billable")).toBeVisible();
  });
  it("shows recorded last edit and opens that connection's history", async () => {
    records.push({
      ...records[0]!,
      id: "edited",
      label: "Edited connection",
      slug: "edited",
      authorship: {
        created_by: null,
        last_change: {
          actor: {
            kind: "person",
            id: "calvin",
            name: "Calvin",
            person_id: "calvin",
            api_key_id: null,
            app_id: null,
          },
          at: "2026-10-01T10:00:00Z",
          action: "updated",
          change_group_id: "change",
        },
      },
    });
    render(preview());
    expect(screen.getByText(/Last edit .* · Calvin/)).toBeVisible();
    await userEvent.click(
      screen.getByRole("button", { name: "Show last edit for OpenAI" }),
    );
    const row = screen.getByRole("row", { name: /Edited connection/ });
    expect(
      within(row).getByRole("button", { name: /History/ }),
    ).toHaveAttribute("aria-expanded", "true");
  });
  it("keeps mixed-source billing separate in Personal view and opens the selected source", async () => {
    records.push({
      ...records[0]!,
      id: "platform",
      slug: "openai-platform",
      label: "Platform account",
      credential_binding: "platform",
      auto_connected: true,
    });
    records.push({
      ...records[2]!,
      id: "platform-only",
      catalog_service_id: "other-id",
      catalog_service_name: "Platform-only service",
      catalog_service_slug: "other",
    });
    for (const connection of records) {
      insightConnections.set(connection.id, {
        service_id: connection.id,
        billing: configuredBilling(connection),
        usage: null,
      });
    }
    const user = userEvent.setup();
    render(preview());
    expect(
      screen.queryByRole("region", { name: "Platform-only service" }),
    ).not.toBeInTheDocument();
    const card = screen.getByRole("region", { name: "OpenAI" });
    expect(within(card).getByText("3 connections")).toBeVisible();
    expect(
      within(card).getByText("Billing unverified · 3 connections"),
    ).toBeVisible();
    expect(within(card).queryByText("Sources")).not.toBeInTheDocument();
    for (const source of ["Personal", "Chrono", "NyxID platform", "Personal"]) {
      await user.hover(
        screen.getByRole("button", {
          name: `${source} · Show sources for OpenAI`,
        }),
      );
      expect(
        within(await screen.findByRole("tooltip")).getByText(source),
      ).toBeVisible();
    }
    expect(screen.getAllByRole("tooltip")).toHaveLength(1);
    expect(
      within(screen.getByRole("tooltip")).getByText(
        "Connections you own. Billing is shown separately.",
      ),
    ).toBeVisible();
    await user.unhover(
      screen.getByRole("button", { name: "Personal · Show sources for OpenAI" }),
    );
    await user.click(
      within(card).getByRole("button", { name: "Expand OpenAI connections" }),
    );
    expect(within(card).getByText("openai-platform")).toBeVisible();
    expect(within(card).getByText("openai-team")).toBeVisible();
    await user.click(
      screen.getByRole("button", {
        name: "Billing for Platform account",
      }),
    );
    expect(
      within(card).getByRole("region", {
        name: "Billing for Platform account",
      }),
    ).toBeVisible();
    expect(
      within(card).getByText(/A priority pool slug can fail over/),
    ).toBeVisible();
    expect(
      within(card).getByText(
        /Eligible allowances → Credit grants → Wallet credits/,
      ),
    ).toBeVisible();
    expect(
      within(card).queryByRole("region", {
        name: "Billing for Personal account",
      }),
    ).not.toBeInTheDocument();
    await user.click(
      screen.getByRole("button", { name: "Service view: Personal" }),
    );
    expect(
      screen.getByRole("region", { name: "Platform-only service" }),
    ).toBeVisible();
  });
  it("shows configured agent associations and identifies unverified billing on hover", async () => {
    insightConnections.set("mine", {
      service_id: "mine",
      billing: configuredBilling(records[0]!),
      usage: {
        access: {
          basis: "configuration",
          visibility: "own_keys",
          truncated: false,
          keys: [
            {
              id: "agent",
              name: "Codex CI",
              platform: "codex",
              owner_id: "user-a",
              permission: "selected_service",
              credential_override: false,
            },
          ],
        },
        activity: {
          visibility: "unavailable",
          tracking: "unavailable",
          period_days: 30,
          request_count: 0,
          requests: [],
          truncated: false,
        },
      },
    });
    const user = userEvent.setup();
    render(preview());
    expect(screen.getByText("1+ agent key")).toBeVisible();
    expect(screen.getByText("Last use not reported")).toBeVisible();
    await user.hover(
      screen.getByRole("button", { name: "Show agent keys and use for OpenAI" }),
    );
    expect(await screen.findByRole("tooltip")).toHaveTextContent(
      "Keys with access: Codex CI",
    );
    await user.hover(
      screen.getByRole("button", { name: "Show billing for OpenAI" }),
    );
    const tooltip = within(await screen.findByRole("tooltip"));
    expect(tooltip.getByText("NyxID usage billing")).toBeInTheDocument();
    expect(
      tooltip.getByText("Personal account: Unverified"),
    ).toBeInTheDocument();
    expect(screen.queryByText("Latest request")).not.toBeInTheDocument();
    expect(screen.queryByText(/No recorded requests/)).not.toBeInTheDocument();
  });
  it("keeps last-used layer and billing visible when the card is expanded and opens row details", async () => {
    const user = userEvent.setup();
    insightConnections.set("mine", {
      service_id: "mine",
      billing: configuredBilling(records[0]!),
      usage: {
        access: { visibility: "own_keys", keys: [], truncated: false },
        activity: {
          visibility: "own_requests",
          tracking: "partial",
          period_days: 30,
          request_count: 1,
          truncated: false,
          requests: [
            {
              id: "event",
              execution_id: "request",
              occurred_at: "2026-09-29T12:00:00Z",
              outcome: "response_received",
              response_status: 200,
              caller: {
                id: "agent",
                kind: "agent_key",
                name: "Codex worker",
                app_id: "app",
                app_name: "Release app",
              },
              source: { kind: "platform", owner_id: "user-a" },
            },
          ],
        },
      },
    });
    render(preview());
    expect(screen.getByText("Agent keys unverified")).toBeVisible();
    expect(screen.getByText(/^Last use /)).toBeVisible();
    await user.hover(
      screen.getByRole("button", { name: "Show agent keys and use for OpenAI" }),
    );
    expect(await screen.findByRole("tooltip")).toHaveTextContent(
      /Last use: Codex worker · Release app/,
    );
    await user.click(
      screen.getByRole("button", { name: "Expand OpenAI connections" }),
    );
    const summary = screen.getByRole("button", {
      name: "Show agent keys and use for OpenAI",
    });
    expect(within(summary).getByText(/^Last use /)).toBeVisible();
    await user.click(summary);
    expect(
      screen.getByRole("table", { name: "Recent connection requests" }),
    ).toBeVisible();
    await user.hover(
      screen.getByRole("button", { name: "Show billing for OpenAI" }),
    );
    await user.click(
      screen.getByRole("button", { name: "Show billing for OpenAI" }),
    );
    expect(
      screen.getByRole("region", { name: "Billing for Personal account" }),
    ).toBeVisible();
    expect(
      screen.queryByRole("table", { name: "Recent connection requests" }),
    ).not.toBeInTheDocument();
  });
  it("keeps scroll height stable when compacting at the bottom of a short filtered view", () => {
    const callbacks: ResizeObserverCallback[] = [];
    vi.stubGlobal(
      "ResizeObserver",
      class {
        constructor(callback: ResizeObserverCallback) {
          callbacks.push(callback);
        }
        observe() {}
        disconnect() {}
      },
    );
    try {
      render(<main>{preview()}</main>);
      const main = screen.getByRole("main");
      const filters = screen.getByRole("region", { name: "Service filters" });
      const container = filters.parentElement!;
      const surface = filters.firstElementChild!;
      surface.getBoundingClientRect = () =>
        new DOMRect(0, 0, 900, filters.dataset.stuck === "true" ? 80 : 240);
      container.getBoundingClientRect = () =>
        new DOMRect(0, 80 - main.scrollTop, 900, 1200);
      filters.getBoundingClientRect = () =>
        new DOMRect(
          0,
          Math.max(0, 80 - main.scrollTop),
          900,
          Math.max(
            surface.getBoundingClientRect().height,
            parseFloat(filters.style.minHeight) || 0,
          ),
        );
      act(() =>
        callbacks.forEach((callback) => callback([], {} as ResizeObserver)),
      );
      main.scrollTop = 100;
      fireEvent.scroll(main);
      for (let frame = 0; frame < 4; frame++) {
        // Model the browser clamping scrollTop after scrollHeight shrinks.
        main.scrollTop = Math.max(
          0,
          Math.min(
            main.scrollTop,
            100 + filters.getBoundingClientRect().height - 240,
          ),
        );
        act(() =>
          callbacks.forEach((callback) => callback([], {} as ResizeObserver)),
        );
        fireEvent.scroll(main);
        expect(main.scrollTop).toBe(100);
        expect(filters).toHaveAttribute("data-stuck", "true");
        expect(
          container.style.getPropertyValue("--service-filters-height"),
        ).toBe("80px");
      }
      main.scrollTop = 0;
      fireEvent.scroll(main);
      expect(filters).toHaveAttribute("data-stuck", "false");
      expect(filters.style.minHeight).toBe("");
      expect(screen.getByRole("button", { name: "Saved views" })).toBeVisible();
    } finally {
      vi.unstubAllGlobals();
    }
  });
  it("covers the gap above a pinned service header and clears it on return or collapse", async () => {
    const user = userEvent.setup();
    render(<main>{preview()}</main>);
    const main = screen.getByRole("main");
    main.scrollTo = vi.fn();
    const card = screen.getByRole("region", { name: "OpenAI" });
    const header = within(card)
      .getByRole("heading", { name: "OpenAI" })
      .closest<HTMLDivElement>(".service-card-header")!;
    card.getBoundingClientRect = () =>
      new DOMRect(0, 300 - main.scrollTop, 800, 1000);
    header.getBoundingClientRect = () =>
      new DOMRect(
        0,
        Math.max(132, 300 - main.scrollTop) +
          (Number.parseFloat(
            (header.style.translate ?? "").split(" ")[1] ?? "0",
          ) || 0),
        800,
        120,
      );
    await user.click(
      screen.getByRole("button", { name: "Expand OpenAI connections" }),
    );
    card.querySelector<HTMLElement>(
      "[data-service-connection-row]",
    )!.getBoundingClientRect = () =>
      new DOMRect(0, 452 - main.scrollTop, 800, 180);
    fireEvent.scroll(main);
    expect(header).toHaveAttribute("data-stuck", "false");
    main.scrollTop = 300;
    fireEvent.scroll(main);
    expect(header).toHaveAttribute("data-stuck", "true");
    main.scrollTop = 0;
    fireEvent.scroll(main);
    expect(header).toHaveAttribute("data-stuck", "false");
    main.scrollTop = 300;
    fireEvent.scroll(main);
    await user.click(
      screen.getByRole("button", { name: "Collapse OpenAI connections" }),
    );
    expect(header).toHaveAttribute("data-stuck", "false");
  });

  it("releases the pinned header before the final three connections and restores it when scrolling back", async () => {
    for (let index = 2; index <= 5; index++) {
      records.push({
        ...records[0]!,
        id: `personal-${index}`,
        slug: `openai-${index}`,
      });
    }
    const user = userEvent.setup();
    render(<main>{preview()}</main>);
    const main = screen.getByRole("main");
    main.scrollTo = vi.fn();
    const card = screen.getByRole("region", { name: "OpenAI" });
    const header = within(card)
      .getByRole("heading", { name: "OpenAI" })
      .closest<HTMLDivElement>(".service-card-header")!;
    card.getBoundingClientRect = () =>
      new DOMRect(0, 300 - main.scrollTop, 800, 900);
    header.getBoundingClientRect = () =>
      new DOMRect(
        0,
        Math.max(132, 300 - main.scrollTop) +
          (Number.parseFloat(
            (header.style.translate ?? "").split(" ")[1] ?? "0",
          ) || 0),
        800,
        120,
      );
    await user.click(
      screen.getByRole("button", { name: "Expand OpenAI connections" }),
    );
    const rows = [
      ...card.querySelectorAll<HTMLElement>("[data-service-connection-row]"),
    ];
    expect(rows).toHaveLength(6);
    const rowTops = [452, 572, 672, 752, 842, 992];
    rows.forEach((row, index) => {
      row.getBoundingClientRect = () =>
        new DOMRect(0, rowTops[index]! - main.scrollTop, 800, 100);
    });
    main.scrollTop = 400;
    fireEvent.scroll(main);
    expect(header.style.translate).toBe("");
    main.scrollTop = 550;
    fireEvent.scroll(main);
    expect(header.style.translate).toBe("0 -50px");
    expect(header.getBoundingClientRect().bottom).toBe(
      rows[3]!.getBoundingClientRect().top,
    );
    main.scrollTop = 600;
    fireEvent.scroll(main);
    expect(header.style.translate).toBe("0 -100px");
    main.scrollTop = 400;
    fireEvent.scroll(main);
    expect(header.style.translate).toBe("");
    await user.click(
      screen.getByRole("button", { name: "Collapse OpenAI connections" }),
    );
    expect(header.style.translate).toBe("");
    expect(header).toHaveAttribute("data-stuck", "false");
  });

  it("shows only filters and active pills while stuck, then restores saved views and the footer", async () => {
    const user = userEvent.setup();
    const connect = vi.fn();
    render(
      <main>
        <ServiceRoutingPreview
          actions={(compact) => (
            <AddCtaButton
              compact={compact}
              compactLabel="Connect"
              label="Connect Service"
              onClick={connect}
            />
          )}
        />
      </main>,
    );
    const main = screen.getByRole("main");
    const filters = screen.getByRole("region", { name: "Service filters" });
    const container = filters.parentElement!;
    container.getBoundingClientRect = () =>
      new DOMRect(0, 80 - main.scrollTop, 900, 1200);
    filters.getBoundingClientRect = () =>
      new DOMRect(0, Math.max(0, 80 - main.scrollTop), 900, 200);
    await user.click(
      screen.getByRole("button", { name: "Expand OpenAI connections" }),
    );
    expect(screen.getByRole("button", { name: "Collapse" })).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Saved views" }));
    expect(screen.getByText(/No saved view yet/)).toBeVisible();
    expect(
      screen.getByRole("button", { name: "Connect Service" }),
    ).toHaveTextContent("Connect Service");
    const personalToggle = screen.getByRole("button", {
      name: "Service view: Personal",
    });
    await user.hover(personalToggle);
    expect(
      await screen.findByRole("tooltip", { name: /Switch to all services/ }),
    ).toHaveTextContent(
      /including their accessible organization and platform connections/,
    );
    await user.unhover(personalToggle);
    main.scrollTop = 120;
    fireEvent.scroll(main);
    expect(
      screen.queryByRole("button", { name: "Saved views" }),
    ).not.toBeInTheDocument();
    expect(screen.queryByText(/No saved view yet/)).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Save as default" }),
    ).not.toBeInTheDocument();
    const compactConnect = screen.getByRole("button", {
      name: "Connect Service",
    });
    expect(compactConnect).toHaveTextContent(/^Connect$/);
    await user.hover(compactConnect);
    expect(
      await screen.findByRole("tooltip", { name: "Connect Service" }),
    ).toBeInTheDocument();
    await user.keyboard("{Escape}");
    expect(
      screen.getByRole("button", { name: "Service view: Personal" }),
    ).toHaveTextContent(/^$/);
    await user.click(compactConnect);
    expect(connect).toHaveBeenCalledOnce();
    expect(
      screen.queryByRole("button", { name: "Filters" }),
    ).not.toBeInTheDocument();
    // Primary actions stay reachable in the stuck filter row.
    expect(
      screen.queryByRole("button", { name: "Refresh metadata" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Collapse" }),
    ).not.toBeInTheDocument();
    expect(screen.queryByText(/matching connection/)).not.toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Service view: Personal" }),
    ).toBeVisible();
    const compactPersonalToggle = screen.getByRole("button", {
      name: "Service view: Personal",
    });
    await user.hover(compactPersonalToggle);
    expect(
      await screen.findByRole("tooltip", { name: /Switch to all services/ }),
    ).toBeVisible();
    await user.click(compactPersonalToggle);
    const allToggle = screen.getByRole("button", {
      name: "Service view: All services",
    });
    expect(allToggle).toBeVisible();
    await user.unhover(allToggle);
    await user.hover(allToggle);
    expect(
      await screen.findByRole("tooltip", { name: /Switch to personal services/ }),
    ).toHaveTextContent("Showing all accessible services.");
    await user.unhover(allToggle);
    await user.click(screen.getByRole("button", { name: "Organization" }));
    await user.click(screen.getByRole("checkbox", { name: "Chrono" }));
    await user.click(screen.getByRole("button", { name: "Done" }));
    expect(
      within(filters).getByRole("button", { name: "Remove Org: Chrono" }),
    ).toBeVisible();
    main.scrollTop = 0;
    fireEvent.scroll(main);
    expect(
      screen.getByRole("button", { name: "Connect Service" }),
    ).toHaveTextContent("Connect Service");
    expect(
      screen.getByRole("button", { name: "Service view: All services" }),
    ).toHaveTextContent("All services");
    expect(screen.getByRole("button", { name: "Saved views" })).toBeVisible();
    expect(
      screen.getByRole("button", { name: "Save as default" }),
    ).toBeVisible();
    expect(
      screen.queryByRole("button", { name: "Refresh metadata" }),
    ).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Collapse" })).toBeVisible();
    expect(screen.getByText(/matching connection/)).toBeVisible();
    expect(
      screen.getAllByRole("button", { name: /^Service view:/ }),
    ).toHaveLength(1);
  });

  it("starts in Personal view and keeps all view controls in the filter card", async () => {
    records.push({
      ...records[0]!,
      id: "platform",
      auto_connected: true,
      label: "Platform connection",
      slug: "openai-platform",
    });
    const user = userEvent.setup();
    render(preview());
    const filters = screen.getByRole("region", { name: "Service filters" });
    expect(
      within(filters).getByRole("button", { name: "Service view: Personal" }),
    ).toBeVisible();
    expect(
      within(filters).getByText("1 service · 3 matching connections"),
    ).toBeVisible();
    expect(
      within(filters).queryByRole("button", { name: "Refresh metadata" }),
    ).not.toBeInTheDocument();
    expect(
      within(filters).getByRole("button", { name: "Save as default" }),
    ).toBeVisible();
    expect(
      within(filters).getAllByRole("button", { name: /^Service view:/ }),
    ).toHaveLength(1);
    await user.click(
      within(filters).getByRole("button", { name: "Saved views" }),
    );
    expect(screen.getByText(/No saved view yet/)).toBeVisible();
    expect(
      screen.getByRole("button", { name: "Save current filters as default" }),
    ).toBeDisabled();
    await user.keyboard("{Escape}");
    await user.click(
      screen.getByRole("button", { name: "Expand OpenAI connections" }),
    );
    expect(screen.getByText("openai-team")).toBeVisible();
    expect(screen.getByText("Platform connection")).toBeVisible();
    await user.click(
      within(filters).getByRole("button", { name: "Service view: Personal" }),
    );
    expect(screen.getByText("openai-team")).toBeVisible();
    expect(screen.getByText("Platform connection")).toBeVisible();
    expect(
      within(filters).getByText("1 service · 3 matching connections"),
    ).toBeVisible();
    await user.click(
      within(filters).getByRole("button", {
        name: "Service view: All services",
      }),
    );
    await user.click(
      within(filters).getByRole("button", { name: "Organization" }),
    );
    await user.click(screen.getByRole("checkbox", { name: "Chrono" }));
    await user.click(screen.getByRole("button", { name: "Done" }));
    expect(
      within(filters).getByRole("button", {
        name: "Service view: All services",
      }),
    ).toBeVisible();
    expect(
      within(filters).getByRole("button", { name: "Remove Org: Chrono" }),
    ).toBeVisible();
    expect(screen.getByText("openai-team")).toBeVisible();
    await user.click(
      within(filters).getByRole("button", {
        name: "Service view: All services",
      }),
    );
    expect(
      screen.queryByRole("button", { name: "Remove Org: Chrono" }),
    ).not.toBeInTheDocument();
    expect(screen.getByText("openai-personal")).toBeVisible();
  });

  it("starts collapsed and compares connections in a table inside one service", async () => {
    const user = userEvent.setup();
    render(preview());
    fireEvent.click(
      screen.getByRole("button", { name: "Service view: Personal" }),
    );
    const group = screen.getByRole("region", { name: "OpenAI" });
    const scroll = vi.fn();
    group.scrollIntoView = scroll;
    expect(within(group).getByText("2 connections")).toBeVisible();
    expect(
      screen.queryByText("https://api.openai.com"),
    ).not.toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Expand OpenAI connections" }),
    ).toHaveAttribute("aria-expanded", "false");
    await user.click(
      screen.getByRole("button", { name: "Expand OpenAI connections" }),
    );
    expect(within(group).getByText("https://api.openai.com")).toBeVisible();
    const table = within(group).getByRole("table", {
      name: "OpenAI connections",
    });
    expect(within(table).getAllByRole("row")).toHaveLength(3);
    expect(scroll).toHaveBeenCalledWith({
      behavior: "smooth",
      block: "start",
      inline: "nearest",
    });
    for (const name of [
      "Owner / Credential",
      "Access & requests",
      "Billing",
      "Connection / Slug",
      "Configuration",
    ]) {
      expect(within(table).getByRole("columnheader", { name })).toBeVisible();
    }
    expect(within(group).getByText("openai-team")).toBeVisible();
    expect(within(group).queryByText("1 granted")).not.toBeInTheDocument();
    expect(
      within(group).getByRole("link", {
        name: "Configure Personal account (Personal)",
      }),
    ).toHaveAttribute("href", "/keys/mine");
    expect(
      within(group).getByRole("link", {
        name: "View all OpenAI service details",
      }),
    ).toHaveAttribute("href", "/keys/services/catalog:openai-id");
    expect(
      within(group).queryByText("Details", { selector: "summary" }),
    ).not.toBeInTheDocument();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    await user.click(
      screen.getByRole("button", { name: "Collapse OpenAI connections" }),
    );
    expect(scroll).toHaveBeenCalledTimes(1);
    expect(
      screen.queryByRole("button", {
        name: /Drag|By service|Individual cards/,
      }),
    ).not.toBeInTheDocument();
  });

  it("closes the previous service and scrolls the most recently opened card", async () => {
    records.push({
      ...records[0]!,
      id: "twilio",
      label: "Twilio account",
      slug: "twilio",
      catalog_service_id: "twilio-id",
      catalog_service_name: "Twilio",
      catalog_service_slug: "twilio",
    });
    const user = userEvent.setup();
    render(preview());
    fireEvent.click(
      screen.getByRole("button", { name: "Service view: Personal" }),
    );
    const openai = screen.getByRole("region", { name: "OpenAI" });
    const twilio = screen.getByRole("region", { name: "Twilio" });
    const scrollOpenai = vi.fn();
    const scrollTwilio = vi.fn();
    openai.scrollIntoView = scrollOpenai;
    twilio.scrollIntoView = scrollTwilio;
    await user.click(
      screen.getByRole("button", { name: "Expand OpenAI connections" }),
    );
    expect(
      screen.getByRole("table", { name: "OpenAI connections" }),
    ).toBeVisible();
    await user.click(within(twilio).getByRole("button", { name: "Twilio" }));
    expect(
      screen.queryByRole("table", { name: "OpenAI connections" }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByRole("table", { name: "Twilio connections" }),
    ).toBeVisible();
    expect(
      screen.getByRole("button", { name: "Expand OpenAI connections" }),
    ).toHaveAttribute("aria-expanded", "false");
    expect(scrollOpenai).toHaveBeenCalledOnce();
    expect(scrollTwilio).toHaveBeenCalledOnce();
    await user.click(
      screen.getByRole("button", { name: "Expand OpenAI connections" }),
    );
    expect(
      screen.queryByRole("table", { name: "Twilio connections" }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByRole("table", { name: "OpenAI connections" }),
    ).toBeVisible();
    expect(scrollOpenai).toHaveBeenCalledTimes(2);
    expect(scrollTwilio).toHaveBeenCalledOnce();
  });

  it("searches a nested connection and shows only matching rows with the group total", async () => {
    const user = userEvent.setup();
    render(preview());
    fireEvent.click(
      screen.getByRole("button", { name: "Service view: Personal" }),
    );
    await user.type(
      screen.getByRole("textbox", { name: "Search services and connections" }),
      "Team account{Enter}",
    );
    expect(screen.getByText("1 of 2 match")).toBeVisible();
    expect(screen.getByText("1 of 2 connections")).toBeVisible();
    await user.click(
      screen.getByRole("button", { name: "Expand OpenAI connections" }),
    );
    expect(screen.queryByText("openai-personal")).not.toBeInTheDocument();
    expect(screen.getByText("openai-team")).toBeVisible();
  });

  it("filters by actual organizations without the extra Filters menu", async () => {
    const user = userEvent.setup();
    render(preview());
    expect(
      screen.queryByRole("button", { name: "Filters" }),
    ).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Organization" }));
    await user.click(screen.getByRole("checkbox", { name: "Chrono" }));
    await user.click(screen.getByRole("button", { name: "Done" }));
    expect(screen.getByText("1 of 2 match")).toBeVisible();
    await user.click(
      screen.getByRole("button", { name: "Expand OpenAI connections" }),
    );
    expect(screen.queryByText("openai-personal")).not.toBeInTheDocument();
    expect(screen.getByText("openai-team")).toBeVisible();
  });

  it("keeps criteria from older saved views visible and removable", async () => {
    useServiceCardView.setState({
      accountId: account.id,
      filters: { ...DEFAULT_SERVICE_FILTERS, state: "disabled" },
    });
    const user = userEvent.setup();
    render(preview());
    expect(
      screen.queryByRole("button", { name: "Filters" }),
    ).not.toBeInTheDocument();
    expect(screen.getByText("Disabled")).toBeVisible();
    expect(
      screen.queryByRole("region", { name: "OpenAI" }),
    ).not.toBeInTheDocument();
    await user.click(
      screen.getByRole("button", { name: "Remove Service state filter" }),
    );
    expect(screen.getByRole("region", { name: "OpenAI" })).toBeVisible();
  });

  it("supports multiple organizations and services with removable selection pills", async () => {
    records.push(
      {
        ...records[1]!,
        id: "elf",
        label: "Elf account",
        credential_source: {
          type: "org",
          org_id: "elf-id",
          org_name: "Elf",
          allowed: true,
          role: "admin",
          avatar_url: null,
        },
      },
      {
        ...records[1]!,
        id: "twilio",
        label: "Twilio",
        slug: "twilio",
        catalog_service_id: "twilio-id",
        catalog_service_name: "Twilio",
        catalog_service_slug: "twilio",
      },
    );
    const user = userEvent.setup();
    render(preview());
    fireEvent.click(
      screen.getByRole("button", { name: "Service view: Personal" }),
    );
    await user.click(screen.getByRole("button", { name: "Organization" }));
    await user.click(screen.getByRole("checkbox", { name: "Chrono" }));
    await user.click(screen.getByRole("checkbox", { name: "Elf" }));
    expect(screen.getByRole("checkbox", { name: "Chrono" })).toBeChecked();
    expect(screen.getByRole("checkbox", { name: "Elf" })).toBeChecked();
    await user.click(screen.getByRole("button", { name: "Done" }));
    expect(
      screen.getByRole("button", { name: "Organization" }),
    ).toHaveTextContent("2 selected");
    expect(
      screen.getByRole("button", { name: "Remove Org: Chrono" }),
    ).toBeVisible();
    expect(
      screen.getByRole("button", { name: "Remove Org: Elf" }),
    ).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Service" }));
    await user.type(
      screen.getByRole("textbox", { name: "Search services" }),
      "OpenAI",
    );
    expect(
      screen.queryByRole("checkbox", { name: "Twilio" }),
    ).not.toBeInTheDocument();
    await user.click(screen.getByRole("checkbox", { name: "OpenAI" }));
    await user.clear(screen.getByRole("textbox", { name: "Search services" }));
    await user.click(screen.getByRole("checkbox", { name: "Twilio" }));
    await user.click(screen.getByRole("button", { name: "Done" }));
    expect(
      screen.getByRole("button", { name: "Remove Service: OpenAI" }),
    ).toBeVisible();
    expect(
      screen.getByRole("button", { name: "Remove Service: Twilio" }),
    ).toBeVisible();
    expect(screen.getByRole("region", { name: "OpenAI" })).toBeVisible();
    expect(screen.getByRole("region", { name: "Twilio" })).toBeVisible();
    await user.click(
      screen.getByRole("button", { name: "Expand OpenAI connections" }),
    );
    expect(
      within(
        screen.getByRole("table", { name: "OpenAI connections" }),
      ).getAllByRole("row"),
    ).toHaveLength(3);
    expect(screen.queryByText("openai-personal")).not.toBeInTheDocument();
    await user.click(
      screen.getByRole("button", { name: "Remove Org: Chrono" }),
    );
    expect(
      screen.queryByRole("region", { name: "Twilio" }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Remove Service: Twilio" }),
    ).toBeVisible();
    expect(
      screen.getByRole("button", { name: "Remove Org: Elf" }),
    ).toBeVisible();
    expect(
      within(
        screen.getByRole("table", { name: "OpenAI connections" }),
      ).getAllByRole("row"),
    ).toHaveLength(2);
    await user.click(
      screen.getByRole("button", { name: "Remove Service: OpenAI" }),
    );
    expect(screen.getByText(/No services match these filters/)).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Organization" }));
    expect(screen.getByRole("checkbox", { name: "Elf" })).toBeChecked();
    expect(screen.getByRole("checkbox", { name: "Chrono" })).not.toBeChecked();
    await user.click(
      screen.getByRole("button", { name: "Clear organizations" }),
    );
    await user.click(screen.getByRole("button", { name: "Done" }));
    expect(screen.getByRole("region", { name: "Twilio" })).toBeVisible();
    expect(
      screen.getByRole("button", { name: "Remove Service: Twilio" }),
    ).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Clear filters" }));
    expect(
      screen.queryByRole("button", { name: /Remove (Org|Service):/ }),
    ).not.toBeInTheDocument();
    expect(screen.getByRole("region", { name: "OpenAI" })).toBeVisible();
  });

  it("distinguishes disabled, inaccessible and missing credentials and reports changes without inventing usage", async () => {
    records.push(
      {
        ...records[0]!,
        id: "disabled",
        label: "Disabled",
        slug: "disabled",
        is_active: false,
      },
      {
        ...records[1]!,
        id: "denied",
        label: "Denied",
        slug: "denied",
        credential_source: {
          ...records[1]!.credential_source!,
          type: "org",
          org_id: "denied-org",
          org_name: "Restricted",
          allowed: false,
          role: "viewer",
          avatar_url: null,
        },
      },
      {
        ...records[0]!,
        id: "missing",
        label: "Missing",
        slug: "missing",
        credential_missing: true,
        last_used_at: "2026-09-27",
        source_app_name: "Provisioning app",
        authorship: {
          created_by: null,
          last_change: {
            actor: {
              kind: "agent",
              id: "agent-id",
              name: "Build agent",
              api_key_id: "agent-id",
              app_id: null,
              person_id: null,
            },
            at: "2026-09-26T10:00:00Z",
            action: "updated",
            change_group_id: "change-id",
          },
        },
      },
    );
    const user = userEvent.setup();
    render(preview());
    fireEvent.click(
      screen.getByRole("button", { name: "Service view: Personal" }),
    );
    await user.click(
      screen.getByRole("button", { name: "Expand OpenAI connections" }),
    );
    const table = screen.getByRole("table", { name: "OpenAI connections" });
    expect(
      within(table).getByText("Disabled", { selector: "div" }),
    ).toBeVisible();
    expect(within(table).getByText("No access")).toBeVisible();
    expect(within(table).getByText("Credential missing")).toBeVisible();
    expect(within(table).getByText(/Changed .*· Build agent/)).toBeVisible();
    expect(
      within(table).queryByText(/Ready|Provisioning app/),
    ).not.toBeInTheDocument();
  });

  it("lets the user clear an empty filter result", async () => {
    const user = userEvent.setup();
    render(preview());
    await user.click(
      screen.getByRole("button", { name: "Service view: Personal" }),
    );
    await user.type(
      screen.getByRole("textbox", { name: "Search services and connections" }),
      "no-matching-service{Enter}",
    );
    expect(screen.getByText(/No services match these filters/)).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Clear filters" }));
    expect(screen.getByRole("region", { name: "OpenAI" })).toBeVisible();
  });

  it("omits nonexistent platform sources and never invents a routing decision", async () => {
    const user = userEvent.setup();
    render(preview());
    fireEvent.click(
      screen.getByRole("button", { name: "Service view: Personal" }),
    );
    await user.click(
      screen.getByRole("button", { name: "Expand OpenAI connections" }),
    );
    expect(screen.queryByText(/NyxID platform/)).not.toBeInTheDocument();
    expect(screen.getAllByText("Not verified")).toHaveLength(2);
    expect(screen.queryByText(/Ready via|Would use/)).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Organization" })).toBeVisible();
    expect(screen.getByRole("button", { name: "Service" })).toBeVisible();
  });

  it("shows a real platform connection inside its service without claiming health", async () => {
    records.push({
      ...records[0]!,
      id: "platform",
      label: "Platform account",
      auto_connected: true,
    });
    const user = userEvent.setup();
    render(preview());
    fireEvent.click(
      screen.getByRole("button", { name: "Service view: Personal" }),
    );
    expect(screen.getByText("3 connections")).toBeVisible();
    await user.click(
      screen.getByRole("button", { name: "Expand OpenAI connections" }),
    );
    expect(screen.getByText("Platform account")).toBeVisible();
    expect(screen.getAllByText("Not verified")).toHaveLength(3);
    expect(
      within(
        screen.getByRole("table", { name: "OpenAI connections" }),
      ).getByRole("img", { name: "NyxID platform" }),
    ).toHaveAttribute("src", "/nyxid-coloured-icon.svg");
    expect(screen.queryByText(/Ready via/)).not.toBeInTheDocument();
  });

  it("keeps custom services separate even when labels and addresses look alike", async () => {
    records.push({
      ...records[0]!,
      id: "custom",
      catalog_service_id: null,
      catalog_service_slug: null,
      label: "OpenAI custom",
    });
    const user = userEvent.setup();
    render(preview());
    fireEvent.click(
      screen.getByRole("button", { name: "Service view: Personal" }),
    );
    expect(screen.getByRole("region", { name: "OpenAI" })).toBeVisible();
    expect(screen.getByRole("region", { name: "OpenAI custom" })).toBeVisible();
    await user.click(
      screen.getByRole("button", { name: "Expand OpenAI connections" }),
    );
    expect(
      screen.getByRole("button", { name: "Expand OpenAI custom connections" }),
    ).toHaveAttribute("aria-expanded", "false");
  });

  it("restores expansion after detail navigation and isolates it when accounts change", async () => {
    const user = userEvent.setup();
    const mounted = render(preview());
    fireEvent.click(
      screen.getByRole("button", { name: "Service view: Personal" }),
    );
    await user.click(
      screen.getByRole("button", { name: "Expand OpenAI connections" }),
    );
    mounted.unmount();
    const next = render(preview());
    expect(
      screen.getByRole("button", { name: "Service view: All services" }),
    ).toBeVisible();
    expect(
      screen.getByRole("button", { name: "Collapse OpenAI connections" }),
    ).toHaveAttribute("aria-expanded", "true");
    account.id = "user-b";
    next.rerender(preview());
    fireEvent.click(
      screen.getByRole("button", { name: "Service view: Personal" }),
    );
    expect(
      screen.getByRole("button", { name: "Expand OpenAI connections" }),
    ).toHaveAttribute("aria-expanded", "false");
  });
});

describe("saved routing in service cards", () => {
  it("does not infer a pool from grouped connections", async () => {
    render(preview());
    expect(screen.getByText("Direct connections · no pool")).toBeVisible();
    await userEvent.click(
      screen.getByRole("button", { name: "Show routing for OpenAI" }),
    );
    expect(
      screen.getByText(/Create a pool to give compatible connections/),
    ).toBeVisible();
  });
  it("opens actual priority failover and the full pool member table inside the card", async () => {
    poolState.data = [
      { ...pool("Reliable"), strategy: "priority", failover: null },
    ];
    render(preview());
    expect(screen.getByText("2 connections · Priority")).toBeVisible();
    expect(screen.getByText("Reliable · Up to 3 attempts")).toBeVisible();
    expect(
      screen.getByRole("button", { name: "Show routing for OpenAI" }),
    ).toHaveAttribute("aria-expanded", "false");
    await userEvent.click(
      screen.getByRole("button", { name: "Show routing for OpenAI" }),
    );
    const card = screen.getByRole("region", { name: "OpenAI" });
    expect(within(card).getByText("/api/v1/proxy/s/reliable")).toBeVisible();
    expect(within(card).getByText("Failover · up to 3 attempts")).toBeVisible();
    expect(
      within(card).getByRole("table", { name: "Reliable route members" }),
    ).toBeVisible();
    expect(within(card).getByText("Personal account")).toBeVisible();
    expect(within(card).getByText("Team account")).toBeVisible();
    expect(
      within(card).queryByRole("button", { name: "Configure pool" }),
    ).not.toBeInTheDocument();
    expect(
      within(card).getByRole("link", { name: "Manage in Service Pools" }),
    ).toHaveAttribute("href", "/keys?tab=pools&view=routing&pool=Reliable");
    expect(
      within(card).queryByRole("table", { name: "OpenAI connections" }),
    ).not.toBeInTheDocument();
    await userEvent.click(
      within(card).getByRole("button", { name: "Expand OpenAI connections" }),
    );
    expect(
      within(card).getByRole("table", { name: "OpenAI connections" }),
    ).toBeVisible();
  });
  it("opens connection billing in one click when switching from the route table", async () => {
    poolState.data = [{ ...pool("Reliable"), strategy: "priority" }];
    for (const connection of records)
      insightConnections.set(connection.id, {
        service_id: connection.id,
        billing: configuredBilling(connection),
        usage: null,
      });
    render(preview());
    await userEvent.click(
      screen.getByRole("button", { name: "Show routing for OpenAI" }),
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Show billing for OpenAI" }),
    );
    expect(screen.getByText("Billing flow")).toBeVisible();
    expect(
      screen.queryByRole("table", { name: "Reliable route members" }),
    ).not.toBeInTheDocument();
  });
  it("does not describe weighted rotation as failover", async () => {
    poolState.data = [{ ...pool("Rotate"), strategy: "weighted" }];
    render(preview());
    expect(screen.getByText("2 connections · Weighted")).toBeVisible();
    expect(screen.getByText("Rotate · Off · single attempt")).toBeVisible();
    await userEvent.click(
      screen.getByRole("button", { name: "Show routing for OpenAI" }),
    );
    expect(screen.getByText("Single attempt · no failover")).toBeVisible();
  });
  it("shows mixed failover at a glance and links the selected org pool to its owner", async () => {
    poolState.data = [
      { ...pool("Personal"), strategy: "weighted" },
      { ...pool("Team"), user_id: "team-id", strategy: "priority" },
    ];
    render(preview());
    const summary = screen.getByRole("button", {
      name: "Show routing for OpenAI",
    });
    expect(within(summary).getByText("2 pools")).toBeVisible();
    expect(
      within(summary).getByText("Failover · On in 1 of 2 pools"),
    ).toBeVisible();
    await userEvent.click(summary);
    await userEvent.click(screen.getByRole("button", { name: "Team" }));
    expect(
      screen.getByRole("link", { name: "Manage in Service Pools" }),
    ).toHaveAttribute(
      "href",
      "/keys?tab=pools&view=routing&pool=Team&org=team-id",
    );
    expect(
      screen.queryByRole("button", { name: "Configure pool" }),
    ).not.toBeInTheDocument();
  });
  it("reports pool read failures instead of claiming individual routes only", async () => {
    poolState.error = new Error("Unavailable");
    render(preview());
    expect(screen.getByText("Pool access incomplete")).toBeVisible();
  });
});
