import type { ReactNode } from "react";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError } from "@/lib/api-client";
import type { KeyInfo } from "@/types/keys";

const {
  hooks,
  mockCopyToClipboard,
  mockNavigate,
  mockToastError,
  mockToastSuccess,
  routerState,
} = vi.hoisted(() => {
  // The page-level fixtures/mocks are reassigned per-test in beforeEach.
  const hooks = {
    key: {
      data: undefined as KeyInfo | undefined,
      isLoading: false,
      error: null as unknown,
      refetch: vi.fn(),
    },
    updateKey: vi.fn(),
    deleteKey: vi.fn(),
    updateEndpoint: vi.fn(),
    updateExternalApiKey: vi.fn(),
    updateUserService: vi.fn(),
    catalogEntry: undefined as unknown,
    nodes: [] as { id: string; name: string }[],
  };
  return {
    hooks,
    mockCopyToClipboard: vi.fn(),
    mockNavigate: vi.fn(),
    mockToastError: vi.fn(),
    mockToastSuccess: vi.fn(),
    routerState: { search: {} as Record<string, unknown> },
  };
});

vi.mock("@tanstack/react-router", () => ({
  Link: ({
    children,
    to,
    ...props
  }: {
    readonly children: ReactNode;
    readonly to: string;
  }) => (
    <a href={to} {...props}>
      {children}
    </a>
  ),
  useNavigate: () => mockNavigate,
  useParams: () => ({ keyId: "key-1" }),
  useSearch: () => routerState.search,
}));

vi.mock("@/hooks/use-keys", () => ({
  useKey: () => hooks.key,
  useUpdateKey: () => ({ mutate: hooks.updateKey, isPending: false }),
  useDeleteKey: () => ({ mutate: hooks.deleteKey, isPending: false }),
  useUpdateEndpoint: () => ({ mutate: hooks.updateEndpoint, isPending: false }),
  useUpdateExternalApiKey: () => ({
    mutate: hooks.updateExternalApiKey,
    isPending: false,
  }),
  useUpdateUserService: () => ({
    mutate: hooks.updateUserService,
    isPending: false,
  }),
  useCatalogEntry: () => ({ data: hooks.catalogEntry }),
}));

vi.mock("@/hooks/use-nodes", () => ({
  useNodes: () => ({ data: hooks.nodes }),
}));

// `useBreadcrumbLabel` lives in the dashboard layout module, which pulls in
// the whole sidebar/auth/command-palette graph. The page only needs the
// breadcrumb side-effect, so stub it with a no-op.
vi.mock("@/components/layout/dashboard-layout", () => ({
  useBreadcrumbLabel: () => {},
}));

// Heavy editable children that live in their own files — stub them so the
// tests exercise the page's own logic, not these widgets' internals.
vi.mock("@/components/dashboard/routing-section", () => ({
  RoutingSection: ({ readOnly }: { readonly readOnly?: boolean }) => (
    <div data-testid="routing-section" data-readonly={String(Boolean(readOnly))}>
      Routing
    </div>
  ),
}));
vi.mock("@/components/shared/default-headers-editor", () => ({
  DefaultHeadersEditor: () => <div data-testid="default-headers-editor" />,
}));
vi.mock("@/components/shared/ws-frame-injections-editor", () => ({
  WsFrameInjectionsEditor: () => <div data-testid="ws-frame-editor" />,
}));
vi.mock("@/components/dashboard/ssh-service-instructions", () => ({
  SshServiceInstructions: () => <div data-testid="ssh-instructions" />,
}));
vi.mock("@/components/dashboard/add-key-dialog", () => ({
  AddKeyDialog: ({
    open,
    reconnectKey,
  }: {
    readonly open: boolean;
    readonly reconnectKey?: KeyInfo | null;
  }) =>
    open ? (
      <div data-testid="add-key-dialog" data-reconnect={reconnectKey?.id ?? ""} />
    ) : null,
}));

vi.mock("sonner", () => ({
  toast: { success: mockToastSuccess, error: mockToastError },
}));

vi.mock("@/lib/utils", async () => {
  const actual =
    await vi.importActual<typeof import("@/lib/utils")>("@/lib/utils");
  return { ...actual, copyToClipboard: mockCopyToClipboard };
});

import { KeyDetailPage } from "./key-detail";

/** A fully-populated, user-managed, non-SSH HTTP key. */
function makeKey(overrides: Partial<KeyInfo> = {}): KeyInfo {
  return {
    id: "key-1",
    label: "My OpenAI",
    slug: "openai-x",
    endpoint_url: "https://api.openai.com/v1",
    endpoint_id: "ep-1",
    api_key_id: "ak-1",
    credential_type: "api_key",
    auth_method: "bearer",
    auth_key_name: "Authorization",
    status: "active",
    catalog_service_id: "cat-1",
    catalog_service_slug: "openai",
    catalog_service_name: "OpenAI",
    node_id: null,
    node_priority: 0,
    is_active: true,
    custom_user_agent: null,
    default_request_headers: null,
    ws_frame_injections: [],
    auto_connected: false,
    source_app_id: null,
    source_app_name: null,
    expires_at: null,
    last_used_at: null,
    error_message: null,
    created_at: "2026-01-01T00:00:00Z",
    service_type: "http",
    ssh_host: null,
    ssh_port: null,
    ssh_ca_public_key: null,
    ssh_allowed_principals: null,
    ssh_certificate_ttl_minutes: null,
    openapi_spec_url: null,
    credential_source: { type: "personal" },
    permission_setup_url: null,
    permission_setup_scopes: null,
    ...overrides,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  routerState.search = {};
  hooks.key = {
    data: makeKey(),
    isLoading: false,
    error: null,
    refetch: vi.fn(),
  };
  hooks.catalogEntry = undefined;
  hooks.nodes = [];
  mockCopyToClipboard.mockResolvedValue(undefined);
});

describe("KeyDetailPage — load states", () => {
  it("renders skeletons while the key query is loading", () => {
    hooks.key = {
      data: undefined,
      isLoading: true,
      error: null,
      refetch: vi.fn(),
    };

    const { container } = render(<KeyDetailPage />);

    // Loading branch renders skeleton placeholders (1 header + 4 grid), no label.
    expect(container.querySelectorAll(".animate-pulse").length).toBe(5);
    expect(screen.queryByText("My OpenAI")).not.toBeInTheDocument();
  });

  it("shows a Not Found header and the API error message with retry on error", async () => {
    const refetch = vi.fn();
    hooks.key = {
      data: undefined,
      isLoading: false,
      error: new ApiError(404, {
        error: "not_found",
        error_code: 1404,
        message: "Key does not exist",
      }),
      refetch,
    };

    render(<KeyDetailPage />);

    expect(
      screen.getByRole("heading", { name: "Key Not Found" }),
    ).toBeInTheDocument();
    expect(screen.getByText("Key does not exist")).toBeInTheDocument();

    await userEvent.setup().click(screen.getByRole("button", { name: "Retry" }));
    expect(refetch).toHaveBeenCalledTimes(1);
  });
});

describe("KeyDetailPage — core rendering", () => {
  it("renders the key's identity, endpoint, credential and service config from the hook", () => {
    render(<KeyDetailPage />);

    // Label heading + catalog-name/proxy-path subtitle.
    expect(screen.getByText("My OpenAI")).toBeInTheDocument();
    expect(
      screen.getByText("OpenAI -- /proxy/s/openai-x"),
    ).toBeInTheDocument();

    // Endpoint card shows the target URL.
    expect(
      screen.getByText("https://api.openai.com/v1"),
    ).toBeInTheDocument();

    // API Key card: credential type. Both the credential status badge and the
    // service-availability badge read "Active", so just assert the pair exists.
    expect(screen.getByText("Type: api_key")).toBeInTheDocument();
    expect(screen.getAllByText("Active")).toHaveLength(2);

    // Service card: auth method + auth key surfaced from the hook, and the
    // slug-derived proxy path.
    expect(screen.getByText("bearer")).toBeInTheDocument();
    expect(screen.getByText("Authorization")).toBeInTheDocument();
    expect(screen.getByText("/proxy/s/openai-x")).toBeInTheDocument();

    // Real routing widget mounted in editable (non-readOnly) mode.
    expect(screen.getByTestId("routing-section")).toHaveAttribute(
      "data-readonly",
      "false",
    );
  });

  it("renders the credential blocked notice when the service is active but its credential is expired", () => {
    hooks.key.data = makeKey({ status: "expired" });

    render(<KeyDetailPage />);

    // deriveServiceBadge → active service + non-active credential = Unavailable.
    expect(screen.getByText("Unavailable")).toBeInTheDocument();
    expect(
      screen.getByText(/Real requests will fail until the credential is restored/),
    ).toBeInTheDocument();
  });

  it("opens reconnect mode for failed OAuth-backed services", async () => {
    const user = userEvent.setup();
    hooks.key.data = makeKey({
      status: "failed",
      credential_type: "oauth2",
      auth_method: "oauth2",
      error_message: "OAuth denied",
    });
    hooks.catalogEntry = {
      slug: "openai",
      provider_type: "oauth2",
    };

    render(<KeyDetailPage />);

    await user.click(screen.getByRole("button", { name: /reconnect/i }));

    expect(screen.getByTestId("add-key-dialog")).toHaveAttribute(
      "data-reconnect",
      "key-1",
    );
  });

  it("uses continue authentication copy for pending OAuth-backed services", async () => {
    const user = userEvent.setup();
    hooks.key.data = makeKey({
      status: "pending_auth",
      credential_type: "oauth2",
      auth_method: "oauth2",
    });
    hooks.catalogEntry = {
      slug: "openai",
      provider_type: "oauth2",
    };

    render(<KeyDetailPage />);

    await user.click(
      screen.getByRole("button", { name: /continue authentication/i }),
    );

    expect(screen.getByTestId("add-key-dialog")).toHaveAttribute(
      "data-reconnect",
      "key-1",
    );
  });

  it("hides reconnect for read-only org-inherited OAuth services", () => {
    hooks.key.data = makeKey({
      status: "failed",
      credential_type: "oauth2",
      auth_method: "oauth2",
      credential_source: {
        type: "org",
        org_id: "org-1",
        org_name: "Acme Org",
        avatar_url: null,
        role: "member",
        allowed: true,
      },
    });
    hooks.catalogEntry = {
      slug: "openai",
      provider_type: "oauth2",
    };

    render(<KeyDetailPage />);

    expect(
      screen.queryByRole("button", { name: /reconnect/i }),
    ).not.toBeInTheDocument();
    expect(screen.getByText("Shared from Acme Org")).toBeInTheDocument();
  });
});

describe("KeyDetailPage — edit flows", () => {
  it("renames the label via useUpdateKey with the trimmed value", async () => {
    const user = userEvent.setup();
    render(<KeyDetailPage />);

    // The pencil next to the label opens the inline editor (first edit button).
    const editButtons = screen.getAllByRole("button");
    // Open label editor: the heading's sibling pencil button.
    const labelHeading = screen.getByText("My OpenAI");
    const pencil = labelHeading.parentElement?.querySelector("button");
    expect(pencil).toBeTruthy();
    await user.click(pencil as HTMLElement);

    const input = screen.getByDisplayValue("My OpenAI");
    await user.clear(input);
    await user.type(input, "  Renamed Key  ");
    // Save (the check button) commits.
    await user.keyboard("{Enter}");

    expect(hooks.updateKey).toHaveBeenCalledTimes(1);
    expect(hooks.updateKey.mock.calls[0]![0]).toEqual({
      keyId: "key-1",
      label: "Renamed Key",
    });
    // Drive the success path.
    hooks.updateKey.mock.calls[0]![1].onSuccess();
    expect(mockToastSuccess).toHaveBeenCalledWith("Label updated");
    expect(editButtons.length).toBeGreaterThan(0);
  });

  it("updates the endpoint URL via useUpdateEndpoint", async () => {
    const user = userEvent.setup();
    render(<KeyDetailPage />);

    const endpointCode = screen.getByText("https://api.openai.com/v1");
    const pencil = endpointCode.parentElement?.querySelector("button");
    await user.click(pencil as HTMLElement);

    const input = screen.getByDisplayValue("https://api.openai.com/v1");
    await user.clear(input);
    await user.type(input, "https://proxy.example.com/v1");
    // Two icon buttons: cancel (X) then save (Check). Save is the last.
    const iconButtons = within(
      input.closest("div") as HTMLElement,
    ).getAllByRole("button");
    await user.click(iconButtons[iconButtons.length - 1]!);

    expect(hooks.updateEndpoint).toHaveBeenCalledTimes(1);
    expect(hooks.updateEndpoint.mock.calls[0]![0]).toEqual({
      endpointId: "ep-1",
      url: "https://proxy.example.com/v1",
    });
    hooks.updateEndpoint.mock.calls[0]![1].onSuccess();
    expect(mockToastSuccess).toHaveBeenCalledWith("Endpoint updated");
  });

  it("saves an OpenAPI spec URL via useUpdateEndpoint", async () => {
    const user = userEvent.setup();
    render(<KeyDetailPage />);

    // "Not set" badge marks the empty OpenAPI spec section.
    const notSet = screen.getByText("Not set");
    const pencil = notSet.parentElement?.querySelector("button");
    await user.click(pencil as HTMLElement);

    const input = document.querySelector<HTMLInputElement>(
      'input[type="url"]',
    );
    expect(input).toBeTruthy();
    await user.type(input as HTMLInputElement, "https://api.openai.com/openapi.json");
    const iconButtons = within(
      (input as HTMLInputElement).closest("div") as HTMLElement,
    ).getAllByRole("button");
    await user.click(iconButtons[iconButtons.length - 1]!);

    expect(hooks.updateEndpoint).toHaveBeenCalledTimes(1);
    expect(hooks.updateEndpoint.mock.calls[0]![0]).toEqual({
      endpointId: "ep-1",
      openapi_spec_url: "https://api.openai.com/openapi.json",
    });
    hooks.updateEndpoint.mock.calls[0]![1].onSuccess();
    expect(mockToastSuccess).toHaveBeenCalledWith("OpenAPI spec URL saved");
  });

  it("rotates the credential via useUpdateExternalApiKey", async () => {
    const user = userEvent.setup();
    render(<KeyDetailPage />);

    await user.click(
      screen.getByRole("button", { name: /Rotate Credentials/i }),
    );
    const input = screen.getByPlaceholderText("Enter new credential");
    await user.type(input, "sk-new-secret");
    await user.click(screen.getByRole("button", { name: "Save" }));

    expect(hooks.updateExternalApiKey).toHaveBeenCalledTimes(1);
    expect(hooks.updateExternalApiKey.mock.calls[0]![0]).toEqual({
      keyId: "ak-1",
      credential: "sk-new-secret",
    });
    hooks.updateExternalApiKey.mock.calls[0]![1].onSuccess();
    expect(mockToastSuccess).toHaveBeenCalledWith("Credential rotated");
  });

  it("toggles the service inactive via useUpdateUserService and surfaces an error toast on failure", async () => {
    const user = userEvent.setup();
    render(<KeyDetailPage />);

    await user.click(screen.getByRole("button", { name: /Deactivate/i }));

    expect(hooks.updateUserService).toHaveBeenCalledTimes(1);
    expect(hooks.updateUserService.mock.calls[0]![0]).toEqual({
      serviceId: "key-1",
      is_active: false,
    });
    // Drive the error path → toast.error with the API message.
    hooks.updateUserService.mock.calls[0]![1].onError(
      new ApiError(403, {
        error: "forbidden",
        error_code: 1403,
        message: "Not allowed",
      }),
    );
    expect(mockToastError).toHaveBeenCalledWith("Not allowed");
  });

  it("saves a custom User-Agent via useUpdateUserService", async () => {
    const user = userEvent.setup();
    render(<KeyDetailPage />);

    // The User-Agent row shows a "Passthrough (default)" badge with a pencil.
    const uaLabel = screen.getByText("User-Agent");
    const uaRow = uaLabel.parentElement as HTMLElement;
    const pencil = within(uaRow).getAllByRole("button")[0]!;
    await user.click(pencil);

    const input = screen.getByPlaceholderText("Passthrough (default)");
    await user.type(input, "MyAgent/1.0");
    // Save = first icon button (Check) inside the editing row.
    const saveBtn = within(input.closest("div") as HTMLElement).getAllByRole(
      "button",
    )[0]!;
    await user.click(saveBtn);

    expect(hooks.updateUserService).toHaveBeenCalledTimes(1);
    expect(hooks.updateUserService.mock.calls[0]![0]).toEqual({
      serviceId: "key-1",
      custom_user_agent: "MyAgent/1.0",
    });
    hooks.updateUserService.mock.calls[0]![1].onSuccess();
    expect(mockToastSuccess).toHaveBeenCalledWith("Custom User-Agent saved");
  });
});

describe("KeyDetailPage — delete flow", () => {
  it("opens the confirm dialog and deletes via useDeleteKey then navigates to /keys", async () => {
    const user = userEvent.setup();
    render(<KeyDetailPage />);

    await user.click(screen.getByRole("button", { name: /^Delete$/i }));

    // Dialog renders in a portal under document.body.
    const dialog = await screen.findByRole("dialog");
    expect(
      within(dialog).getByText("Delete Service"),
    ).toBeInTheDocument();

    await user.click(within(dialog).getByRole("button", { name: "Delete" }));

    expect(hooks.deleteKey).toHaveBeenCalledTimes(1);
    expect(hooks.deleteKey.mock.calls[0]![0]).toBe("key-1");
    // Drive success → toast + navigation back to the keys list.
    hooks.deleteKey.mock.calls[0]![1].onSuccess();
    expect(mockToastSuccess).toHaveBeenCalledWith("Key deleted");
    expect(mockNavigate).toHaveBeenCalledWith({ to: "/keys", search: {} });
  });
});

describe("KeyDetailPage — API usage section", () => {
  it("copies the proxy base URL to the clipboard", async () => {
    const user = userEvent.setup();
    render(<KeyDetailPage />);

    const proxyUrl = `${window.location.origin}/api/v1/proxy/s/openai-x`;
    // The proxy URL is shown inside a <pre> with a copy button beside it.
    const pre = screen.getByText(proxyUrl);
    const copyBtn = pre.parentElement?.querySelector("button");
    await user.click(copyBtn as HTMLElement);

    await waitFor(() =>
      expect(mockCopyToClipboard).toHaveBeenCalledWith(proxyUrl),
    );
    expect(mockToastSuccess).toHaveBeenCalledWith("Proxy URL copied");
  });
});

describe("KeyDetailPage — org read-only branch", () => {
  it("shows the shared-from-org banner and hides edit/delete controls for non-admin org members", () => {
    hooks.key.data = makeKey({
      credential_source: {
        type: "org",
        org_id: "org-1",
        org_name: "Acme Org",
        role: "member",
        allowed: true,
      },
    });

    render(<KeyDetailPage />);

    expect(screen.getByText("Shared from Acme Org")).toBeInTheDocument();
    // Read-only: no Delete or Deactivate buttons, routing widget read-only.
    expect(
      screen.queryByRole("button", { name: /^Delete$/i }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /Deactivate/i }),
    ).not.toBeInTheDocument();
    expect(screen.getByTestId("routing-section")).toHaveAttribute(
      "data-readonly",
      "true",
    );
    // Label heading still shown but with no inline edit pencil.
    const heading = screen.getByText("My OpenAI");
    expect(heading.parentElement?.querySelector("button")).toBeNull();
  });
});

describe("KeyDetailPage — auto-connected branch", () => {
  it("renders the platform-managed service details card instead of editors", () => {
    hooks.key.data = makeKey({
      auto_connected: true,
      api_key_id: null,
      credential_type: "none",
      auth_method: "none",
      source_app_name: "Wizard",
    });

    render(<KeyDetailPage />);

    expect(screen.getByText("Service Details")).toBeInTheDocument();
    expect(screen.getByText("Connected via Wizard")).toBeInTheDocument();
    expect(
      screen.getByText("None (no credentials required)"),
    ).toBeInTheDocument();
    // No node bound → routing shows Direct.
    expect(screen.getByText("Direct")).toBeInTheDocument();
    // Editable sections (e.g. the rotate button) are absent.
    expect(
      screen.queryByRole("button", { name: /Rotate Credentials/i }),
    ).not.toBeInTheDocument();
  });
});

describe("KeyDetailPage — provider connect callback", () => {
  it("toasts success and clears the provider_status search param", async () => {
    routerState.search = { provider_status: "success" };

    render(<KeyDetailPage />);

    await waitFor(() =>
      expect(mockToastSuccess).toHaveBeenCalledWith(
        "Service connected successfully",
      ),
    );
    expect(mockNavigate).toHaveBeenCalledWith({
      to: ".",
      search: {},
      replace: true,
    });
  });

  it("toasts the backend message and clears the search param when provider_status is error", async () => {
    routerState.search = {
      provider_status: "error",
      message: "OAuth was denied by the provider",
    };

    render(<KeyDetailPage />);

    await waitFor(() =>
      expect(mockToastError).toHaveBeenCalledWith(
        "OAuth was denied by the provider",
      ),
    );
    expect(mockToastSuccess).not.toHaveBeenCalled();
    expect(mockNavigate).toHaveBeenCalledWith({
      to: ".",
      search: {},
      replace: true,
    });
  });

  it("falls back to a generic message when provider_status is error with no message", async () => {
    routerState.search = { provider_status: "error" };

    render(<KeyDetailPage />);

    // `search.message ?? "Failed to connect service"` — the fallback fires
    // when the callback carried no human-readable reason.
    await waitFor(() =>
      expect(mockToastError).toHaveBeenCalledWith("Failed to connect service"),
    );
  });
});

describe("KeyDetailPage — node-routed extras", () => {
  it("renders the node setup helper and resolves the bound node name", () => {
    hooks.nodes = [{ id: "node-9", name: "Edge Node" }];
    hooks.key.data = makeKey({ node_id: "node-9" });

    render(<KeyDetailPage />);

    // Node Setup helper card appears for node-routed non-SSH keys.
    expect(screen.getByText("Node Setup")).toBeInTheDocument();
    // The setup command embeds the slug.
    expect(
      screen.getByText(/nyxid node credentials setup --service openai-x/),
    ).toBeInTheDocument();
  });
});

describe("KeyDetailPage — SSH branch", () => {
  it("renders SSH connection details and routes the terminal button to the SSH terminal", async () => {
    hooks.key.data = makeKey({
      service_type: "ssh",
      api_key_id: null,
      credential_type: "none",
      auth_method: "none",
      catalog_service_id: "ssh-svc-1",
      endpoint_url: "ssh://bastion.example.com",
      ssh_host: "bastion.example.com",
      ssh_port: 2222,
      ssh_ca_public_key: "ssh-ed25519 AAAACA...",
      ssh_allowed_principals: ["deploy", "ops"],
      ssh_certificate_ttl_minutes: 15,
    });

    const user = userEvent.setup();
    render(<KeyDetailPage />);

    // SSH connection card surfaces host:port, TTL and principals.
    expect(screen.getByText("bastion.example.com:2222")).toBeInTheDocument();
    expect(screen.getByText("15 minutes")).toBeInTheDocument();
    expect(screen.getByText("deploy")).toBeInTheDocument();
    expect(screen.getByText("ops")).toBeInTheDocument();
    // Connection instructions card mounts the SSH instructions child.
    expect(screen.getByTestId("ssh-instructions")).toBeInTheDocument();
    // The non-SSH-only API usage / OpenAPI sections are absent.
    expect(screen.queryByText("API Usage")).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /Terminal/i }));
    expect(mockNavigate).toHaveBeenCalledWith({
      to: "/ssh/$serviceId/terminal",
      params: { serviceId: "ssh-svc-1" },
      search: { principal: "deploy", returnKeyId: "key-1" },
    });
  });
});

describe("KeyDetailPage — Lark permission setup", () => {
  it("renders the permissions deep link and pre-selected scopes", () => {
    hooks.key.data = makeKey({
      permission_setup_url: "https://open.larksuite.com/app/abc/permission",
      permission_setup_scopes: ["im:message", "contact:user.id:readonly"],
    });

    render(<KeyDetailPage />);

    const link = screen.getByRole("link", { name: /Open Permissions Page/i });
    expect(link).toHaveAttribute(
      "href",
      "https://open.larksuite.com/app/abc/permission",
    );
    expect(screen.getByText("im:message")).toBeInTheDocument();
    expect(screen.getByText("contact:user.id:readonly")).toBeInTheDocument();
  });
});
