import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactNode } from "react";
import type { CatalogEntry, KeyInfo } from "@/types/keys";
import { resetSkillCatalog } from "@/lib/assistant/skills";

const mocks = vi.hoisted(() => ({
  useCatalog: vi.fn(),
  useKeys: vi.fn(),
}));

vi.mock("@/hooks/use-keys", () => ({
  useCatalog: mocks.useCatalog,
  useKeys: mocks.useKeys,
}));

vi.mock("@tanstack/react-router", () => ({
  Link: ({
    to,
    params,
    search,
    children,
    className,
  }: {
    readonly to: string;
    readonly params?: Record<string, string>;
    readonly search?: Record<string, string>;
    readonly children?: ReactNode;
    readonly className?: string;
  }) => (
    <a
      href="#"
      className={className}
      data-to={to}
      data-params={JSON.stringify(params ?? null)}
      data-search={JSON.stringify(search ?? null)}
    >
      {children}
    </a>
  ),
}));

import { PluginsView } from "./plugins-view";

const keys = [
  {
    id: "key-1",
    label: "OpenAI",
    slug: "openai",
    endpoint_url: "https://api.openai.com/v1",
    credential_type: "api_key",
    catalog_service_slug: "openai",
    service_type: "http",
  },
  {
    id: "key-2",
    label: "Internal Admin API",
    slug: "internal-admin",
    endpoint_url: "https://internal.example.com",
    credential_type: "bearer_token",
    catalog_service_slug: null,
    service_type: "http",
  },
] as unknown as readonly KeyInfo[];

const catalog = [
  {
    slug: "github",
    name: "GitHub",
    description: "Repos, issues, and PRs.",
    base_url: "https://api.github.com",
    provider_type: "oauth2",
    service_type: "http",
    requires_credential: true,
    requires_gateway_url: false,
    token_exchange_credential_fields: null,
  },
  {
    slug: "openai",
    name: "OpenAI",
    description: "GPT models via your own API key.",
    base_url: "https://api.openai.com/v1",
    provider_type: null,
    service_type: "http",
    requires_credential: true,
    requires_gateway_url: false,
    token_exchange_credential_fields: null,
  },
  {
    slug: "stripe",
    name: "Stripe",
    description: "Payments data and operations.",
    base_url: "https://api.stripe.com/v1",
    provider_type: null,
    service_type: "http",
    requires_credential: true,
    requires_gateway_url: false,
    token_exchange_credential_fields: null,
  },
] as unknown as readonly CatalogEntry[];

function mockLoaded({
  keyRows = keys,
  catalogRows = catalog,
}: {
  readonly keyRows?: readonly KeyInfo[];
  readonly catalogRows?: readonly CatalogEntry[];
} = {}) {
  mocks.useKeys.mockReturnValue({
    data: keyRows,
    isLoading: false,
    error: null,
    refetch: vi.fn(),
  });
  mocks.useCatalog.mockReturnValue({
    data: catalogRows,
    isLoading: false,
    error: null,
    refetch: vi.fn(),
  });
}

describe("PluginsView", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetSkillCatalog();
    mockLoaded();
  });

  it("splits connectors into real connections and unconnected catalog entries", () => {
    render(<PluginsView />);
    expect(screen.getByText("Added")).toBeInTheDocument();
    expect(screen.getByText("Available to add")).toBeInTheDocument();
    // Both /keys rows are Added — including the custom (non-catalog) one.
    expect(screen.getAllByText("Connected")).toHaveLength(2);
    expect(screen.getByText("Internal Admin API")).toBeInTheDocument();
    // Connected catalog slugs are removed from Available: exactly one OpenAI card.
    expect(screen.getAllByText("OpenAI")).toHaveLength(1);
    expect(screen.getByText("GitHub")).toBeInTheDocument();
    expect(screen.getByText("Stripe")).toBeInTheDocument();
    // Auth-kind meta derived from the catalog entry shape.
    expect(screen.getByText("oauth")).toBeInTheDocument();
  });

  it("deep-links Connect to the /keys add-service flow for the catalog slug", () => {
    render(<PluginsView />);
    const connect = screen.getAllByRole("link", { name: "Connect" })[0];
    expect(connect).toHaveAttribute("data-to", "/keys");
    expect(connect).toHaveAttribute("data-search", '{"slug":"github"}');
  });

  it("links Manage to the key detail page", () => {
    render(<PluginsView />);
    const manage = screen.getAllByRole("link", { name: "Manage" })[0];
    expect(manage).toHaveAttribute("data-to", "/keys/$keyId");
    expect(manage).toHaveAttribute("data-params", '{"keyId":"key-1"}');
  });

  it("shows loading skeletons while either query is in flight", () => {
    mocks.useKeys.mockReturnValue({
      data: undefined,
      isLoading: true,
      error: null,
      refetch: vi.fn(),
    });
    render(<PluginsView />);
    expect(
      screen.getByRole("status", { name: "Loading the plugin catalog" }),
    ).toBeInTheDocument();
    expect(screen.queryByText("Added")).not.toBeInTheDocument();
  });

  it("shows an error banner and retries both queries", async () => {
    const user = userEvent.setup();
    const refetchKeys = vi.fn();
    const refetchCatalog = vi.fn();
    mocks.useKeys.mockReturnValue({
      data: undefined,
      isLoading: false,
      error: new Error("boom"),
      refetch: refetchKeys,
    });
    mocks.useCatalog.mockReturnValue({
      data: undefined,
      isLoading: false,
      error: null,
      refetch: refetchCatalog,
    });
    render(<PluginsView />);
    expect(
      screen.getByText("Failed to load the plugin catalog. Please try again."),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Retry" }));
    expect(refetchKeys).toHaveBeenCalled();
    expect(refetchCatalog).toHaveBeenCalled();
  });

  it("keeps the Added section with an empty state when nothing is connected", () => {
    mockLoaded({ keyRows: [] });
    render(<PluginsView />);
    expect(screen.getByText("Added")).toBeInTheDocument();
    expect(
      screen.getByText(/No connected services yet/),
    ).toBeInTheDocument();
    // All catalog entries are available when nothing is connected.
    expect(screen.getAllByRole("link", { name: "Connect" })).toHaveLength(3);
  });

  it("filters the catalog by search", async () => {
    const user = userEvent.setup();
    render(<PluginsView />);
    await user.type(
      screen.getByPlaceholderText("Search the catalog..."),
      "github",
    );
    expect(screen.getByText("GitHub")).toBeInTheDocument();
    expect(screen.queryByText("OpenAI")).not.toBeInTheDocument();
    expect(screen.queryByText("Added")).not.toBeInTheDocument();
  });

  it("shows the no-match state when the search hits nothing", async () => {
    const user = userEvent.setup();
    render(<PluginsView />);
    await user.type(
      screen.getByPlaceholderText("Search the catalog..."),
      "zzz-no-such-plugin",
    );
    expect(
      screen.getByText("No plugins match this search."),
    ).toBeInTheDocument();
  });

  it("renders the skill catalog with the add-your-own card and installs a skill", async () => {
    const user = userEvent.setup();
    render(<PluginsView />);
    await user.click(screen.getByRole("tab", { name: "Skills" }));
    expect(screen.getByText("Chrono AI Service Manual")).toBeInTheDocument();
    expect(screen.getAllByText("Installed")).toHaveLength(1);
    expect(screen.getByText("Add your own skill")).toBeInTheDocument();
    expect(screen.queryByText("GitHub")).not.toBeInTheDocument();
    // Author · version meta line for available skills.
    expect(screen.getByText("Ornn · v1.2")).toBeInTheDocument();

    await user.click(
      screen.getAllByRole("button", { name: "Install" })[0] as HTMLElement,
    );
    expect(screen.getAllByText("Installed")).toHaveLength(2);
  });

  it("keeps installed skills across unmount and remount", async () => {
    const user = userEvent.setup();
    const first = render(<PluginsView />);
    await user.click(screen.getByRole("tab", { name: "Skills" }));
    await user.click(
      screen.getAllByRole("button", { name: "Install" })[0] as HTMLElement,
    );
    expect(screen.getAllByText("Installed")).toHaveLength(2);
    first.unmount();

    render(<PluginsView />);
    await user.click(screen.getByRole("tab", { name: "Skills" }));
    expect(screen.getAllByText("Installed")).toHaveLength(2);
  });
});
