import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CatalogEntry, KeyInfo } from "@/types/keys";
import { ApiError } from "@/lib/api-client";
import { SPEC_CATALOG_SLUGS } from "@/components/service-icons";
import { useOAuthPopupStore } from "@/stores/oauth-popup-store";
import { AddKeyDialog } from "./add-key-dialog";

const {
  catalog,
  createKeyMutate,
  createKeyMutateAsync,
  createApiKeyMutate,
  initiateOAuthMutateAsync,
  initiateDeviceCodeMutateAsync,
  pollDeviceCodeMutate,
  mockApiDelete,
  mockHardRedirect,
  mockNavigate,
  pendingKeyStatus,
  pendingLastAuthorizedAt,
  popupReceiverOptions,
  toastFns,
} = vi.hoisted(() => ({
  catalog: {
    entries: [] as unknown[],
    allEntries: null as unknown[] | null,
    requests: [] as boolean[],
  },
  // Status the OAuth step's placeholder-key poll observes. `null` = still
  // `pending_auth` (the user hasn't finished at the provider yet).
  pendingKeyStatus: { value: null as string | null },
  pendingLastAuthorizedAt: { value: null as string | null },
  popupReceiverOptions: {
    current: null as null | { onRetry?: () => Promise<unknown> },
  },
  createKeyMutate: vi.fn(),
  createKeyMutateAsync: vi.fn(),
  // Wave-aha-1 A4+ — the verify step auto-mints an Agent Key. The mock
  // intentionally swallows the call without firing onSuccess so the
  // dialog stays in the "minting" phase; the test only needs to assert
  // the verify step renders, not that the mint succeeded.
  createApiKeyMutate: vi.fn(),
  initiateOAuthMutateAsync: vi.fn(),
  initiateDeviceCodeMutateAsync: vi.fn(),
  pollDeviceCodeMutate: vi.fn(),
  mockApiDelete: vi.fn(),
  mockHardRedirect: vi.fn(),
  mockNavigate: vi.fn(),
  toastFns: { success: vi.fn(), error: vi.fn() },
}));

vi.mock("@/hooks/use-keys", () => ({
  useCatalog: (options: { readonly includeAll?: boolean } = {}) => {
    const includeAll = options.includeAll ?? false;
    catalog.requests.push(includeAll);
    return {
      data: includeAll
        ? (catalog.allEntries ?? catalog.entries)
        : catalog.entries,
      isLoading: false,
    };
  },
  useCreateKey: () => ({
    mutate: createKeyMutate,
    mutateAsync: createKeyMutateAsync,
    isPending: false,
  }),
  KEY_AUTH_ACTIVE: "active",
  KEY_AUTH_FAILED: "failed",
  // The OAuth step polls the placeholder key while the user authorizes in
  // another tab. Tests drive the observed status through `pendingKeyStatus`.
  useKeyAuthorizationStatus: () => ({
    data: pendingKeyStatus.value
      ? {
          status: pendingKeyStatus.value,
          last_authorized_at: pendingLastAuthorizedAt.value,
        }
      : undefined,
  }),
}));

vi.mock("@/hooks/use-api-keys", () => ({
  useCreateApiKey: () => ({
    mutate: createApiKeyMutate,
    isPending: false,
  }),
}));

vi.mock("@/hooks/use-providers", () => ({
  useInitiateOAuth: () => ({
    mutateAsync: initiateOAuthMutateAsync,
    isPending: false,
  }),
  useInitiateDeviceCode: () => ({
    mutateAsync: initiateDeviceCodeMutateAsync,
    isPending: false,
  }),
  usePollDeviceCode: () => ({
    mutate: pollDeviceCodeMutate,
    isPending: false,
  }),
}));

// Popup protocol behavior has focused hook/page coverage. These legacy dialog
// tests intentionally render without a QueryClientProvider.
vi.mock("@/hooks/use-oauth-popup", () => ({
  useOAuthPopupReceiver: (options: { onRetry?: () => Promise<unknown> }) => {
    popupReceiverOptions.current = options;
  },
}));

vi.mock("@/lib/api-client", async () => {
  const actual =
    await vi.importActual<typeof import("@/lib/api-client")>(
      "@/lib/api-client",
    );
  return {
    ...actual,
    api: {
      ...actual.api,
      delete: mockApiDelete,
    },
  };
});

vi.mock("@/lib/navigation", () => ({
  hardRedirect: mockHardRedirect,
}));

// RoutingStep reads online nodes; OwnerPicker reads admin orgs. Empty
// arrays keep the node picker empty and hide the owner picker entirely
// (it renders null without an admin org), so neither pulls in extra deps.
vi.mock("@/hooks/use-nodes", () => ({
  useNodes: () => ({ data: [], isLoading: false }),
}));
vi.mock("@/hooks/use-orgs", () => ({
  useOrgs: () => ({ data: [] }),
}));
// The BYO credential form (OAuthCredentialsStep) renders
// OAuthCallbackGuidance, whose useRuntimeConfig would need a real
// QueryClientProvider. The platform one-click tests reach that form.
vi.mock("@/hooks/use-runtime-config", () => ({
  useRuntimeConfig: () => ({ data: undefined, isLoading: false }),
}));

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
}));

vi.mock("sonner", () => ({ toast: toastFns }));

const OPENAI_ENTRY = {
  slug: "openai",
  name: "OpenAI",
  description: "OpenAI API",
  base_url: "https://api.openai.com/v1",
  auth_method: "bearer",
  auth_key_name: "Authorization",
  requires_gateway_url: false,
  service_type: "http",
} as unknown as CatalogEntry;

const OAUTH_ENTRY = {
  ...OPENAI_ENTRY,
  slug: "github",
  name: "GitHub",
  provider_config_id: "provider-oauth",
  provider_type: "oauth2",
  auth_method: "oauth2",
  auth_key_name: "Authorization",
} as unknown as CatalogEntry;

const DEVICE_CODE_ENTRY = {
  ...OPENAI_ENTRY,
  slug: "codex",
  name: "Codex",
  provider_config_id: "provider-device",
  provider_type: "device_code",
  auth_method: "oauth2",
  auth_key_name: "Authorization",
  device_code_format: "openai",
} as unknown as CatalogEntry;

function makeReconnectKey(overrides: Partial<KeyInfo> = {}): KeyInfo {
  return {
    id: "existing-service-1",
    label: "Existing GitHub",
    slug: "github-existing",
    endpoint_url: "https://api.github.com",
    endpoint_id: "endpoint-1",
    api_key_id: "api-key-1",
    credential_type: "oauth2",
    auth_method: "oauth2",
    auth_key_name: "Authorization",
    status: "failed",
    catalog_service_id: "catalog-1",
    catalog_service_slug: "github",
    catalog_service_name: "GitHub",
    node_id: null,
    node_priority: 0,
    is_active: true,
    ws_frame_injections: [],
    auto_connected: false,
    expires_at: null,
    last_used_at: null,
    last_authorized_at: "2026-01-01T00:00:00Z",
    error_message: "Previous authorization failed",
    created_at: "2026-01-01T00:00:00Z",
    service_type: "http",
    ssh_host: null,
    ssh_port: null,
    ssh_ca_public_key: null,
    ssh_allowed_principals: null,
    ssh_certificate_ttl_minutes: null,
    ...overrides,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  window.history.replaceState({}, "", "/");
  catalog.entries = [OPENAI_ENTRY];
  catalog.allEntries = null;
  catalog.requests = [];
  pendingKeyStatus.value = null;
  pendingLastAuthorizedAt.value = null;
  popupReceiverOptions.current = null;
  createKeyMutateAsync.mockResolvedValue({ id: "created-service-1" });
  initiateOAuthMutateAsync.mockResolvedValue({
    authorization_url: "https://provider.example/oauth",
  });
  initiateDeviceCodeMutateAsync.mockResolvedValue({
    user_code: "ABCD-EFGH",
    verification_uri: "https://provider.example/device",
    state: "device-state",
    expires_in: 900,
    interval: 5,
  });
  mockApiDelete.mockResolvedValue(undefined);
  useOAuthPopupStore.setState({ attempt: null });
});

afterEach(() => {
  vi.restoreAllMocks();
});

/**
 * Type into an input addressed by its DOM id. Labels here are dynamic, and
 * the dialog renders in a Radix portal under document.body (not the render
 * container), so query the whole document.
 */
async function typeInto(
  user: ReturnType<typeof userEvent.setup>,
  id: string,
  value: string,
) {
  const el = document.querySelector<HTMLInputElement>(`#${id}`);
  if (!el) throw new Error(`input #${id} not found`);
  await user.type(el, value);
}

describe("AddKeyDialog — custom endpoint path", () => {
  it("opens the assistant custom-service path with every mappable field prefilled", async () => {
    const user = userEvent.setup();
    render(
      <AddKeyDialog
        open
        onOpenChange={vi.fn()}
        prefillCustom={{
          name: "Build API",
          endpointUrl: "https://build.example.test/v1",
          authMethod: "header",
          authKeyName: "X-Build-Key",
        }}
      />,
    );

    await user.click(
      await screen.findByRole("button", { name: /Next: Enter Credentials/i }),
    );

    expect(document.querySelector("#add-key-label")).toHaveValue("Build API");
    expect(document.querySelector("#add-key-endpoint")).toHaveValue(
      "https://build.example.test/v1",
    );
    expect(document.querySelector("#add-key-auth-key")).toHaveValue(
      "X-Build-Key",
    );
    expect(document.querySelector("#add-key-auth-method")).toHaveTextContent(
      "Header",
    );
  });

  it("creates a key from a hand-entered endpoint and navigates to it", async () => {
    createKeyMutate.mockImplementation((_params, opts) => {
      opts?.onSuccess?.({ id: "new-key-1" });
    });
    const user = userEvent.setup();
    render(<AddKeyDialog open onOpenChange={vi.fn()} />);

    // Catalog step → choose "Custom Endpoint".
    await user.click(screen.getByRole("button", { name: /Custom Endpoint/i }));
    // Routing step → keep the default "Direct" routing.
    await user.click(
      screen.getByRole("button", { name: /Next: Enter Credentials/i }),
    );

    // Form step → fill the custom endpoint, label and credential.
    await typeInto(user, "add-key-label", "My Custom API");
    await typeInto(user, "add-key-credential", "sk-custom-123");
    await typeInto(user, "add-key-endpoint", "https://my.endpoint/v1");

    await user.click(screen.getByRole("button", { name: "Connect Service" }));

    await waitFor(() => expect(createKeyMutate).toHaveBeenCalledTimes(1));
    expect(createKeyMutate).toHaveBeenCalledWith(
      {
        credential: "sk-custom-123",
        label: "My Custom API",
        endpoint_url: "https://my.endpoint/v1",
        auth_method: "bearer",
        auth_key_name: "Authorization",
      },
      expect.anything(),
    );
    // Wave-aha-1 A4: success path now transitions to the inline `verify`
    // step instead of toasting + navigating. The dialog stays open so
    // the user sees their first 200 (or a precise failure diagnosis)
    // right here. View-details / Done buttons handle the close + nav.
    // Verify-step DialogTitle: brand icon + quoted service name (no verb).
    // "Connected" now lives in the DialogDescription + inline body copy,
    // not the heading — so we just assert the service name appears.
    await waitFor(() =>
      expect(
        screen.getByRole("heading", {
          name: (n) => /My Custom API/i.test(n),
        }),
      ).toBeInTheDocument(),
    );
    expect(toastFns.success).not.toHaveBeenCalledWith("Key created");
    expect(mockNavigate).not.toHaveBeenCalled();
  });

  it("surfaces the API error message when key creation fails", async () => {
    createKeyMutate.mockImplementation((_params, opts) => {
      opts?.onError?.(
        new ApiError(400, {
          error: "bad_request",
          error_code: 1000,
          message: "Endpoint URL is invalid",
        }),
      );
    });
    const user = userEvent.setup();
    render(<AddKeyDialog open onOpenChange={vi.fn()} />);

    await user.click(screen.getByRole("button", { name: /Custom Endpoint/i }));
    await user.click(
      screen.getByRole("button", { name: /Next: Enter Credentials/i }),
    );
    await typeInto(user, "add-key-label", "Broken");
    await typeInto(user, "add-key-credential", "sk-x");
    // Well-formed URL so the client-side format check passes and the
    // mocked backend rejection is what surfaces.
    await typeInto(user, "add-key-endpoint", "https://api.example.com/v1");
    await user.click(screen.getByRole("button", { name: "Connect Service" }));

    await waitFor(() =>
      expect(toastFns.error).toHaveBeenCalledWith("Endpoint URL is invalid"),
    );
    expect(mockNavigate).not.toHaveBeenCalled();
  });

  it("blocks submit and shows an inline error for a TLD-less endpoint URL", async () => {
    const user = userEvent.setup();
    render(<AddKeyDialog open onOpenChange={vi.fn()} />);

    await user.click(screen.getByRole("button", { name: /Custom Endpoint/i }));
    await user.click(
      screen.getByRole("button", { name: /Next: Enter Credentials/i }),
    );
    await typeInto(user, "add-key-label", "Typo URL");
    await typeInto(user, "add-key-credential", "sk-x");
    await typeInto(user, "add-key-endpoint", "https://www.");

    expect(
      screen.getByText(/Must be a full URL with a domain/),
    ).toBeInTheDocument();
    expect(screen.getByLabelText(/Endpoint URL/)).toHaveAttribute(
      "aria-invalid",
      "true",
    );
    expect(
      screen.getByRole("button", { name: "Connect Service" }),
    ).toBeDisabled();
    expect(createKeyMutate).not.toHaveBeenCalled();
  });
});

describe("AddKeyDialog — catalog template path", () => {
  it("resolves an action prefill by exact slug from the full catalog", async () => {
    catalog.entries = [OAUTH_ENTRY];
    catalog.allEntries = [{ ...OAUTH_ENTRY, slug: "api-github" }];
    render(
      <AddKeyDialog
        open
        onOpenChange={vi.fn()}
        prefillSlug="api-github"
        prefillIncludeAllCatalog
      />,
    );

    expect(
      await screen.findByRole("heading", {
        name: "Configure routing for GitHub",
      }),
    ).toBeInTheDocument();
    expect(catalog.requests).toContain(true);
  });

  it("stays on the catalog step instead of substituting a slug alias", () => {
    catalog.entries = [OAUTH_ENTRY];
    catalog.allEntries = [OAUTH_ENTRY];
    render(
      <AddKeyDialog
        open
        onOpenChange={vi.fn()}
        prefillSlug="api-github"
        prefillIncludeAllCatalog
      />,
    );

    expect(
      screen.getByRole("heading", { name: "Add AI Service" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("heading", { name: "Configure routing for GitHub" }),
    ).not.toBeInTheDocument();
  });

  it("creates a key from a catalog entry, omitting params that match catalog defaults", async () => {
    createKeyMutate.mockImplementation((_params, opts) => {
      opts?.onSuccess?.({ id: "new-key-2" });
    });
    const user = userEvent.setup();
    render(<AddKeyDialog open onOpenChange={vi.fn()} />);

    // Catalog step → pick the OpenAI template (prefills label + endpoint).
    await user.click(screen.getByRole("button", { name: /OpenAI/i }));
    await user.click(
      screen.getByRole("button", { name: /Next: Enter Credentials/i }),
    );

    // Only the credential needs entering — label/endpoint are prefilled.
    await typeInto(user, "add-key-credential", "sk-openai-key");
    await user.click(screen.getByRole("button", { name: "Connect Service" }));

    await waitFor(() => expect(createKeyMutate).toHaveBeenCalledTimes(1));
    // auth_method / auth_key_name are omitted because they equal the
    // catalog defaults; endpoint_url rides along from the prefilled base_url.
    expect(createKeyMutate).toHaveBeenCalledWith(
      {
        credential: "sk-openai-key",
        label: "OpenAI",
        service_slug: "openai",
        endpoint_url: "https://api.openai.com/v1",
      },
      expect.anything(),
    );
    // Wave-aha-1 A4: dialog transitions to the verify step with the
    // catalog-entry's display name ("OpenAI"). No premature toast,
    // no premature navigate.
    await waitFor(() =>
      expect(
        screen.getByRole("heading", {
          name: (n) => /OpenAI/i.test(n),
        }),
      ).toBeInTheDocument(),
    );
    expect(toastFns.success).not.toHaveBeenCalledWith("Key created");
    expect(mockNavigate).not.toHaveBeenCalled();
  });
});

describe("AddKeyDialog — platform one-click path (credential_mode=both)", () => {
  const PLATFORM_OAUTH_ENTRY = {
    ...OAUTH_ENTRY,
    credential_mode: "both",
    has_platform_oauth_credentials: true,
  } as unknown as CatalogEntry;

  const BYO_ONLY_BOTH_ENTRY = {
    ...OAUTH_ENTRY,
    credential_mode: "both",
    has_platform_oauth_credentials: false,
  } as unknown as CatalogEntry;

  // Open the routing screen for a platform entry (choice cards live here now).
  async function gotoRouting(user: ReturnType<typeof userEvent.setup>) {
    await user.click(screen.getByRole("button", { name: /GitHub/i }));
    // The OAuth-client choice is merged into the routing screen (Direct
    // selected by default), with NyxID managed pre-selected.
    expect(
      screen.getByRole("radio", { name: /NyxID managed/i }),
    ).toHaveAttribute("aria-checked", "true");
  }

  it("shows the OAuth-client choice on the routing screen; managed reaches one-click connect", async () => {
    catalog.entries = [PLATFORM_OAUTH_ENTRY];
    const user = userEvent.setup();
    render(<AddKeyDialog open onOpenChange={vi.fn()} />);

    await gotoRouting(user);
    // Default (managed) → connect step, no client-ID/secret form.
    await user.click(screen.getByRole("button", { name: "Next: Connect" }));
    expect(
      screen.getByRole("button", { name: /Connect with GitHub/i }),
    ).toBeInTheDocument();
    expect(
      screen.queryByText(/Setup GitHub credentials/i),
    ).not.toBeInTheDocument();
  });

  it("advances the managed OAuth path to verify inside dev mock mode", async () => {
    window.history.replaceState({}, "", "/assistant?mock");
    catalog.entries = [PLATFORM_OAUTH_ENTRY];
    catalog.allEntries = [{ ...PLATFORM_OAUTH_ENTRY, slug: "api-github" }];
    createKeyMutateAsync.mockResolvedValue({
      id: "mock-user-service",
      slug: "github-action-demo",
    });
    const onSuccess = vi.fn();
    const user = userEvent.setup();
    render(
      <AddKeyDialog
        open
        onOpenChange={vi.fn()}
        prefillSlug="api-github"
        prefillIncludeAllCatalog
        onSuccess={onSuccess}
      />,
    );

    await user.click(
      await screen.findByRole("button", { name: "Next: Connect" }),
    );
    await user.click(
      screen.getByRole("button", { name: "Connect with GitHub" }),
    );

    expect(
      await screen.findByRole("heading", {
        name: /Service.*GitHub.*connected/i,
      }),
    ).toBeInTheDocument();
    expect(initiateOAuthMutateAsync).not.toHaveBeenCalled();
    expect(mockHardRedirect).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Maybe later" }));
    expect(onSuccess).toHaveBeenCalledWith({
      userServiceId: "mock-user-service",
    });
  });

  it("hides the OAuth-client choice and goes to the BYO form when platform credentials are absent", async () => {
    catalog.entries = [BYO_ONLY_BOTH_ENTRY];
    const user = userEvent.setup();
    render(<AddKeyDialog open onOpenChange={vi.fn()} />);

    await user.click(screen.getByRole("button", { name: /GitHub/i }));
    // No managed option → no choice cards on the routing screen.
    expect(
      screen.queryByRole("radio", { name: /NyxID managed/i }),
    ).not.toBeInTheDocument();
    await user.click(
      screen.getByRole("button", { name: /Next: Enter Credentials/i }),
    );
    expect(screen.getByText(/Setup GitHub credentials/i)).toBeInTheDocument();
  });

  it("self-managed card opens the BYO form and Back returns to routing", async () => {
    catalog.entries = [PLATFORM_OAUTH_ENTRY];
    const user = userEvent.setup();
    render(<AddKeyDialog open onOpenChange={vi.fn()} />);

    await gotoRouting(user);
    await user.click(
      screen.getByRole("radio", { name: /Your own OAuth app/i }),
    );
    await user.click(
      screen.getByRole("button", { name: /Next: Enter Credentials/i }),
    );
    expect(screen.getByText(/Setup GitHub credentials/i)).toBeInTheDocument();

    // Back returns to the merged routing screen (choice cards visible again).
    await user.click(screen.getByRole("button", { name: /^Back$/i }));
    expect(
      screen.getByRole("radio", { name: /NyxID managed/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("radio", { name: /Your own OAuth app/i }),
    ).toBeInTheDocument();
  });

  it("gates non-allowlisted scope pills on the managed path", async () => {
    catalog.entries = [
      {
        ...PLATFORM_OAUTH_ENTRY,
        default_scopes: ["read:user"],
        scope_catalog: [
          {
            scope: "read:user",
            label: "Read profile",
            description: "Read profile.",
          },
          {
            scope: "delete_repo",
            label: "Delete repos",
            description: "Delete repositories.",
            sensitive: true,
          },
        ],
        platform_scope_allowlist: ["read:user", "user:email"],
      } as unknown as CatalogEntry,
    ];
    const user = userEvent.setup();
    render(<AddKeyDialog open onOpenChange={vi.fn()} />);

    await gotoRouting(user);
    await user.click(screen.getByRole("button", { name: "Next: Connect" }));

    // Allowlisted pill selectable; non-allowlisted disabled with the marker —
    // the user can never select a scope the shared app would reject.
    expect(screen.getByRole("button", { name: /Read profile/i })).toBeEnabled();
    const gated = screen.getByRole("button", { name: /Delete repos/i });
    expect(gated).toBeDisabled();
    expect(gated).toHaveTextContent(/own app/i);
  });

  it("requires the client secret when choosing self-managed on a platform-capable entry", async () => {
    catalog.entries = [PLATFORM_OAUTH_ENTRY];
    const user = userEvent.setup();
    render(<AddKeyDialog open onOpenChange={vi.fn()} />);

    await gotoRouting(user);
    await user.click(
      screen.getByRole("radio", { name: /Your own OAuth app/i }),
    );
    await user.click(
      screen.getByRole("button", { name: /Next: Enter Credentials/i }),
    );

    // ID alone must not enable Continue — an id-only submit would silently
    // ride the platform app instead of the user's own.
    await user.type(screen.getByLabelText(/Client ID/i), "Ov23li-my-own-app");
    expect(
      screen.getByRole("button", { name: /Continue to Authentication/i }),
    ).toBeDisabled();
    await user.type(screen.getByLabelText(/Client Secret/i), "s3cret");
    expect(
      screen.getByRole("button", { name: /Continue to Authentication/i }),
    ).toBeEnabled();
  });

  it("Back from the managed connect step returns to the routing screen", async () => {
    catalog.entries = [PLATFORM_OAUTH_ENTRY];
    const user = userEvent.setup();
    render(<AddKeyDialog open onOpenChange={vi.fn()} />);

    await gotoRouting(user);
    await user.click(screen.getByRole("button", { name: "Next: Connect" }));
    await user.click(screen.getByRole("button", { name: /^Back$/i }));

    // The merged routing screen, not the BYO form the managed path never showed.
    expect(
      screen.getByRole("radio", { name: /NyxID managed/i }),
    ).toBeInTheDocument();
    expect(
      screen.queryByText(/Setup GitHub credentials/i),
    ).not.toBeInTheDocument();
  });
});

describe("AddKeyDialog — reconnect path", () => {
  it("starts OAuth reconnect with the existing key id and detail redirect without creating or deleting a key", async () => {
    catalog.entries = [OAUTH_ENTRY];
    const user = userEvent.setup();
    render(
      <AddKeyDialog
        open
        onOpenChange={vi.fn()}
        reconnectKey={makeReconnectKey()}
      />,
    );

    await user.click(
      screen.getByRole("button", { name: /Connect with GitHub/i }),
    );

    await waitFor(() => {
      expect(initiateOAuthMutateAsync).toHaveBeenCalledTimes(1);
    });
    expect(initiateOAuthMutateAsync).toHaveBeenCalledWith({
      providerId: "provider-oauth",
      redirectPath: "/keys/existing-service-1",
      // Scope picker (NyxID#917): unedited submit sends the provider defaults
      // as the complete override set. OAUTH_ENTRY has no defaults → empty.
      scopeOverride: [],
      keyId: "existing-service-1",
    });
    expect(createKeyMutate).not.toHaveBeenCalled();
    expect(createKeyMutateAsync).not.toHaveBeenCalled();
    expect(mockApiDelete).not.toHaveBeenCalled();
    // The dialog must NOT navigate the whole tab away: the assistant's
    // in-chat connect card cannot survive its own tab going to GitHub.
    // Authorization is handed off through an explicit link instead, and the
    // placeholder key is polled in place.
    expect(mockHardRedirect).not.toHaveBeenCalled();
    const authorizeLink = await screen.findByRole("link", {
      name: /Open GitHub/i,
    });
    expect(authorizeLink).toHaveAttribute(
      "href",
      "https://provider.example/oauth",
    );
    expect(authorizeLink).toHaveAttribute("target", "_blank");
    expect(screen.getByRole("status")).toHaveTextContent(/Waiting for GitHub/i);
  });

  it("reports success in place once the polled placeholder key goes active", async () => {
    catalog.entries = [OAUTH_ENTRY];
    pendingKeyStatus.value = "active";
    pendingLastAuthorizedAt.value = "2026-01-01T00:01:00Z";
    const user = userEvent.setup();
    render(
      <AddKeyDialog
        open
        onOpenChange={vi.fn()}
        reconnectKey={makeReconnectKey()}
      />,
    );

    await user.click(
      screen.getByRole("button", { name: /Connect with GitHub/i }),
    );

    await waitFor(() => {
      expect(screen.getByRole("status")).toHaveTextContent(/Connected/i);
    });
    expect(mockHardRedirect).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Continue" }));
    expect(
      await screen.findByRole("heading", { name: /connected/i }),
    ).toBeInTheDocument();
  });

  it("does not treat a preserved active reconnect credential as fresh authorization", async () => {
    catalog.entries = [OAUTH_ENTRY];
    pendingKeyStatus.value = "active";
    pendingLastAuthorizedAt.value = "2026-01-01T00:00:00Z";
    const user = userEvent.setup();
    render(
      <AddKeyDialog
        open
        onOpenChange={vi.fn()}
        reconnectKey={makeReconnectKey({ status: "active" })}
      />,
    );

    await user.click(
      screen.getByRole("button", { name: /Connect with GitHub/i }),
    );

    await waitFor(() => {
      expect(screen.getByRole("status")).toHaveTextContent(
        /Waiting for GitHub/i,
      );
    });
    expect(
      screen.queryByRole("button", { name: "Continue" }),
    ).not.toBeInTheDocument();
  });

  it("offers a retry when the provider denies authorization", async () => {
    catalog.entries = [OAUTH_ENTRY];
    pendingKeyStatus.value = "failed";
    const user = userEvent.setup();
    render(
      <AddKeyDialog
        open
        onOpenChange={vi.fn()}
        reconnectKey={makeReconnectKey()}
      />,
    );

    await user.click(
      screen.getByRole("button", { name: /Connect with GitHub/i }),
    );

    await waitFor(() => {
      expect(screen.getByRole("status")).toHaveTextContent(
        /denied or expired/i,
      );
    });
    // Back to the scope picker so a retry mints a fresh authorization.
    await user.click(screen.getByRole("button", { name: /Try again/i }));
    expect(
      screen.getByRole("button", { name: /Connect with GitHub/i }),
    ).toBeInTheDocument();
  });

  it("passes targetOrgId for admin org-owned OAuth reconnects", async () => {
    catalog.entries = [OAUTH_ENTRY];
    const user = userEvent.setup();
    render(
      <AddKeyDialog
        open
        onOpenChange={vi.fn()}
        reconnectKey={makeReconnectKey({
          credential_source: {
            type: "org",
            org_id: "org-user-1",
            org_name: "Acme",
            avatar_url: null,
            role: "admin",
            allowed: true,
          },
        })}
      />,
    );

    await user.click(
      screen.getByRole("button", { name: /Connect with GitHub/i }),
    );

    expect(initiateOAuthMutateAsync).toHaveBeenCalledWith(
      expect.objectContaining({
        keyId: "existing-service-1",
        redirectPath: "/keys/existing-service-1",
        targetOrgId: "org-user-1",
      }),
    );
  });

  it("does not delete an existing OAuth key when initiate fails or Back closes the reconnect dialog", async () => {
    catalog.entries = [OAUTH_ENTRY];
    initiateOAuthMutateAsync.mockRejectedValue(
      new ApiError(400, {
        error: "bad_request",
        error_code: 1000,
        message: "provider unavailable",
      }),
    );
    const onOpenChange = vi.fn();
    const user = userEvent.setup();
    render(
      <AddKeyDialog
        open
        onOpenChange={onOpenChange}
        reconnectKey={makeReconnectKey({ status: "pending_auth" })}
      />,
    );

    await user.click(
      screen.getByRole("button", { name: /Connect with GitHub/i }),
    );
    await waitFor(() =>
      expect(screen.getByText("provider unavailable")).toBeInTheDocument(),
    );
    await user.click(screen.getByRole("button", { name: /^Back$/i }));

    expect(mockApiDelete).not.toHaveBeenCalled();
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it("starts device-code reconnect with the existing key id and never creates a key", async () => {
    catalog.entries = [DEVICE_CODE_ENTRY];
    const user = userEvent.setup();
    render(
      <AddKeyDialog
        open
        onOpenChange={vi.fn()}
        reconnectKey={makeReconnectKey({
          catalog_service_slug: "codex",
          catalog_service_name: "Codex",
        })}
      />,
    );

    await user.click(screen.getByRole("button", { name: "Continue" }));

    await waitFor(() => {
      expect(initiateDeviceCodeMutateAsync).toHaveBeenCalledTimes(1);
    });
    expect(initiateDeviceCodeMutateAsync).toHaveBeenCalledWith({
      providerId: "provider-device",
      // DEVICE_CODE_ENTRY is openai-format → no scope override is sent.
      scopeOverride: undefined,
      keyId: "existing-service-1",
    });
    expect(createKeyMutate).not.toHaveBeenCalled();
    expect(createKeyMutateAsync).not.toHaveBeenCalled();
    expect(mockApiDelete).not.toHaveBeenCalled();
  });

  it("does not announce a device-code attempt when initiation fails", async () => {
    catalog.entries = [DEVICE_CODE_ENTRY];
    initiateDeviceCodeMutateAsync.mockRejectedValue(
      new Error("device unavailable"),
    );
    const onAuthorizationPending = vi.fn();
    const user = userEvent.setup();
    render(
      <AddKeyDialog
        open
        onOpenChange={vi.fn()}
        reconnectKey={makeReconnectKey({
          catalog_service_slug: "codex",
          catalog_service_name: "Codex",
        })}
        onAuthorizationPending={onAuthorizationPending}
      />,
    );

    await user.click(screen.getByRole("button", { name: "Continue" }));
    expect(
      await screen.findByText("Failed to request device code"),
    ).toBeInTheDocument();
    expect(onAuthorizationPending).not.toHaveBeenCalled();
  });

  it("does not delete an existing device-code key on Back or unmount during reconnect", async () => {
    catalog.entries = [DEVICE_CODE_ENTRY];
    const onOpenChange = vi.fn();
    const user = userEvent.setup();
    const first = render(
      <AddKeyDialog
        open
        onOpenChange={onOpenChange}
        reconnectKey={makeReconnectKey({
          catalog_service_slug: "codex",
          catalog_service_name: "Codex",
          status: "pending_auth",
        })}
      />,
    );

    await user.click(screen.getByRole("button", { name: "Continue" }));
    await waitFor(() => {
      expect(screen.getByText("ABCD-EFGH")).toBeInTheDocument();
    });
    await user.click(screen.getByRole("button", { name: /^Back$/i }));

    expect(mockApiDelete).not.toHaveBeenCalled();
    expect(onOpenChange).toHaveBeenCalledWith(false);
    first.unmount();

    vi.clearAllMocks();
    catalog.entries = [DEVICE_CODE_ENTRY];
    const second = render(
      <AddKeyDialog
        open
        onOpenChange={vi.fn()}
        reconnectKey={makeReconnectKey({
          catalog_service_slug: "codex",
          catalog_service_name: "Codex",
          status: "pending_auth",
        })}
      />,
    );
    await user.click(screen.getByRole("button", { name: "Continue" }));
    await waitFor(() => {
      expect(screen.getByText("ABCD-EFGH")).toBeInTheDocument();
    });
    second.unmount();

    expect(mockApiDelete).not.toHaveBeenCalled();
  });
});

describe("AddKeyDialog — managed OAuth popup", () => {
  const nonce = "8e1fcf2a-e679-4da2-9f54-2d90cd5f0085";
  const authorizationUrl = `https://github.com/login/oauth/authorize?state=1cc_${nonce}`;

  function popupWindow() {
    return {
      closed: false,
      close: vi.fn(),
      postMessage: vi.fn(),
    } as unknown as Window;
  }

  it("opens synchronously and sends only the chat flow contract", async () => {
    catalog.entries = [OAUTH_ENTRY];
    const popup = popupWindow();
    const open = vi.spyOn(window, "open").mockReturnValue(popup);
    initiateOAuthMutateAsync.mockResolvedValue({
      authorization_url: authorizationUrl,
      attempt_nonce: nonce,
    });
    const user = userEvent.setup();
    const { unmount } = render(
      <AddKeyDialog
        open
        onOpenChange={vi.fn()}
        reconnectKey={makeReconnectKey()}
        launch="popup"
        flow="cc"
      />,
    );

    await user.click(
      screen.getByRole("button", { name: /Connect with GitHub/i }),
    );
    await waitFor(() =>
      expect(initiateOAuthMutateAsync).toHaveBeenCalledTimes(1),
    );

    expect(open).toHaveBeenCalledTimes(1);
    expect(open.mock.invocationCallOrder[0]).toBeLessThan(
      initiateOAuthMutateAsync.mock.invocationCallOrder[0] ?? Infinity,
    );
    expect(initiateOAuthMutateAsync).toHaveBeenCalledWith({
      providerId: "provider-oauth",
      scopeOverride: [],
      keyId: "existing-service-1",
      flow: "cc",
    });
    expect(initiateOAuthMutateAsync.mock.calls[0]?.[0]).not.toHaveProperty(
      "redirectPath",
    );
    expect(useOAuthPopupStore.getState().attempt).toMatchObject({
      nonce,
      keyId: "existing-service-1",
    });
    unmount();
    expect(popup.close).toHaveBeenCalled();
    expect(useOAuthPopupStore.getState().attempt).toBeNull();
  });

  it("falls back to the existing anchor when the popup is blocked", async () => {
    catalog.entries = [OAUTH_ENTRY];
    vi.spyOn(window, "open").mockReturnValue(null);
    initiateOAuthMutateAsync.mockResolvedValue({
      authorization_url: authorizationUrl,
      attempt_nonce: nonce,
    });
    const user = userEvent.setup();
    render(
      <AddKeyDialog
        open
        onOpenChange={vi.fn()}
        reconnectKey={makeReconnectKey()}
        launch="popup"
        flow="cc"
      />,
    );

    await user.click(
      screen.getByRole("button", { name: /Connect with GitHub/i }),
    );

    expect(
      await screen.findByRole("link", { name: /Open GitHub/i }),
    ).toHaveAttribute("href", authorizationUrl);
    expect(useOAuthPopupStore.getState().attempt).toBeNull();
  });

  it("closes the placeholder and releases ownership when initiation fails", async () => {
    catalog.entries = [OAUTH_ENTRY];
    const popup = popupWindow();
    vi.spyOn(window, "open").mockReturnValue(popup);
    initiateOAuthMutateAsync.mockRejectedValue(new Error("network down"));
    const user = userEvent.setup();
    render(
      <AddKeyDialog
        open
        onOpenChange={vi.fn()}
        reconnectKey={makeReconnectKey()}
        launch="popup"
        flow="cc"
      />,
    );

    await user.click(
      screen.getByRole("button", { name: /Connect with GitHub/i }),
    );

    expect(
      await screen.findByText("Failed to start OAuth flow"),
    ).toBeInTheDocument();
    expect(popup.close).toHaveBeenCalled();
    expect(useOAuthPopupStore.getState().attempt).toBeNull();
  });

  it("falls back to the manual link when popup handoff fails", async () => {
    catalog.entries = [OAUTH_ENTRY];
    const popup = popupWindow();
    vi.mocked(popup.postMessage).mockImplementation(() => {
      throw new Error("popup detached");
    });
    vi.spyOn(window, "open").mockReturnValue(popup);
    initiateOAuthMutateAsync.mockResolvedValue({
      authorization_url: authorizationUrl,
      attempt_nonce: nonce,
    });
    const onAuthorizationPending = vi.fn();
    const onAuthorizationAborted = vi.fn();
    const user = userEvent.setup();
    render(
      <AddKeyDialog
        open
        onOpenChange={vi.fn()}
        reconnectKey={makeReconnectKey()}
        launch="popup"
        flow="cc"
        onAuthorizationPending={onAuthorizationPending}
        onAuthorizationAborted={onAuthorizationAborted}
      />,
    );

    await user.click(
      screen.getByRole("button", { name: /Connect with GitHub/i }),
    );
    await waitFor(() => expect(initiateOAuthMutateAsync).toHaveBeenCalled());
    const launchId = useOAuthPopupStore.getState().attempt?.launchId;
    window.dispatchEvent(
      new MessageEvent("message", {
        origin: window.location.origin,
        source: popup,
        data: { type: "oauth_launch_ready", launchId },
      }),
    );

    expect(
      await screen.findByRole("link", { name: /Open GitHub/i }),
    ).toHaveAttribute("href", authorizationUrl);
    expect(onAuthorizationPending).toHaveBeenCalledTimes(1);
    expect(onAuthorizationAborted).not.toHaveBeenCalled();
    expect(mockApiDelete).not.toHaveBeenCalled();
    expect(useOAuthPopupStore.getState().attempt).toBeNull();
    expect(
      screen.queryByText("Failed to start OAuth flow"),
    ).not.toBeInTheDocument();
  });

  it("falls back to a safe manual link when an older backend omits the popup nonce", async () => {
    catalog.entries = [OAUTH_ENTRY];
    const popup = popupWindow();
    vi.spyOn(window, "open").mockReturnValue(popup);
    initiateOAuthMutateAsync.mockResolvedValue({
      authorization_url: "https://github.com/login/oauth/authorize",
    });
    const onAuthorizationPending = vi.fn();
    const user = userEvent.setup();
    render(
      <AddKeyDialog
        open
        onOpenChange={vi.fn()}
        reconnectKey={makeReconnectKey()}
        launch="popup"
        flow="cc"
        onAuthorizationPending={onAuthorizationPending}
      />,
    );

    await user.click(
      screen.getByRole("button", { name: /Connect with GitHub/i }),
    );

    expect(
      await screen.findByRole("link", { name: /Open GitHub/i }),
    ).toHaveAttribute("href", "https://github.com/login/oauth/authorize");
    expect(onAuthorizationPending).toHaveBeenCalledTimes(1);
    expect(mockApiDelete).not.toHaveBeenCalled();
    expect(useOAuthPopupStore.getState().attempt).toBeNull();
  });

  it("does not clean up a fresh placeholder when initiation rejects post-unmount", async () => {
    catalog.entries = [OAUTH_ENTRY];
    const popup = popupWindow();
    vi.spyOn(window, "open").mockReturnValue(popup);
    createKeyMutateAsync.mockResolvedValue(
      makeReconnectKey({
        id: "fresh-service-1",
        status: "pending_auth",
        last_authorized_at: null,
      }),
    );
    let rejectInitiation: ((reason: Error) => void) | undefined;
    initiateOAuthMutateAsync.mockImplementation(
      () =>
        new Promise((_, reject) => {
          rejectInitiation = reject;
        }),
    );
    const user = userEvent.setup();
    const view = render(
      <AddKeyDialog
        open
        onOpenChange={vi.fn()}
        prefillSlug="github"
        launch="popup"
        flow="cc"
      />,
    );
    await user.click(
      await screen.findByRole("button", { name: "Next: Connect" }),
    );
    await user.click(
      screen.getByRole("button", { name: /Connect with GitHub/i }),
    );
    await waitFor(() => expect(initiateOAuthMutateAsync).toHaveBeenCalled());

    view.unmount();
    rejectInitiation?.(new Error("request completed after unmount"));
    await waitFor(() => expect(popup.close).toHaveBeenCalled());

    expect(mockApiDelete).not.toHaveBeenCalled();
  });

  it("announces a fresh attempt generation when the popup retries", async () => {
    const nextNonce = "23f1c824-960c-4b5c-8e12-71f1dbce5b20";
    catalog.entries = [OAUTH_ENTRY];
    vi.spyOn(window, "open").mockReturnValue(null);
    initiateOAuthMutateAsync
      .mockResolvedValueOnce({
        authorization_url: authorizationUrl,
        attempt_nonce: nonce,
      })
      .mockResolvedValueOnce({
        authorization_url: `https://github.com/login/oauth/authorize?state=1cc_${nextNonce}`,
        attempt_nonce: nextNonce,
      });
    const onAuthorizationPending = vi.fn();
    const user = userEvent.setup();
    render(
      <AddKeyDialog
        open
        onOpenChange={vi.fn()}
        reconnectKey={makeReconnectKey()}
        launch="popup"
        flow="cc"
        onAuthorizationPending={onAuthorizationPending}
      />,
    );

    await user.click(
      screen.getByRole("button", { name: /Connect with GitHub/i }),
    );
    await waitFor(() =>
      expect(onAuthorizationPending).toHaveBeenCalledTimes(1),
    );
    await act(async () => {
      await popupReceiverOptions.current?.onRetry?.();
    });

    expect(onAuthorizationPending).toHaveBeenCalledTimes(2);
    expect(onAuthorizationPending.mock.calls[1]?.[0].keyId).toBe(
      onAuthorizationPending.mock.calls[0]?.[0].keyId,
    );
    expect(onAuthorizationPending.mock.calls[1]?.[0].attemptId).not.toBe(
      onAuthorizationPending.mock.calls[0]?.[0].attemptId,
    );
  });

  it("aborts on dialog close before terminal status but not after authorization", async () => {
    catalog.entries = [OAUTH_ENTRY];
    vi.spyOn(window, "open").mockReturnValue(null);
    initiateOAuthMutateAsync.mockResolvedValue({
      authorization_url: authorizationUrl,
      attempt_nonce: nonce,
    });
    const pendingBeforeClose = vi.fn();
    const abortedBeforeClose = vi.fn();
    const user = userEvent.setup();
    const pending = render(
      <AddKeyDialog
        open
        onOpenChange={vi.fn()}
        reconnectKey={makeReconnectKey()}
        launch="popup"
        flow="cc"
        onAuthorizationPending={pendingBeforeClose}
        onAuthorizationAborted={abortedBeforeClose}
      />,
    );
    await user.click(
      screen.getByRole("button", { name: /Connect with GitHub/i }),
    );
    await waitFor(() => expect(pendingBeforeClose).toHaveBeenCalledTimes(1));
    pending.unmount();
    expect(abortedBeforeClose).toHaveBeenCalledWith(
      pendingBeforeClose.mock.calls[0]?.[0].attemptId,
    );

    pendingKeyStatus.value = null;
    const pendingAfterSuccess = vi.fn();
    const abortedAfterSuccess = vi.fn();
    const completed = render(
      <AddKeyDialog
        open
        onOpenChange={vi.fn()}
        reconnectKey={makeReconnectKey()}
        launch="popup"
        flow="cc"
        onAuthorizationPending={pendingAfterSuccess}
        onAuthorizationAborted={abortedAfterSuccess}
      />,
    );
    await user.click(
      screen.getByRole("button", { name: /Connect with GitHub/i }),
    );
    await waitFor(() => expect(pendingAfterSuccess).toHaveBeenCalledTimes(1));
    pendingKeyStatus.value = "active";
    pendingLastAuthorizedAt.value = "2026-08-06T12:00:00Z";
    completed.rerender(
      <AddKeyDialog
        open
        onOpenChange={vi.fn()}
        reconnectKey={makeReconnectKey()}
        launch="popup"
        flow="cc"
        onAuthorizationPending={pendingAfterSuccess}
        onAuthorizationAborted={abortedAfterSuccess}
      />,
    );
    expect(
      await screen.findByText(/Connected\. Credential sealed/i),
    ).toBeInTheDocument();
    completed.unmount();
    expect(abortedAfterSuccess).not.toHaveBeenCalled();
  });

  it("treats popup closure as a soft hint and mutates nothing automatically", async () => {
    catalog.entries = [OAUTH_ENTRY];
    const popup = {
      closed: false,
      close: vi.fn(),
      postMessage: vi.fn(),
    };
    vi.spyOn(window, "open").mockReturnValue(popup as unknown as Window);
    initiateOAuthMutateAsync.mockResolvedValue({
      authorization_url: authorizationUrl,
      attempt_nonce: nonce,
    });
    const user = userEvent.setup();
    render(
      <AddKeyDialog
        open
        onOpenChange={vi.fn()}
        reconnectKey={makeReconnectKey({ status: "active" })}
        launch="popup"
        flow="cc"
      />,
    );

    await user.click(
      screen.getByRole("button", { name: /Connect with GitHub/i }),
    );
    expect(await screen.findByText(/Waiting for GitHub/i)).toBeInTheDocument();
    popup.closed = true;

    expect(
      await screen.findByText(
        /provider window may have closed/i,
        {},
        { timeout: 2_500 },
      ),
    ).toBeInTheDocument();
    expect(mockApiDelete).not.toHaveBeenCalled();
    expect(initiateOAuthMutateAsync).toHaveBeenCalledTimes(1);
    expect(useOAuthPopupStore.getState().attempt).not.toBeNull();

    await user.click(screen.getByRole("button", { name: "Start again" }));
    expect(useOAuthPopupStore.getState().attempt).toBeNull();
    expect(mockApiDelete).not.toHaveBeenCalled();
    expect(initiateOAuthMutateAsync).toHaveBeenCalledTimes(1);
    expect(
      screen.getByRole("button", { name: /Connect with GitHub/i }),
    ).toBeInTheDocument();
  });

  it("keeps the legacy caller free of popup and flow parameters", async () => {
    catalog.entries = [OAUTH_ENTRY];
    const open = vi.spyOn(window, "open");
    const user = userEvent.setup();
    render(
      <AddKeyDialog
        open
        onOpenChange={vi.fn()}
        reconnectKey={makeReconnectKey()}
      />,
    );

    await user.click(
      screen.getByRole("button", { name: /Connect with GitHub/i }),
    );
    await waitFor(() =>
      expect(initiateOAuthMutateAsync).toHaveBeenCalledTimes(1),
    );

    expect(open).not.toHaveBeenCalled();
    expect(initiateOAuthMutateAsync).toHaveBeenCalledWith({
      providerId: "provider-oauth",
      redirectPath: "/keys/existing-service-1",
      scopeOverride: [],
      keyId: "existing-service-1",
    });
    expect(initiateOAuthMutateAsync.mock.calls[0]?.[0]).not.toHaveProperty(
      "flow",
    );
  });
});

// Build a minimal catalog entry suitable for CatalogGrid rendering. The
// dialog only reads `slug`, `name`, `description`, `base_url`,
// `service_type`, `requires_gateway_url`, and `provider_type` from each entry
// when rendering the tile; everything else can be cast through `unknown` so
// we don't duplicate the full `CatalogEntry` shape just for icon assertions.
function minCatalogEntry(slug: string, name = slug): CatalogEntry {
  return {
    slug,
    name,
    description: null,
    base_url: "https://example.invalid",
    auth_method: "bearer",
    auth_key_name: "Authorization",
    provider_config_id: null,
    provider_type: null,
    requires_gateway_url: false,
    credential_mode: null,
    api_key_instructions: null,
    api_key_url: null,
    icon_url: null,
    documentation_url: null,
    service_type: "http",
    ssh_host: null,
    ssh_port: null,
    ssh_ca_public_key: null,
    ssh_allowed_principals: null,
    ssh_certificate_ttl_minutes: null,
    authorization_url: null,
    token_url: null,
    device_code_url: null,
    default_scopes: null,
    supports_pkce: null,
    device_code_format: null,
    oauth_client_id: null,
    client_id_param_name: null,
    requires_credential: true,
  } as unknown as CatalogEntry;
}

describe("AddKeyDialog → ConnectVerifyStep integration (end-to-end wiring)", () => {
  it("reports the created UserService id only when the success step is finished", async () => {
    createKeyMutate.mockImplementation((_params, opts) => {
      opts?.onSuccess?.({
        id: "user-service-from-action",
        slug: "custom-action-service",
      });
    });
    const onOpenChange = vi.fn();
    const onSuccess = vi.fn();
    const user = userEvent.setup();
    render(
      <AddKeyDialog open onOpenChange={onOpenChange} onSuccess={onSuccess} />,
    );

    await user.click(screen.getByRole("button", { name: /Custom Endpoint/i }));
    await user.click(
      screen.getByRole("button", { name: /Next: Enter Credentials/i }),
    );
    await typeInto(user, "add-key-label", "Action Service");
    await typeInto(user, "add-key-credential", "obviously-fake-credential");
    await typeInto(user, "add-key-endpoint", "https://action.example.test/v1");
    await user.click(screen.getByRole("button", { name: "Connect Service" }));

    expect(onSuccess).not.toHaveBeenCalled();
    await user.click(
      await screen.findByRole("button", { name: "Maybe later" }),
    );

    expect(onSuccess).toHaveBeenCalledOnce();
    expect(onSuccess).toHaveBeenCalledWith({
      userServiceId: "user-service-from-action",
    });
    expect(onOpenChange).toHaveBeenCalledWith(false);
  }, 10_000);

  // GLM #8 + Kimi — the intentionally-swallowed createApiKeyMutate
  // in the other tests hides the real dialog → verify-step wiring.
  // This test lets the mint fire onSuccess so we can assert the
  // subsequent probe actually reaches window.fetch with the right
  // slug + bearer. A regression that broke createdKey.slug threading
  // (undefined slug → empty proxy URL) would fail here.
  it("mint success wires the probe against the correct proxy slug", async () => {
    createKeyMutate.mockImplementation((_params, opts) => {
      // Backend returns id + slug — the slug is what threads into
      // ConnectVerifyStep and drives the probe URL. If the field
      // name ever drifts (e.g. `service_slug` vs `slug`), the probe
      // URL below breaks.
      // Prefixed service_slug — matches what the backend actually seeds
      // (`llm-openai` from provider_service.rs) so the recipe registry
      // fires and the probe URL below resolves correctly.
      opts?.onSuccess?.({ id: "new-key-1", slug: "llm-openai" });
    });
    createApiKeyMutate.mockImplementation((_params, opts) => {
      opts?.onSuccess?.({
        id: "ak-1",
        full_key: "nyxid_ag_integration_secret",
        key_prefix: "nyxid_ag_",
        scopes: ["proxy"],
        allow_all_services: false,
        allowed_service_ids: ["new-key-1"],
      });
    });
    // Downstream response with the X-NyxID-Agent-Id header set —
    // proves the probe actually hits the classifier's happy path.
    const fetchSpy = vi.spyOn(window, "fetch").mockResolvedValue(
      new Response("{}", {
        status: 200,
        headers: { "x-nyxid-agent-id": "ak-1" },
      }),
    );

    const user = userEvent.setup();
    render(<AddKeyDialog open onOpenChange={vi.fn()} />);

    // Walk the catalog → routing → form path.
    await user.click(screen.getByRole("button", { name: /OpenAI/i }));
    await user.click(
      screen.getByRole("button", { name: /Next: Enter Credentials/i }),
    );
    await typeInto(user, "add-key-credential", "sk-integration");
    await user.click(screen.getByRole("button", { name: "Connect Service" }));

    // Wait for the verify step to mount + user clicks Create Agent Key
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: /Create Agent Key/i }),
      ).toBeInTheDocument(),
    );
    await user.click(screen.getByRole("button", { name: /Create Agent Key/i }));

    // Panel appears with the minted secret; Test button available.
    await waitFor(() =>
      expect(
        screen.getByText("nyxid_ag_integration_secret"),
      ).toBeInTheDocument(),
    );
    await user.click(screen.getByRole("button", { name: /Test Agent Key/i }));

    // The probe reached fetch — pin the exact URL derived from the
    // OpenAI slug's registry recipe (/models — relative to the
    // seeded base_url `.../v1`). If the wiring drops createdKey.slug,
    // this test fails immediately.
    await waitFor(() => expect(fetchSpy).toHaveBeenCalledTimes(1));
    const call = fetchSpy.mock.calls[0];
    if (!call) throw new Error("probe fetch was not called");
    expect(String(call[0])).toBe("/api/v1/proxy/s/llm-openai/models");
    expect(call[1]?.headers).toMatchObject({
      Authorization: "Bearer nyxid_ag_integration_secret",
    });
  });
});

describe("AddKeyDialog — catalog service icons", () => {
  it("renders a dedicated brand icon for every seeded catalog slug (no fallback)", () => {
    catalog.entries = SPEC_CATALOG_SLUGS.map((slug) =>
      minCatalogEntry(slug),
    ) as unknown as typeof catalog.entries;
    render(<AddKeyDialog open onOpenChange={vi.fn()} />);

    // Each seeded slug must render its own dedicated `<svg data-slug=…>`.
    // Assert `tagName === "svg"` so the test fails if `data-slug` ever moves
    // off the SVG onto a wrapper span (Codex review nit).
    for (const slug of SPEC_CATALOG_SLUGS) {
      const matched = document.querySelector(`[data-slug="${slug}"]`);
      expect(matched, `expected [data-slug="${slug}"] in DOM`).not.toBeNull();
      expect(
        matched?.tagName.toLowerCase(),
        `expected data-slug="${slug}" on an <svg>, not a wrapper`,
      ).toBe("svg");
    }

    // And no tile should fall back to the generic Globe / `data-fallback`.
    const fallbacks = document.querySelectorAll('[data-fallback="true"]');
    expect(fallbacks).toHaveLength(0);
  });

  it("renders the generic fallback (Globe) for an unknown slug, without tagging it with data-slug", () => {
    const unknownSlug = "xyz-not-real";
    catalog.entries = [
      minCatalogEntry(unknownSlug, "Fake Unknown Service"),
    ] as unknown as typeof catalog.entries;
    render(<AddKeyDialog open onOpenChange={vi.fn()} />);

    // Unknown slug → FallbackIcon → exactly one `<svg data-fallback="true">`
    // node (tighter than `toBeGreaterThan(0)`; Codex review nit).
    const fallbacks = document.querySelectorAll('[data-fallback="true"]');
    expect(fallbacks).toHaveLength(1);

    // Crucially, the fallback must NOT pretend to be a per-slug icon: the
    // `data-slug="xyz-not-real"` selector must miss entirely.
    expect(document.querySelector(`[data-slug="${unknownSlug}"]`)).toBeNull();
  });
});
