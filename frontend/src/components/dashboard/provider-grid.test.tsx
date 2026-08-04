import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
  ProviderConfig,
  UserProviderCredentials,
  UserProviderToken,
} from "@/types/api";
import { ProviderGrid } from "./provider-grid";

const mocks = vi.hoisted(() => ({
  useProviders: vi.fn(),
  useMyProviderTokens: vi.fn(),
  useConnectApiKey: vi.fn(),
  useInitiateOAuth: vi.fn(),
  useDisconnectProvider: vi.fn(),
  useRefreshProviderToken: vi.fn(),
  useMyProviderCredentials: vi.fn(),
  useLlmStatus: vi.fn(),
  useOrgs: vi.fn(),
  hardRedirect: vi.fn(),
  toastError: vi.fn(),
  toastSuccess: vi.fn(),
}));

vi.mock("@/hooks/use-providers", () => ({
  useProviders: mocks.useProviders,
  useMyProviderTokens: mocks.useMyProviderTokens,
  useConnectApiKey: mocks.useConnectApiKey,
  useInitiateOAuth: mocks.useInitiateOAuth,
  useDisconnectProvider: mocks.useDisconnectProvider,
  useRefreshProviderToken: mocks.useRefreshProviderToken,
  useMyProviderCredentials: mocks.useMyProviderCredentials,
}));

vi.mock("@/hooks/use-llm-gateway", () => ({
  useLlmStatus: mocks.useLlmStatus,
}));

vi.mock("@/hooks/use-orgs", () => ({
  useOrgs: mocks.useOrgs,
}));

vi.mock("@/components/shared/org-scope-select", () => ({
  OrgScopeSelect: ({
    value,
    onChange,
    label,
  }: {
    readonly value: string | null;
    readonly onChange: (value: string | null) => void;
    readonly label?: string;
  }) => (
    <select
      aria-label={label ?? "Scope"}
      value={value ?? ""}
      onChange={(event) => onChange(event.target.value || null)}
    >
      <option value="">Personal</option>
      <option value="org-1">Acme Org</option>
    </select>
  ),
}));

vi.mock("@/lib/navigation", () => ({
  hardRedirect: mocks.hardRedirect,
}));

vi.mock("sonner", () => ({
  toast: {
    error: mocks.toastError,
    success: mocks.toastSuccess,
  },
}));

vi.mock("./api-key-dialog", () => ({
  ApiKeyDialog: () => null,
}));

vi.mock("./device-code-dialog", () => ({
  DeviceCodeDialog: () => null,
}));

vi.mock("./user-credentials-dialog", () => ({
  UserCredentialsDialog: () => null,
}));

vi.mock("./telegram-login-dialog", () => ({
  TelegramLoginDialog: () => null,
}));

const provider: ProviderConfig = {
  id: "provider-twitter",
  slug: "twitter",
  name: "Twitter",
  description: "Connect your own Twitter app",
  provider_type: "oauth2",
  has_oauth_config: false,
  credential_mode: "user",
  default_scopes: ["tweet.read"],
  supports_pkce: true,
  device_code_url: null,
  device_token_url: null,
  device_verification_url: null,
  hosted_callback_url: null,
  api_key_instructions: null,
  api_key_url: null,
  token_endpoint_auth_method: "client_secret_post",
  extra_auth_params: null,
  device_code_format: "rfc8628",
  client_id_param_name: null,
  requires_gateway_url: false,
  icon_url: null,
  documentation_url: "https://developer.x.com",
  is_active: true,
  created_at: "2026-03-09T00:00:00Z",
  updated_at: "2026-03-09T00:00:00Z",
};

const userCredentials: UserProviderCredentials = {
  provider_config_id: provider.id,
  has_credentials: true,
  label: "My Twitter App",
  created_at: "2026-03-09T00:00:00Z",
  updated_at: "2026-03-09T00:00:00Z",
};

const providerToken: UserProviderToken = {
  provider_id: provider.id,
  provider_name: provider.name,
  provider_slug: provider.slug,
  provider_type: provider.provider_type,
  status: "active",
  label: null,
  gateway_url: null,
  expires_at: null,
  last_used_at: null,
  connected_at: "2026-03-09T00:00:00Z",
  metadata: null,
};

describe("ProviderGrid", () => {
  beforeEach(() => {
    vi.clearAllMocks();

    mocks.useProviders.mockReturnValue({
      data: [provider],
      isLoading: false,
    });
    mocks.useMyProviderTokens.mockReturnValue({
      data: [],
      isLoading: false,
    });
    mocks.useConnectApiKey.mockReturnValue({
      mutateAsync: vi.fn(),
      isPending: false,
    });
    mocks.useDisconnectProvider.mockReturnValue({
      mutateAsync: vi.fn(),
      isPending: false,
    });
    mocks.useRefreshProviderToken.mockReturnValue({
      mutateAsync: vi.fn(),
      isPending: false,
    });
    mocks.useLlmStatus.mockReturnValue({
      data: undefined,
    });
    mocks.useOrgs.mockReturnValue({
      data: [],
      isLoading: false,
    });
    mocks.useMyProviderCredentials.mockImplementation((providerId: string) => ({
      data: providerId === provider.id ? userCredentials : undefined,
    }));
  });

  it("initiates OAuth for a user-mode provider when per-user credentials exist", async () => {
    const user = userEvent.setup();
    const mutateAsync = vi.fn().mockResolvedValue({
      authorization_url: "https://example.com/oauth/authorize",
    });
    mocks.useInitiateOAuth.mockReturnValue({
      mutateAsync,
      isPending: false,
    });

    render(<ProviderGrid />);

    await user.click(screen.getByRole("button", { name: "Connect" }));

    await waitFor(() => {
      expect(mutateAsync).toHaveBeenCalledWith(provider.id);
    });
    expect(mocks.hardRedirect).toHaveBeenCalledWith(
      "https://example.com/oauth/authorize",
    );
    expect(mocks.toastError).not.toHaveBeenCalled();
  });

  it("passes the selected org scope to the provider token query", async () => {
    const user = userEvent.setup();
    mocks.useOrgs.mockReturnValue({
      data: [
        {
          id: "org-1",
          display_name: "Acme Org",
          avatar_url: null,
          contact_email: null,
          your_role: "admin",
          created_at: "2026-03-09T00:00:00Z",
        },
      ],
      isLoading: false,
    });
    mocks.useInitiateOAuth.mockReturnValue({
      mutateAsync: vi.fn(),
      isPending: false,
    });

    render(<ProviderGrid />);

    expect(mocks.useMyProviderTokens).toHaveBeenLastCalledWith({
      targetOrgId: null,
    });

    await user.selectOptions(
      screen.getByLabelText("Provider token owner"),
      "org-1",
    );

    await waitFor(() => {
      expect(mocks.useMyProviderTokens).toHaveBeenLastCalledWith({
        targetOrgId: "org-1",
      });
    });
    expect(
      screen.getByText("No provider tokens for Acme Org."),
    ).toBeInTheDocument();
  });

  it("passes the selected org scope when disconnecting a provider token", async () => {
    const user = userEvent.setup();
    const disconnect = vi.fn().mockResolvedValue({
      status: "disconnected",
      message: "Provider disconnected",
    });
    mocks.useOrgs.mockReturnValue({
      data: [
        {
          id: "org-1",
          display_name: "Acme Org",
          avatar_url: null,
          contact_email: null,
          your_role: "admin",
          created_at: "2026-03-09T00:00:00Z",
        },
      ],
      isLoading: false,
    });
    mocks.useMyProviderTokens.mockImplementation(
      ({ targetOrgId }: { readonly targetOrgId: string | null }) => ({
        data: targetOrgId === "org-1" ? [providerToken] : [],
        isLoading: false,
      }),
    );
    mocks.useDisconnectProvider.mockReturnValue({
      mutateAsync: disconnect,
      isPending: false,
    });
    mocks.useInitiateOAuth.mockReturnValue({
      mutateAsync: vi.fn(),
      isPending: false,
    });

    render(<ProviderGrid />);

    await user.selectOptions(
      screen.getByLabelText("Provider token owner"),
      "org-1",
    );
    await user.click(screen.getByRole("button", { name: "Disconnect" }));

    await waitFor(() => {
      expect(disconnect).toHaveBeenCalledWith({
        providerId: provider.id,
        targetOrgId: "org-1",
      });
    });
  });
});
