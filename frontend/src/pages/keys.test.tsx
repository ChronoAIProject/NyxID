import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import {
  render as renderDom,
  screen,
  waitFor,
  fireEvent,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { KeyInfo } from "@/types/keys";

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

const { mockNavigate, mockPoolOwner, state } = vi.hoisted(() => ({
  mockNavigate: vi.fn(),
  mockPoolOwner: vi.fn(),
  // Mutable containers populated per-test before render.
  state: {
    search: {} as {
      tab?: string;
      slug?: string;
      action?: string;
      view?: string;
      pool?: string;
      org?: string;
    },
    keys: [] as KeyInfo[],
    keysLoading: false,
    keysError: null as unknown,
    userServices: [] as unknown[],
    nodes: [] as { id: string; name: string }[],
  },
}));

vi.mock("@tanstack/react-router", () => ({
  Link: ({
    children,
    to,
    params,
    ...props
  }: {
    readonly children: ReactNode;
    readonly to: string;
    readonly params?: Record<string, string>;
    readonly "aria-label"?: string;
  }) => (
    <a
      aria-label={props["aria-label"]}
      href={params ? `${to}:${Object.values(params).join("/")}` : to}
    >
      {children}
    </a>
  ),
  useNavigate: () => mockNavigate,
  useSearch: () => state.search,
}));

vi.mock("@/hooks/use-keys", () => ({
  useCatalog: () => ({ data: [], refetch: vi.fn() }),
  useKeys: () => ({
    data: state.keys,
    isLoading: state.keysLoading,
    error: state.keysError,
    refetch: vi.fn(),
  }),
}));

vi.mock("@/hooks/use-user-services", () => ({
  useUserServices: () => ({ data: state.userServices }),
}));

vi.mock("@/hooks/use-nodes", () => ({
  useNodes: () => ({ data: state.nodes }),
}));

vi.mock("@/hooks/use-pools", () => ({
  useUpdateServicePool: () => ({ mutateAsync: vi.fn(), isPending: false }),
  useDeleteServicePool: () => ({ mutateAsync: vi.fn(), isPending: false }),
  usePoolHealth: () => ({
    data: { candidates: [] },
    isError: false,
    isLoading: false,
  }),
  useServicePools: (orgId?: string) => {
    mockPoolOwner(orgId);
    return {
      data: [
        {
          id: "pool-one",
          name: "My pool",
          slug: "my-pool",
          strategy: "round_robin",
          members: [{ user_service_id: "key-1", enabled: true, weight: 1 }],
          is_active: true,
        },
      ],
    };
  },
}));

vi.mock("@/hooks/use-service-routing-pools", () => ({
  useServiceRoutingPools: () => ({
    pools: [],
    loading: false,
    incomplete: false,
  }),
}));
vi.mock("@/hooks/use-orgs", () => ({
  useOrgs: () => ({
    data: [
      {
        id: "team-id",
        display_name: "Research",
        slug: "research",
        your_role: "admin",
      },
    ],
  }),
}));

// Heavy children — stubbed to assert wiring (open state, presence), not driven.
vi.mock("@/components/providers/codex-connection", () => ({
  CodexConnectionSection: () => <div data-testid="codex-connection" />,
}));

vi.mock("@/components/dashboard/add-key-dialog", () => ({
  AddKeyDialog: ({
    open,
    prefillSlug,
    reconnectKey,
  }: {
    readonly open: boolean;
    readonly prefillSlug?: string;
    readonly reconnectKey?: KeyInfo | null;
  }) =>
    open ? (
      <div
        data-testid="add-key-dialog"
        data-prefill={prefillSlug ?? ""}
        data-reconnect={reconnectKey?.id ?? ""}
      />
    ) : null,
}));

vi.mock("@/components/dashboard/api-key-table", () => ({
  ApiKeyTable: ({ viewMode }: { readonly viewMode: string }) => (
    <div data-testid="api-key-table" data-view={viewMode} />
  ),
}));

vi.mock("@/components/dashboard/api-key-create-dialog", () => ({
  ApiKeyCreateDialog: ({
    externalOpen,
  }: {
    readonly externalOpen?: boolean;
  }) => (
    <div data-testid="api-key-create-dialog" data-open={String(externalOpen)} />
  ),
}));

vi.mock("@/components/dashboard/api-key-usage-dashboard", () => ({
  ApiKeyUsageDashboard: () => <div data-testid="api-key-usage-dashboard" />,
}));

vi.mock("@/components/orgs/role-badge", () => ({
  RoleBadge: ({ role }: { readonly role: string }) => (
    <span data-testid="role-badge">{role}</span>
  ),
}));

vi.mock("@/components/orgs/org-avatar", () => ({
  OrgAvatar: ({ displayName }: { readonly displayName: string }) => (
    <div data-testid="org-avatar">{displayName}</div>
  ),
}));

import { KeysPage } from "./keys";
import { useServiceCardView } from "@/stores/service-card-view-store";

function expandConnections() {
  for (const button of screen.queryAllByRole("button", {
    name: /^Expand .+ connections$/,
  }))
    fireEvent.click(button);
}

function makeKey(overrides: Partial<KeyInfo> = {}): KeyInfo {
  return {
    id: "key-1",
    credential_source: { type: "personal" },
    label: "My OpenAI",
    slug: "openai",
    endpoint_url: "https://api.openai.com",
    endpoint_id: "ep-1",
    credential_type: "bearer",
    auth_method: "bearer",
    auth_key_name: "Authorization",
    status: "active",
    catalog_service_id: "cat-1",
    catalog_service_slug: "openai",
    catalog_service_name: "OpenAI",
    node_id: null,
    node_priority: 0,
    is_active: true,
    ws_frame_injections: [],
    auto_connected: false,
    expires_at: null,
    last_used_at: null,
    error_message: null,
    created_at: "2026-04-20T00:00:00Z",
    service_type: "http",
    ssh_host: null,
    ssh_port: null,
    ssh_ca_public_key: null,
    ssh_allowed_principals: null,
    ssh_certificate_ttl_minutes: null,
    ...overrides,
  };
}

describe("KeysPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useServiceCardView.setState({
      accountId: undefined,
      expanded: [],
      filters: undefined,
    });
    state.search = {};
    state.keys = [];
    state.keysLoading = false;
    state.keysError = null;
    state.userServices = [];
    state.nodes = [];
  });

  it("defaults to one collapsed card per service with duplicates inside", async () => {
    state.keys = [
      makeKey({ id: "key-1", label: "Personal OpenAI", slug: "openai" }),
      makeKey({ id: "key-2", label: "Work OpenAI", slug: "openai-work" }),
    ];
    render(<KeysPage />);
    const group = screen.getByRole("region", { name: "OpenAI" });
    expect(within(group).getByText("2 connections")).toBeVisible();
    expect(screen.queryByText("openai")).not.toBeInTheDocument();
    expect(screen.queryByTestId("api-key-table")).not.toBeInTheDocument();
    expandConnections();
    expect(within(group).getByText("openai")).toBeVisible();
    expect(within(group).getByText("openai-work")).toBeVisible();
    expect(
      within(group).getByRole("link", {
        name: "View Personal OpenAI connection details (Personal)",
      }),
    ).toHaveAttribute("href", "/keys/$keyId:key-1");
    expect(
      within(group).getByRole("link", {
        name: "View Work OpenAI connection details (Personal)",
      }),
    ).toHaveAttribute("href", "/keys/$keyId:key-2");
  });

  it("uses the connection table and full detail navigation in the routing view", async () => {
    state.search = { view: "routing" };
    state.keys = [
      makeKey({
        label: "My preserved connection",
        slug: "my-openai",
        credential_source: { type: "personal" },
      }),
    ];
    render(<KeysPage />);
    await screen.findByRole("button", { name: "Expand OpenAI connections" });
    expandConnections();
    expect(screen.getByText("https://api.openai.com")).toBeVisible();
    expect(screen.getByText("my-openai")).toBeVisible();
    expect(
      screen.queryByText("Details", { selector: "summary" }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByRole("link", {
        name: "Configure My preserved connection (Personal)",
      }),
    ).toHaveAttribute("href", "/keys/$keyId:key-1");
    expect(
      screen.getByRole("link", { name: "View all OpenAI service details" }),
    ).toHaveAttribute("href", "/keys/services/$groupId:catalog:cat-1");
    expect(
      screen.queryByRole("button", { name: "Individual cards" }),
    ).not.toBeInTheDocument();
  });

  it("opens saved pool routing and members inside the expanded pool card", async () => {
    state.search = { view: "routing", tab: "pools" };
    state.keys = [makeKey({ credential_source: { type: "personal" } })];
    render(<KeysPage />);
    expect(await screen.findByText("My pool")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "View route" }));
    expect(
      screen.getByRole("table", { name: "My pool route members" }),
    ).toBeVisible();
    expect(screen.getByText("My OpenAI")).toBeVisible();
    expect(screen.getByText("/api/v1/proxy/s/my-pool")).toBeVisible();
    expect(screen.getByRole("button", { name: "Create pool" })).toBeVisible();
    expect(
      screen.getByRole("button", { name: "Configure pool" }),
    ).toBeVisible();
  });

  it.each([undefined, "team-id"])(
    "opens a linked pool under its owner (%s) without opening an editor",
    async (org) => {
      state.search = { tab: "pools", pool: "pool-one", org };
      state.keys = [makeKey({ credential_source: { type: "personal" } })];
      render(<KeysPage />);
      expect(
        await screen.findByRole("table", { name: "My pool route members" }),
      ).toBeVisible();
      expect(mockPoolOwner).toHaveBeenLastCalledWith(org);
      expect(
        screen.getByRole("button", { name: "Configure pool" }),
      ).toBeVisible();
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    },
  );

  it("omits oauth2 and api_key credential pills from service cards", () => {
    state.keys = [
      makeKey({
        id: "oauth-service",
        label: "OAuth Service",
        slug: "oauth-service",
        credential_type: "oauth2",
      }),
      makeKey({
        id: "api-key-service",
        label: "API Key Service",
        slug: "api-key-service",
        credential_type: "api_key",
      }),
      makeKey({
        id: "bearer-service",
        label: "Bearer Service",
        slug: "bearer-service",
        credential_type: "bearer",
      }),
    ];

    render(<KeysPage />);
    expandConnections();

    expect(screen.queryByText("oauth2")).not.toBeInTheDocument();
    expect(screen.queryByText("api_key")).not.toBeInTheDocument();
    expect(screen.queryByText("bearer")).not.toBeInTheDocument();
    expect(screen.queryByText("Direct")).not.toBeInTheDocument();
  });

  it("shows the empty state with an Add Your First Service CTA when there are no services", async () => {
    state.keys = [];

    render(<KeysPage />);

    expect(screen.getByText("No AI services yet")).toBeInTheDocument();
    // Both buttons render when the list is empty: the empty-state CTA
    // "Add your first service" (intentionally kept per the canon decision
    // to leave empty-state copy alone) and the toolbar CTA "Connect
    // Service" (canon verb for top-level CTAs).
    expect(
      screen.getByRole("button", { name: /add your first service/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /connect service/i }),
    ).toBeInTheDocument();
  });

  it("renders a loading skeleton while keys are loading", () => {
    state.keysLoading = true;

    const { container } = render(<KeysPage />);

    expect(screen.queryByText("No AI services yet")).not.toBeInTheDocument();
    expect(container.querySelectorAll(".animate-pulse").length).toBeGreaterThan(
      0,
    );
  });

  it("shows an error banner with retry when the keys query fails", () => {
    state.keysError = new Error("boom");

    render(<KeysPage />);

    expect(screen.getByText(/failed to load services/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /retry/i })).toBeInTheDocument();
  });

  it("keeps personal and platform counterparts visible together without the removed Filters menu", async () => {
    state.keys = [
      makeKey(),
      makeKey({
        id: "platform",
        label: "Platform counterpart",
        slug: "platform-openai",
        auto_connected: true,
        credential_binding: "platform",
      }),
    ];
    render(<KeysPage />);
    fireEvent.click(
      screen.getByRole("button", { name: "Auto-connected services: hidden" }),
    );
    expect(
      screen.queryByRole("button", { name: "Filters" }),
    ).not.toBeInTheDocument();
    expandConnections();
    expect(screen.getByText("My OpenAI")).toBeVisible();
    expect(screen.getByText("Platform counterpart")).toBeVisible();
  });

  it("hides auto-connected endpoint URLs in table rows without changing normal rows", async () => {
    localStorage.setItem("nyxid-view-mode:keys-services", "table");
    state.keys = [
      makeKey({
        id: "manual-table",
        label: "Manual Table",
        endpoint_url: "https://manual-table.example/v1",
      }),
      makeKey({
        id: "auto-table",
        label: "Auto Table",
        endpoint_url: "https://platform-table.internal/v1",
        auto_connected: true,
      }),
    ];

    try {
      render(<KeysPage />);
      fireEvent.click(
        screen.getByRole("button", { name: "Auto-connected services: hidden" }),
      );
      fireEvent.click(
        screen.getByRole("button", { name: "Service view: Personal" }),
      );

      expect(
        screen.getByText("https://manual-table.example/v1"),
      ).toBeInTheDocument();
      expect(
        screen.queryByText("https://platform-table.internal/v1"),
      ).not.toBeInTheDocument();
      expect(screen.getAllByText("Platform managed").length).toBeGreaterThan(0);
    } finally {
      localStorage.removeItem("nyxid-view-mode:keys-services");
    }
  });

  it("keeps organization identity and role in the connection table", () => {
    state.keys = [
      makeKey({
        id: "org-key",
        label: "Org OpenAI",
        credential_source: {
          type: "org",
          org_id: "org-1",
          org_name: "Acme Org",
          avatar_url: null,
          role: "member",
          allowed: true,
        },
      }),
    ];

    render(<KeysPage />);
    fireEvent.click(
      screen.getByRole("button", { name: "Service view: Personal" }),
    );
    expandConnections();

    expect(screen.getAllByText("Acme Org").length).toBeGreaterThan(0);
    expect(screen.getByTitle("Acme Org · Organization · member")).toBeVisible();
  });

  it("opens the Add Key dialog when the toolbar Connect Service button is clicked", async () => {
    const user = userEvent.setup();
    state.keys = [makeKey()];

    render(<KeysPage />);

    expect(screen.queryByTestId("add-key-dialog")).not.toBeInTheDocument();

    // Toolbar Connect Service button (the empty-state CTA isn't shown when keys exist).
    await user.click(screen.getByRole("button", { name: /connect service/i }));

    expect(screen.getByTestId("add-key-dialog")).toBeInTheDocument();
  });

  it("opens reconnect mode from a recoverable OAuth service card without navigating", async () => {
    const user = userEvent.setup();
    state.keys = [
      makeKey({
        id: "oauth-key",
        label: "Needs Reauth",
        credential_type: "oauth2",
        auth_method: "oauth2",
        status: "refresh_failed",
      }),
    ];

    render(<KeysPage />);
    expandConnections();

    await user.click(screen.getByRole("button", { name: /reconnect/i }));

    expect(screen.getByTestId("add-key-dialog")).toHaveAttribute(
      "data-reconnect",
      "oauth-key",
    );
    expect(mockNavigate).not.toHaveBeenCalledWith({
      to: "/keys/$keyId",
      params: { keyId: "oauth-key" },
    });
  });

  it("badges and reconnects an OAuth service whose credential row is missing", async () => {
    const user = userEvent.setup();
    state.keys = [
      makeKey({
        id: "missing-oauth-key",
        label: "Missing OAuth",
        api_key_id: null,
        credential_type: "none",
        auth_method: "oauth2",
        status: "active",
        is_active: false,
        credential_missing: true,
      }),
    ];

    render(<KeysPage />);
    expandConnections();

    expect(screen.getByText("Disabled")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /reconnect/i }));
    expect(screen.getByTestId("add-key-dialog")).toHaveAttribute(
      "data-reconnect",
      "missing-oauth-key",
    );
  });

  it("badges an OAuth credential with known expired connection health", () => {
    state.keys = [
      makeKey({
        id: "expired-oauth",
        label: "Expired OAuth",
        credential_type: "oauth2",
        auth_method: "oauth2",
        status: "active",
        connection_status: "expired",
      }),
    ];

    render(<KeysPage />);
    expandConnections();

    expect(screen.getByText("Reconnect needed")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /reconnect/i }),
    ).toBeInTheDocument();
  });

  it("labels pending OAuth service cards as continue authentication", async () => {
    const user = userEvent.setup();
    state.keys = [
      makeKey({
        id: "pending-oauth",
        label: "Pending OAuth",
        credential_type: "oauth2",
        auth_method: "oauth2",
        status: "pending_auth",
      }),
    ];

    render(<KeysPage />);
    expandConnections();

    await user.click(
      screen.getByRole("button", { name: /continue authentication/i }),
    );

    expect(screen.getByTestId("add-key-dialog")).toHaveAttribute(
      "data-reconnect",
      "pending-oauth",
    );
  });

  it("hides reconnect for read-only org-inherited OAuth services", () => {
    state.keys = [
      makeKey({
        id: "org-oauth",
        label: "Org OAuth",
        credential_type: "oauth2",
        auth_method: "oauth2",
        status: "failed",
        credential_source: {
          type: "org",
          org_id: "org-1",
          org_name: "Acme Org",
          avatar_url: null,
          role: "member",
          allowed: true,
        },
      }),
    ];

    render(<KeysPage />);
    fireEvent.click(
      screen.getByRole("button", { name: "Service view: Personal" }),
    );
    expandConnections();

    expect(
      screen.queryByRole("button", { name: /reconnect/i }),
    ).not.toBeInTheDocument();
    expect(screen.getByTitle("Acme Org · Organization · member")).toBeVisible();
  });

  it("does not offer reconnect when organization access is denied despite an admin role", () => {
    state.keys = [
      makeKey({
        credential_type: "oauth2",
        auth_method: "oauth2",
        status: "failed",
        credential_source: {
          type: "org",
          org_id: "denied-org",
          org_name: "Restricted",
          role: "admin",
          allowed: false,
          avatar_url: null,
        },
      }),
    ];
    render(<KeysPage />);
    fireEvent.click(
      screen.getByRole("button", { name: "Service view: Personal" }),
    );
    expandConnections();
    expect(screen.getByText("No access")).toBeVisible();
    expect(
      screen.queryByRole("button", { name: /Reconnect/ }),
    ).not.toBeInTheDocument();
  });

  it("switches to the Agent Keys tab and mounts the API key table + usage dashboard", async () => {
    const user = userEvent.setup();
    state.keys = [makeKey()];

    render(<KeysPage />);

    await user.click(screen.getByRole("tab", { name: "Agent Keys" }));

    // setTab navigates with the chosen tab value.
    expect(mockNavigate).toHaveBeenCalledWith({
      to: "/keys",
      search: { tab: "nyxid" },
      replace: true,
    });
  });

  it("renders the Agent Keys tab content when ?tab=nyxid is in the URL", () => {
    state.search = { tab: "nyxid" };
    state.keys = [makeKey()];

    render(<KeysPage />);

    expect(screen.getByTestId("api-key-table")).toBeInTheDocument();
    expect(screen.getByTestId("api-key-usage-dashboard")).toBeInTheDocument();
    // The toolbar CTA on the nyxid tab is "Create API Key", not "Connect Service".
    expect(
      screen.getByRole("button", { name: /create api key/i }),
    ).toBeInTheDocument();
  });

  it("auto-opens the prefilled Add Key dialog from a ?slug= deep link and clears the slug", async () => {
    state.search = { slug: "anthropic" };

    render(<KeysPage />);

    await waitFor(() => {
      expect(screen.getByTestId("add-key-dialog")).toBeInTheDocument();
    });
    expect(screen.getByTestId("add-key-dialog")).toHaveAttribute(
      "data-prefill",
      "anthropic",
    );
    // The effect rewrites the URL to drop the slug and pin the services tab.
    expect(mockNavigate).toHaveBeenCalledWith({
      to: "/keys",
      search: { tab: "services" },
      replace: true,
    });
  });

  it("shows connection metadata within table rows with explicit configuration navigation", async () => {
    // useViewMode reads localStorage to default the services tab into table mode.
    localStorage.setItem("nyxid-view-mode:keys-services", "table");
    const user = userEvent.setup();
    state.keys = [makeKey({ id: "key-1", label: "My OpenAI", slug: "openai" })];

    try {
      render(<KeysPage />);

      // Table view renders column headers instead of cards.
      expect(
        screen.getByRole("columnheader", { name: "Configuration" }),
      ).toBeInTheDocument();
      expect(
        screen.getByRole("columnheader", { name: "Connection / Slug" }),
      ).toBeInTheDocument();

      await user.click(
        screen.getByRole("button", {
          name: "Details for My OpenAI (Personal)",
        }),
      );

      expect(
        screen.getByRole("button", {
          name: "Details for My OpenAI (Personal)",
        }),
      ).toHaveAttribute("aria-expanded", "true");
      expect(
        screen.getByRole("link", { name: "Configure My OpenAI (Personal)" }),
      ).toHaveAttribute("href", "/keys/$keyId:key-1");
    } finally {
      localStorage.removeItem("nyxid-view-mode:keys-services");
    }
  });

  it("opens reconnect mode from a table row action without navigating to detail", async () => {
    localStorage.setItem("nyxid-view-mode:keys-services", "table");
    const user = userEvent.setup();
    state.keys = [
      makeKey({
        id: "table-oauth",
        label: "Table OAuth",
        credential_type: "oauth2",
        auth_method: "oauth2",
        status: "failed",
      }),
    ];

    try {
      render(<KeysPage />);

      await user.click(screen.getByRole("button", { name: /reconnect/i }));

      expect(screen.getByTestId("add-key-dialog")).toHaveAttribute(
        "data-reconnect",
        "table-oauth",
      );
      expect(mockNavigate).not.toHaveBeenCalledWith({
        to: "/keys/$keyId",
        params: { keyId: "table-oauth" },
      });
    } finally {
      localStorage.removeItem("nyxid-view-mode:keys-services");
    }
  });

  it("auto-opens the create-key dialog from a ?action=create-key deep link", async () => {
    state.search = { action: "create-key", tab: "nyxid" };

    render(<KeysPage />);

    await waitFor(() => {
      expect(screen.getByTestId("api-key-create-dialog")).toHaveAttribute(
        "data-open",
        "true",
      );
    });
  });
});
