import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { toast } from "sonner";
import type { ReactNode } from "react";
import type { CatalogEntry, KeyInfo } from "@/types/keys";
import { resetSkillCatalog } from "@/lib/assistant/skills";
import { ApiError } from "@/lib/api-client";

const mocks = vi.hoisted(() => ({
  useCatalog: vi.fn(),
  useCatalogEntry: vi.fn(),
  useKeys: vi.fn(),
  useKey: vi.fn(),
  useUpdateKey: vi.fn(),
  useDeleteKey: vi.fn(),
  useUpdateExternalApiKey: vi.fn(),
}));

// The full add-service dialog has its own test suite and many hooks; stub it
// to a marker so this test only asserts Connect opens it with the right slug.
vi.mock("@/components/dashboard/add-key-dialog", () => ({
  AddKeyDialog: ({
    open,
    prefillSlug,
  }: {
    readonly open: boolean;
    readonly prefillSlug?: string;
  }) =>
    open ? (
      <div role="dialog" aria-label="Add service" data-prefill={prefillSlug}>
        Add service dialog
      </div>
    ) : null,
}));

vi.mock("@/hooks/use-keys", () => ({
  useCatalog: mocks.useCatalog,
  useCatalogEntry: mocks.useCatalogEntry,
  useKeys: mocks.useKeys,
  useKey: mocks.useKey,
  useUpdateKey: mocks.useUpdateKey,
  useDeleteKey: mocks.useDeleteKey,
  useUpdateExternalApiKey: mocks.useUpdateExternalApiKey,
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
    // Manage-modal hooks: a fully-shaped key + no-op mutations.
    mocks.useKey.mockReturnValue({
      data: {
        ...keys[0],
        api_key_id: "api-key-1",
        status: "active",
        is_active: true,
        proxy_url: "http://localhost:3011/api/v1/proxy/s/openai",
        last_used_at: null,
        granted_scopes: null,
      },
      isLoading: false,
    });
    mocks.useCatalogEntry.mockReturnValue({
      data: catalog.find((entry) => entry.slug === "openai"),
    });
    mocks.useUpdateKey.mockReturnValue({ mutate: vi.fn(), isPending: false });
    mocks.useDeleteKey.mockReturnValue({ mutate: vi.fn(), isPending: false });
    mocks.useUpdateExternalApiKey.mockReturnValue({
      mutate: vi.fn(),
      isPending: false,
    });
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

  it("connects straight from the card, with no intermediate detail step", async () => {
    const user = userEvent.setup();
    render(<PluginsView />);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    // The card itself is the only CTA — there is no nested Connect button.
    await user.click(screen.getByRole("button", { name: "Connect GitHub" }));
    const dialog = await screen.findByRole("dialog", { name: "Add service" });
    expect(dialog).toHaveAttribute("data-prefill", "github");
  });

  it("connects from the keyboard", async () => {
    const user = userEvent.setup();
    render(<PluginsView />);
    screen.getByRole("button", { name: "Connect Stripe" }).focus();
    await user.keyboard("{Enter}");
    expect(
      await screen.findByRole("dialog", { name: "Add service" }),
    ).toHaveAttribute("data-prefill", "stripe");
  });

  it("opens the manage modal straight from a connected card", async () => {
    const user = userEvent.setup();
    render(<PluginsView />);
    // Conditional mount: no key detail is fetched before the card is clicked.
    expect(mocks.useKey).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Manage OpenAI" }));
    expect(mocks.useKey).toHaveBeenCalledWith("key-1");
    expect(await screen.findByRole("dialog")).toBeInTheDocument();
    const advanced = screen.getByRole("link", { name: /advanced settings/i });
    expect(advanced).toHaveAttribute("data-to", "/keys/$keyId");
    expect(advanced).toHaveAttribute("data-params", '{"keyId":"key-1"}');
  });

  it("toggles the connection active state through the modal", async () => {
    const mutate = vi.fn();
    mocks.useUpdateKey.mockReturnValue({ mutate, isPending: false });
    const user = userEvent.setup();
    render(<PluginsView />);
    await user.click(screen.getByRole("button", { name: "Manage OpenAI" }));
    await user.click(
      await screen.findByRole("switch", { name: /toggle connection enabled/i }),
    );
    expect(mutate).toHaveBeenCalledWith(
      { keyId: "key-1", is_active: false },
      expect.anything(),
    );
  });

  it("replaces the stored credential from inside the modal", async () => {
    const mutate = vi.fn();
    mocks.useUpdateExternalApiKey.mockReturnValue({
      mutate,
      isPending: false,
    });
    const user = userEvent.setup();
    render(<PluginsView />);
    await user.click(screen.getByRole("button", { name: "Manage OpenAI" }));
    await user.click(await screen.findByRole("button", { name: "Replace" }));
    await user.type(screen.getByLabelText("New credential"), "sk-new-secret");
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(mutate).toHaveBeenCalledWith(
      { keyId: "api-key-1", credential: "sk-new-secret" },
      expect.anything(),
    );
  });

  it("requires an explicit confirmation before revoking", async () => {
    const mutate = vi.fn();
    mocks.useDeleteKey.mockReturnValue({ mutate, isPending: false });
    const user = userEvent.setup();
    render(<PluginsView />);
    await user.click(screen.getByRole("button", { name: "Manage OpenAI" }));
    // First Revoke click only opens the confirmation dialog — no delete yet.
    // (Dialog copy and cancel behaviour are covered in
    // manage-connection-modal.test.tsx.)
    await user.click(await screen.findByRole("button", { name: "Revoke" }));
    expect(mutate).not.toHaveBeenCalled();
    // The confirm click sends the delete with the key id.
    const confirm = screen
      .getAllByRole("button", { name: "Revoke" })
      .at(-1) as HTMLElement;
    await user.click(confirm);
    expect(mutate).toHaveBeenCalledWith("key-1", expect.anything());
  });

  it("surfaces a 11500 payload and retries the assistant delete with token scope", async () => {
    const mutate = vi.fn();
    mocks.useDeleteKey.mockReturnValue({ mutate, isPending: false });
    mocks.useCatalogEntry.mockReturnValue({
      data: {
        ...catalog[0],
        revocation: { revokes_grant: true },
      },
    });
    mocks.useKey.mockReturnValue({
      data: {
        ...keys[0],
        catalog_service_slug: "github",
        catalog_service_name: "GitHub",
        status: "active",
        is_active: true,
        last_used_at: null,
        granted_scopes: null,
        revocation: { revokes_grant: true },
      },
      isLoading: false,
    });

    const user = userEvent.setup();
    render(<PluginsView />);
    await user.click(screen.getByRole("button", { name: "Manage OpenAI" }));
    await user.click(await screen.findByRole("button", { name: "Revoke" }));
    const confirm = screen
      .getAllByRole("button", { name: "Revoke" })
      .at(-1) as HTMLElement;
    await user.click(confirm);

    act(() => {
      mutate.mock.calls[0]![1].onError(
        new ApiError(409, {
          error: "grant_cascade_confirmation_required",
          error_code: 11500,
          message: "Confirmation required",
          details: {
            provider_slug: "github",
            provider_name: "GitHub",
            revokes_grant: true,
            siblings: [
              {
                user_service_id: "service-2",
                name: "GitHub Issues",
                slug: "github-issues",
              },
            ],
            unaffected_other_app: [],
            token_scope_available: true,
          },
        }),
      );
    });

    await user.click(
      await screen.findByRole("button", { name: "Remove only this service" }),
    );
    expect(mutate.mock.calls[1]![0]).toEqual({
      keyId: "key-1",
      grantScope: "token",
    });
  });

  it("hides mutation controls for an auto-connected (platform-managed) key", async () => {
    mocks.useKey.mockReturnValue({
      data: {
        ...keys[0],
        status: "active",
        is_active: true,
        proxy_url: "http://localhost:3011/api/v1/proxy/s/openai",
        last_used_at: null,
        granted_scopes: null,
        auto_connected: true,
      },
      isLoading: false,
    });
    const user = userEvent.setup();
    render(<PluginsView />);
    await user.click(screen.getByRole("button", { name: "Manage OpenAI" }));
    expect(await screen.findByRole("dialog")).toBeInTheDocument();
    expect(screen.queryByRole("switch")).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Revoke" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Replace" }),
    ).not.toBeInTheDocument();
    // The advanced-settings escape hatch remains.
    expect(
      screen.getByRole("link", { name: /advanced settings/i }),
    ).toBeInTheDocument();
  });

  it("stacks every credential of a multi-connection service in one modal", async () => {
    mockLoaded({
      keyRows: [
        ...keys,
        {
          id: "key-3",
          label: "OpenAI (org)",
          slug: "openai-2",
          endpoint_url: "https://api.openai.com/v1",
          credential_type: "api_key",
          catalog_service_slug: "openai",
          service_type: "http",
        },
      ] as unknown as readonly KeyInfo[],
    });
    // Each panel fetches its own connection.
    mocks.useKey.mockImplementation((keyId: string) => ({
      data: {
        id: keyId,
        label: keyId === "key-1" ? "OpenAI" : "OpenAI (org)",
        credential_type: "api_key",
        status: "active",
        is_active: true,
        last_used_at: null,
        granted_scopes: null,
      },
      isLoading: false,
    }));
    const user = userEvent.setup();
    render(<PluginsView />);
    await user.click(screen.getByRole("button", { name: "Manage OpenAI" }));

    const dialog = await screen.findByRole("dialog");
    // Both connections are managed in place — no keys-list hand-off.
    expect(mocks.useKey).toHaveBeenCalledWith("key-1");
    expect(mocks.useKey).toHaveBeenCalledWith("key-3");
    expect(within(dialog).getByText("2 connections")).toBeInTheDocument();
    expect(within(dialog).getAllByRole("switch")).toHaveLength(2);
    expect(
      within(dialog).getAllByRole("link", { name: /advanced settings/i }),
    ).toHaveLength(2);
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

  it("stays usable when the catalog fails: toast, rendered view, inline retry", async () => {
    // "FE do not block any error": the failure is a toast, never a view
    // replacement. The view keeps its search box and sections, and the retry
    // affordance lives inside the working view.
    const user = userEvent.setup();
    const toastSpy = vi.spyOn(toast, "error");
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
      screen.queryByText("Failed to load the plugin catalog. Please try again."),
    ).not.toBeInTheDocument();
    expect(toastSpy).toHaveBeenCalledWith(
      "Could not load plugins",
      expect.objectContaining({ id: "assistant-plugins-load-failed" }),
    );
    // The view is still standing: heading, search, and the honest empty copy.
    expect(screen.getByText("Added")).toBeInTheDocument();
    expect(screen.getByRole("textbox")).toBeEnabled();
    expect(
      screen.getByText("Connected services could not be loaded right now."),
    ).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Retry" }));
    expect(refetchKeys).toHaveBeenCalled();
    expect(refetchCatalog).toHaveBeenCalled();
    toastSpy.mockRestore();
  });

  it("keeps the Added section with an empty state when nothing is connected", () => {
    mockLoaded({ keyRows: [] });
    render(<PluginsView />);
    expect(screen.getByText("Added")).toBeInTheDocument();
    expect(screen.getByText(/No connected services yet/)).toBeInTheDocument();
    // All catalog entries are available when nothing is connected.
    expect(screen.getAllByRole("button", { name: /^Connect / })).toHaveLength(
      3,
    );
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
      screen.getAllByRole("button", { name: /^Install / })[0] as HTMLElement,
    );
    expect(screen.getAllByText("Installed")).toHaveLength(2);
  });

  it("leaves an installed skill's card inert (no management flow yet)", async () => {
    const user = userEvent.setup();
    render(<PluginsView />);
    await user.click(screen.getByRole("tab", { name: "Skills" }));
    // Available skills are clickable; the installed one exposes no action.
    expect(
      screen.getByRole("button", { name: /^Install / }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /^Manage / }),
    ).not.toBeInTheDocument();
  });

  it("keeps installed skills across unmount and remount", async () => {
    const user = userEvent.setup();
    const first = render(<PluginsView />);
    await user.click(screen.getByRole("tab", { name: "Skills" }));
    await user.click(
      screen.getAllByRole("button", { name: /^Install / })[0] as HTMLElement,
    );
    expect(screen.getAllByText("Installed")).toHaveLength(2);
    first.unmount();

    render(<PluginsView />);
    await user.click(screen.getByRole("tab", { name: "Skills" }));
    expect(screen.getAllByText("Installed")).toHaveLength(2);
  });
});
