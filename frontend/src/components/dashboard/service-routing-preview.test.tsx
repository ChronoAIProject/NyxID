import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  render as renderDom,
  screen,
  cleanup,
  fireEvent,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactNode } from "react";
import type { RoutingCandidate } from "@/lib/service-routing-preview";
import type { KeyInfo } from "@/types/keys";
import type { ServicePool } from "@/schemas/pools";

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

const { records, account, poolState } = vi.hoisted(() => ({
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
    params,
    ...props
  }: {
    children: ReactNode;
    params: { keyId?: string; groupId?: string };
    "aria-label"?: string;
  }) => (
    <a
      aria-label={props["aria-label"]}
      href={
        params.groupId
          ? `/keys/services/${params.groupId}`
          : `/keys/${params.keyId}`
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
}));
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (selector: (state: { user: { id: string } }) => unknown) =>
    selector({ user: account }),
}));

import ServiceRoutingPreview from "./service-routing-preview";
import { useServiceCardView } from "@/stores/service-card-view-store";
import ServicePoolRoutingPreview from "./service-pool-routing-preview";

vi.mock("@/hooks/use-service-insights", () => ({
  useServiceInsights: () => ({
    connections: new Map(),
    status: "unavailable",
    refresh: vi.fn(),
  }),
}));
vi.mock("@/hooks/use-nodes", () => ({ useNodes: () => ({ data: [] }) }));
function renderConnection(candidate: RoutingCandidate) {
  return <span>{candidate.key.label}</span>;
}
function preview() {
  return <ServiceRoutingPreview />;
}
function poolPreview() {
  return <ServicePoolRoutingPreview renderConnection={renderConnection} />;
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
});
afterEach(() => {
  cleanup();
  records.splice(2);
});

describe("live grouped services", () => {
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
    expect(rows).toHaveLength(5);
    const rowTops = [452, 572, 752, 842, 992];
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
      rows[2]!.getBoundingClientRect().top,
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
    render(<main>{preview()}</main>);
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
    main.scrollTop = 120;
    fireEvent.scroll(main);
    expect(
      screen.queryByRole("button", { name: "Saved views" }),
    ).not.toBeInTheDocument();
    expect(screen.queryByText(/No saved view yet/)).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Save as default" }),
    ).not.toBeInTheDocument();
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
    await user.click(
      screen.getByRole("button", { name: "Service view: Personal" }),
    );
    expect(
      screen.getByRole("button", { name: "Service view: All services" }),
    ).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Organization" }));
    await user.click(screen.getByRole("checkbox", { name: "Chrono" }));
    await user.click(screen.getByRole("button", { name: "Done" }));
    expect(
      within(filters).getByRole("button", { name: "Remove Org: Chrono" }),
    ).toBeVisible();
    main.scrollTop = 0;
    fireEvent.scroll(main);
    expect(screen.getByRole("button", { name: "Saved views" })).toBeVisible();
    expect(
      screen.getByRole("button", { name: "Save as default" }),
    ).toBeVisible();
    expect(
      screen.getByRole("button", { name: "Refresh metadata" }),
    ).toBeVisible();
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
    });
    const user = userEvent.setup();
    render(preview());
    const filters = screen.getByRole("region", { name: "Service filters" });
    expect(
      within(filters).getByRole("button", { name: "Service view: Personal" }),
    ).toBeVisible();
    expect(
      within(filters).getByText("1 service · 1 matching connection"),
    ).toBeVisible();
    expect(
      within(filters).getByRole("button", { name: "Refresh metadata" }),
    ).toBeVisible();
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
    expect(screen.queryByText("openai-team")).not.toBeInTheDocument();
    expect(screen.queryByText("Platform connection")).not.toBeInTheDocument();
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

  it("filters by actual sources and hides nonmatching connection rows", async () => {
    const user = userEvent.setup();
    render(preview());
    fireEvent.click(
      screen.getByRole("button", { name: "Service view: Personal" }),
    );
    await user.click(screen.getByRole("button", { name: "Filters" }));
    expect(
      screen.queryByRole("button", { name: "NyxID platform" }),
    ).not.toBeInTheDocument();
    await user.click(
      within(screen.getByRole("dialog")).getByRole("button", {
        name: "Organization",
      }),
    );
    await user.click(screen.getByRole("button", { name: "Apply filters" }));
    expect(screen.getByText("1 of 2 match")).toBeVisible();
    await user.click(
      screen.getByRole("button", { name: "Expand OpenAI connections" }),
    );
    expect(screen.queryByText("openai-personal")).not.toBeInTheDocument();
    expect(screen.getByText("openai-team")).toBeVisible();
    expect(
      screen.getByRole("button", { name: "Save as default" }),
    ).toBeDisabled();
  });

  it("uses the audit log Apply, Cancel and editable filter chip flow", async () => {
    const user = userEvent.setup();
    render(preview());
    fireEvent.click(
      screen.getByRole("button", { name: "Service view: Personal" }),
    );
    await user.click(screen.getByRole("button", { name: "Filters" }));
    await user.click(screen.getByRole("button", { name: "Service state" }));
    await user.click(screen.getByRole("button", { name: "Disabled" }));
    expect(screen.getByRole("region", { name: "OpenAI" })).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.getByRole("button", { name: "Filters" })).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Filters" }));
    await user.click(screen.getByRole("button", { name: "Disabled" }));
    await user.click(screen.getByRole("button", { name: "Apply filters" }));
    await user.click(
      screen.getByRole("button", { name: /Edit Service state/ }),
    );
    expect(screen.getByRole("button", { name: "Disabled" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    await user.click(screen.getByRole("button", { name: "Cancel" }));
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
    expect(within(table).getByText("Changed by Build agent")).toBeVisible();
    expect(
      within(table).queryByText(/Last used|Ready|Provisioning app/),
    ).not.toBeInTheDocument();
  });

  it("lets the user clear an empty filter result", async () => {
    const user = userEvent.setup();
    render(preview());
    fireEvent.click(
      screen.getByRole("button", { name: "Service view: Personal" }),
    );
    await user.click(screen.getByRole("button", { name: "Filters" }));
    await user.click(screen.getByRole("button", { name: "Service state" }));
    await user.click(screen.getByRole("button", { name: "Disabled" }));
    await user.click(screen.getByRole("button", { name: "Apply filters" }));
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
    expect(
      screen.queryByText(/Ready via|Would use|Automatic/),
    ).not.toBeInTheDocument();
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

describe("pool member ordering", () => {
  it("never infers a pool from connections that share a catalog service", () => {
    render(poolPreview());
    expect(screen.getByText("No service pools in your account.")).toBeVisible();
    expect(
      screen.queryByRole("button", { name: /Drag/ }),
    ).not.toBeInTheDocument();
  });

  it("requires opting into priority, scopes drag to one actual pool, and preserves live records", async () => {
    const user = userEvent.setup();
    poolState.data = [pool("First"), pool("Second")];
    const before = JSON.stringify(poolState.data);
    render(poolPreview());
    expect(
      screen.queryByRole("button", { name: /Drag/ }),
    ).not.toBeInTheDocument();
    await user.selectOptions(
      screen.getByRole("combobox", { name: "Selection for First" }),
      "priority",
    );
    await user.selectOptions(
      screen.getByRole("combobox", { name: "Selection for Second" }),
      "priority",
    );
    const transferData = new Map<string, string>();
    const transfer = {
      setData: (type: string, value: string) => transferData.set(type, value),
      getData: (type: string) => transferData.get(type),
      effectAllowed: "",
      dropEffect: "",
    };
    fireEvent.dragStart(
      screen.getByRole("button", { name: "Drag Team account in First" }),
      { dataTransfer: transfer },
    );
    const foreign = screen
      .getByRole("button", { name: "Drag Personal account in Second" })
      .closest("li")!;
    fireEvent.dragOver(foreign, { dataTransfer: transfer });
    fireEvent.drop(foreign, { dataTransfer: transfer });
    expect(
      within(
        within(
          screen.getByRole("list", { name: "Members of Second" }),
        ).getAllByRole("listitem")[0]!,
      ).getByText("Personal account"),
    ).toBeVisible();
    const target = screen
      .getByRole("button", { name: "Drag Personal account in First" })
      .closest("li")!;
    fireEvent.dragOver(target, { dataTransfer: transfer });
    fireEvent.drop(target, { dataTransfer: transfer });
    expect(
      within(
        within(
          screen.getByRole("list", { name: "Members of First" }),
        ).getAllByRole("listitem")[0]!,
      ).getByText("Team account"),
    ).toBeVisible();
    expect(JSON.stringify(poolState.data)).toBe(before);
    expect(
      screen.queryByText(/Ready via|Selected connection/),
    ).not.toBeInTheDocument();
  });

  it("supports keyboard ordering and persists independently for each pool and account", async () => {
    const user = userEvent.setup();
    poolState.data = [pool("First"), pool("Second")];
    const mounted = render(poolPreview());
    await user.selectOptions(
      screen.getByRole("combobox", { name: "Selection for First" }),
      "priority",
    );
    fireEvent.keyDown(
      screen.getByRole("button", { name: "Drag Team account in First" }),
      { key: "ArrowUp" },
    );
    mounted.unmount();
    const next = render(poolPreview());
    expect(
      within(
        within(
          screen.getByRole("list", { name: "Members of First" }),
        ).getAllByRole("listitem")[0]!,
      ).getByText("Team account"),
    ).toBeVisible();
    expect(
      screen.getByRole("combobox", { name: "Selection for Second" }),
    ).toHaveValue("current");
    account.id = "user-b";
    next.rerender(poolPreview());
    expect(
      screen.getByRole("combobox", { name: "Selection for First" }),
    ).toHaveValue("current");
  });

  it("shows excluded and missing members without selecting them or fabricating cards", async () => {
    poolState.data = [pool("First", ["mine", "missing"])];
    poolState.data[0]!.members[0]!.enabled = false;
    render(poolPreview());
    expect(screen.getByText("Excluded")).toBeVisible();
    expect(screen.getByText("Service unavailable")).toBeVisible();
    expect(screen.queryByText("Team account")).not.toBeInTheDocument();
  });
});
