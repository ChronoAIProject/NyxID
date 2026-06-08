import type { ReactNode } from "react";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { KeyInfo } from "@/types/keys";

const { mockNavigate, state } = vi.hoisted(() => ({
  mockNavigate: vi.fn(),
  // Mutable containers populated per-test before render.
  state: {
    search: {} as { tab?: string; slug?: string; action?: string },
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
  }: {
    readonly children: ReactNode;
    readonly to: string;
    readonly params?: Record<string, string>;
  }) => (
    <a href={params ? `${to}:${Object.values(params).join("/")}` : to}>
      {children}
    </a>
  ),
  useNavigate: () => mockNavigate,
  useSearch: () => state.search,
}));

vi.mock("@/hooks/use-keys", () => ({
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

// Heavy children — stubbed to assert wiring (open state, presence), not driven.
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

function makeKey(overrides: Partial<KeyInfo> = {}): KeyInfo {
  return {
    id: "key-1",
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
    state.search = {};
    state.keys = [];
    state.keysLoading = false;
    state.keysError = null;
    state.userServices = [];
    state.nodes = [];
  });

  it("defaults to the External Services tab and lists personal services as a flat grid", () => {
    state.keys = [
      makeKey({ id: "key-1", label: "My OpenAI", slug: "openai" }),
      makeKey({ id: "key-2", label: "My GitHub", slug: "github" }),
    ];

    render(<KeysPage />);

    // The personal-only path renders cards directly with no section header.
    expect(screen.getByText("My OpenAI")).toBeInTheDocument();
    expect(screen.getByText("My GitHub")).toBeInTheDocument();
    // Default tab is "services", so the Agent Keys table is not mounted.
    expect(screen.queryByTestId("api-key-table")).not.toBeInTheDocument();
    // Proxy slug for an HTTP service is rendered as /proxy/s/{slug}.
    expect(screen.getByText("/proxy/s/openai")).toBeInTheDocument();
  });

  it("shows the empty state with an Add Service CTA when there are no services", async () => {
    state.keys = [];

    render(<KeysPage />);

    expect(screen.getByText("No AI services yet")).toBeInTheDocument();
    const addButtons = screen.getAllByRole("button", { name: /add service/i });
    expect(addButtons.length).toBeGreaterThan(0);
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

    expect(
      screen.getByText(/failed to load services/i),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /retry/i })).toBeInTheDocument();
  });

  it("hides auto-connected services until the toggle is enabled", async () => {
    const user = userEvent.setup();
    state.keys = [
      makeKey({ id: "user-1", label: "Manual Key", auto_connected: false }),
      makeKey({
        id: "auto-1",
        label: "Auto Key",
        auto_connected: true,
        source_app_name: "Claude Code",
      }),
    ];

    render(<KeysPage />);

    // Auto-connected hidden by default.
    expect(screen.getByText("Manual Key")).toBeInTheDocument();
    expect(screen.queryByText("Auto Key")).not.toBeInTheDocument();
    // The toggle label reflects the auto-connected count.
    expect(screen.getByText("Show auto-connected (1)")).toBeInTheDocument();

    await user.click(screen.getByRole("switch"));

    expect(screen.getByText("Auto Key")).toBeInTheDocument();
  });

  it("groups org-inherited services into a labelled section with a role badge", () => {
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

    // Org section header (h3) + role badge come from the org credential source.
    // ("Acme Org" also appears inside the stubbed OrgAvatar, so scope to the heading.)
    expect(
      screen.getByRole("heading", { name: "Acme Org" }),
    ).toBeInTheDocument();
    expect(screen.getByTestId("role-badge")).toHaveTextContent("member");
    expect(screen.getByText("Shared from organization")).toBeInTheDocument();
    // Member (non-admin) org cards are flagged View-Only.
    expect(screen.getByText("View-Only")).toBeInTheDocument();
  });

  it("opens the Add Key dialog when the toolbar Add Service button is clicked", async () => {
    const user = userEvent.setup();
    state.keys = [makeKey()];

    render(<KeysPage />);

    expect(screen.queryByTestId("add-key-dialog")).not.toBeInTheDocument();

    // Toolbar Add Service button (the empty-state CTA isn't shown when keys exist).
    await user.click(screen.getByRole("button", { name: /add service/i }));

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

    expect(
      screen.queryByRole("button", { name: /reconnect/i }),
    ).not.toBeInTheDocument();
    expect(screen.getByText("View-Only")).toBeInTheDocument();
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
    // The toolbar CTA on the nyxid tab is "Create API Key", not "Add Service".
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

  it("renders services as a table and navigates to the key detail when a row is clicked", async () => {
    // useViewMode reads localStorage to default the services tab into table mode.
    localStorage.setItem("nyxid-view-mode:keys-services", "table");
    const user = userEvent.setup();
    state.keys = [makeKey({ id: "key-1", label: "My OpenAI", slug: "openai" })];

    try {
      render(<KeysPage />);

      // Table view renders column headers instead of cards.
      expect(
        screen.getByRole("columnheader", { name: "Endpoint" }),
      ).toBeInTheDocument();
      expect(
        screen.getByRole("columnheader", { name: "Proxy Slug" }),
      ).toBeInTheDocument();

      await user.click(screen.getByText("My OpenAI"));

      expect(mockNavigate).toHaveBeenCalledWith({
        to: "/keys/$keyId",
        params: { keyId: "key-1" },
      });
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
      expect(
        screen.getByTestId("api-key-create-dialog"),
      ).toHaveAttribute("data-open", "true");
    });
  });
});
