import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ProviderConfig, UserProviderCredentials } from "@/types/api";
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
});
