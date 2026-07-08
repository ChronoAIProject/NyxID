import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { CatalogEntry, KeyInfo } from "@/types/keys";
import { ApiError } from "@/lib/api-client";
import { SPEC_CATALOG_SLUGS } from "@/components/service-icons";
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
  toastFns,
} = vi.hoisted(() => ({
  catalog: { entries: [] as unknown[] },
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
  useCatalog: () => ({ data: catalog.entries, isLoading: false }),
  useCreateKey: () => ({
    mutate: createKeyMutate,
    mutateAsync: createKeyMutateAsync,
    isPending: false,
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
  catalog.entries = [OPENAI_ENTRY];
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
  it("creates a key from a hand-entered endpoint and navigates to it", async () => {
    createKeyMutate.mockImplementation((_params, opts) => {
      opts?.onSuccess?.({ id: "new-key-1" });
    });
    const user = userEvent.setup();
    render(
      <AddKeyDialog open onOpenChange={vi.fn()} />,
    );

    // Catalog step → choose "Custom Endpoint".
    await user.click(
      screen.getByRole("button", { name: /Custom Endpoint/i }),
    );
    // Routing step → keep the default "Direct" routing.
    await user.click(
      screen.getByRole("button", { name: /Next: Enter Credentials/i }),
    );

    // Form step → fill the custom endpoint, label and credential.
    await typeInto(user, "add-key-label", "My Custom API");
    await typeInto(user, "add-key-credential", "sk-custom-123");
    await typeInto(
      user,
      "add-key-endpoint",
      "https://my.endpoint/v1",
    );

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
    render(
      <AddKeyDialog open onOpenChange={vi.fn()} />,
    );

    await user.click(
      screen.getByRole("button", { name: /Custom Endpoint/i }),
    );
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

    await user.click(
      screen.getByRole("button", { name: /Custom Endpoint/i }),
    );
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
  it("creates a key from a catalog entry, omitting params that match catalog defaults", async () => {
    createKeyMutate.mockImplementation((_params, opts) => {
      opts?.onSuccess?.({ id: "new-key-2" });
    });
    const user = userEvent.setup();
    render(
      <AddKeyDialog open onOpenChange={vi.fn()} />,
    );

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

    await user.click(screen.getByRole("button", { name: /Connect with GitHub/i }));

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
    expect(mockHardRedirect).toHaveBeenCalledWith(
      "https://provider.example/oauth",
    );
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

    await user.click(screen.getByRole("button", { name: /Connect with GitHub/i }));

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

    await user.click(screen.getByRole("button", { name: /Connect with GitHub/i }));
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

// Build a minimal catalog entry suitable for CatalogGrid rendering. The
// dialog only reads `slug`, `name`, `description`, `base_url`,
// `service_type`, `requires_gateway_url`, and `provider_type` from each entry
// when rendering the tile; everything else can be cast through `unknown` so
// we don't duplicate the full `CatalogEntry` shape just for icon assertions.
function minCatalogEntry(
  slug: string,
  name = slug,
): CatalogEntry {
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
