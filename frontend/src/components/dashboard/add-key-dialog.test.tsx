import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { CatalogEntry, KeyInfo } from "@/types/keys";
import { ApiError } from "@/lib/api-client";
import { AddKeyDialog } from "./add-key-dialog";

const {
  catalog,
  createKeyMutate,
  createKeyMutateAsync,
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

    await user.click(screen.getByRole("button", { name: "Create Service" }));

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
    expect(toastFns.success).toHaveBeenCalledWith("Key created");
    expect(mockNavigate).toHaveBeenCalledWith({
      to: "/keys/$keyId",
      params: { keyId: "new-key-1" },
    });
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
    await typeInto(user, "add-key-endpoint", "not-a-url");
    await user.click(screen.getByRole("button", { name: "Create Service" }));

    await waitFor(() =>
      expect(toastFns.error).toHaveBeenCalledWith("Endpoint URL is invalid"),
    );
    expect(mockNavigate).not.toHaveBeenCalled();
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
    await user.click(screen.getByRole("button", { name: "Create Service" }));

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
    expect(toastFns.success).toHaveBeenCalledWith("Key created");
    expect(mockNavigate).toHaveBeenCalledWith({
      to: "/keys/$keyId",
      params: { keyId: "new-key-2" },
    });
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
      additionalScopes: [],
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
      additionalScopes: [],
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
