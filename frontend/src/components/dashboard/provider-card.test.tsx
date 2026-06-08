import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { ProviderConfig, UserProviderToken } from "@/types/api";
import { ProviderCard } from "./provider-card";

const telegramProvider: ProviderConfig = {
  id: "provider-telegram",
  slug: "telegram",
  name: "Telegram",
  description: "Connect via Telegram Login Widget",
  provider_type: "telegram_widget",
  has_oauth_config: true,
  credential_mode: "admin",
  default_scopes: null,
  supports_pkce: false,
  device_code_url: null,
  device_token_url: null,
  device_verification_url: null,
  hosted_callback_url: null,
  api_key_instructions: null,
  api_key_url: null,
  token_endpoint_auth_method: "client_secret_post",
  extra_auth_params: null,
  device_code_format: "rfc8628",
  client_id_param_name: "NyxIdBot",
  icon_url: null,
  documentation_url: "https://core.telegram.org/widgets/login",
  is_active: true,
  created_at: "2026-03-09T00:00:00Z",
  requires_gateway_url: false,
  updated_at: "2026-03-09T00:00:00Z",
};

const oauthProvider: ProviderConfig = {
  ...telegramProvider,
  id: "provider-github",
  slug: "github",
  name: "GitHub",
  description: "Connect GitHub",
  provider_type: "oauth2",
  credential_mode: "admin",
  default_scopes: ["repo"],
  supports_pkce: true,
  documentation_url: "https://docs.github.com",
};

const deviceCodeProvider: ProviderConfig = {
  ...oauthProvider,
  id: "provider-device",
  slug: "device-provider",
  name: "Device Provider",
  provider_type: "device_code",
  device_code_url: "https://example.com/device/code",
  device_token_url: "https://example.com/device/token",
  device_verification_url: "https://example.com/device",
};

function makeTelegramToken(
  metadata: Record<string, string>,
): UserProviderToken {
  return makeToken(telegramProvider, "active", metadata);
}

function makeToken(
  provider: ProviderConfig,
  status: UserProviderToken["status"],
  metadata: Record<string, string> | null = null,
): UserProviderToken {
  return {
    provider_id: provider.id,
    provider_name: provider.name,
    provider_slug: provider.slug,
    provider_type: provider.provider_type,
    status,
    label: null,
    expires_at: null,
    last_used_at: null,
    connected_at: "2026-03-10T00:00:00Z",
    metadata,
    gateway_url: null,
  };
}

function renderCard(token?: UserProviderToken, provider = telegramProvider) {
  return render(
    <ProviderCard
      provider={provider}
      token={token}
      llmStatus={undefined}
      gatewayUrl=""
      hasUserCredentials={false}
      onConnect={vi.fn()}
      onDisconnect={vi.fn()}
      onRefresh={vi.fn()}
      onSetupCredentials={vi.fn()}
      isConnecting={false}
      isDisconnecting={false}
      isRefreshing={false}
    />,
  );
}

describe("ProviderCard", () => {
  it("renders telegram identity metadata for connected accounts", () => {
    const token = makeTelegramToken({
      username: "nyx_user",
      first_name: "Nyx",
      photo_url: "https://cdn.example.com/nyx.png",
    });

    const { container } = renderCard(token);

    expect(screen.getByText("@nyx_user")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Disconnect" })).toBeEnabled();
    expect(
      screen.queryByRole("button", { name: "Refresh" }),
    ).not.toBeInTheDocument();

    const avatar = container.querySelector("img.rounded-full");
    expect(avatar).not.toBeNull();
    expect(avatar).toHaveAttribute("src", "https://cdn.example.com/nyx.png");
  });

  it("falls back to the first name when telegram username is unavailable", () => {
    const token = makeTelegramToken({
      first_name: "Nyx",
    });

    renderCard(token);

    expect(screen.getByText("Nyx")).toBeInTheDocument();
  });

  it("does not render photo for non-https photo_url", () => {
    const token = makeTelegramToken({
      username: "nyx_user",
      first_name: "Nyx",
      photo_url: "javascript:alert(1)",
    });

    const { container } = renderCard(token);

    const avatar = container.querySelector("img.rounded-full");
    expect(avatar).toBeNull();
    expect(screen.getByText("@nyx_user")).toBeInTheDocument();
  });

  it("does not render photo for data: URI photo_url", () => {
    const token = makeTelegramToken({
      username: "nyx_user",
      first_name: "Nyx",
      photo_url: "data:text/html,<script>alert(1)</script>",
    });

    const { container } = renderCard(token);

    const avatar = container.querySelector("img.rounded-full");
    expect(avatar).toBeNull();
  });

  it("disables connect for unconfigured telegram providers", () => {
    const provider: ProviderConfig = {
      ...telegramProvider,
      has_oauth_config: false,
    };

    renderCard(undefined, provider);

    expect(
      screen.getByRole("button", { name: "Setup required" }),
    ).toBeDisabled();
    expect(
      screen.getByText(
        "Admin must configure the Telegram bot username and bot token first.",
      ),
    ).toBeInTheDocument();
  });

  it.each(["pending_auth", "failed", "refresh_failed"] as const)(
    "shows reconnect for OAuth token status %s",
    (status) => {
      const onConnect = vi.fn();

      render(
        <ProviderCard
          provider={oauthProvider}
          token={makeToken(oauthProvider, status)}
          llmStatus={undefined}
          gatewayUrl=""
          hasUserCredentials
          onConnect={onConnect}
          onDisconnect={vi.fn()}
          onRefresh={vi.fn()}
          onSetupCredentials={vi.fn()}
          isConnecting={false}
          isDisconnecting={false}
          isRefreshing={false}
        />,
      );

      fireEvent.click(screen.getByRole("button", { name: "Reconnect" }));

      expect(onConnect).toHaveBeenCalledWith(oauthProvider, true);
      expect(
        screen.queryByRole("button", { name: "Refresh" }),
      ).not.toBeInTheDocument();
    },
  );

  it("keeps refresh for expired OAuth tokens", () => {
    const onRefresh = vi.fn();

    render(
      <ProviderCard
        provider={oauthProvider}
        token={makeToken(oauthProvider, "expired")}
        llmStatus={undefined}
        gatewayUrl=""
        hasUserCredentials={false}
        onConnect={vi.fn()}
        onDisconnect={vi.fn()}
        onRefresh={onRefresh}
        onSetupCredentials={vi.fn()}
        isConnecting={false}
        isDisconnecting={false}
        isRefreshing={false}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Refresh" }));

    expect(onRefresh).toHaveBeenCalledWith(oauthProvider.id);
    expect(
      screen.queryByRole("button", { name: "Reconnect" }),
    ).not.toBeInTheDocument();
  });

  it("does not offer OAuth reconnect for pending device-code tokens", () => {
    renderCard(
      makeToken(deviceCodeProvider, "pending_auth"),
      deviceCodeProvider,
    );

    expect(screen.getByText("Pending Auth")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Reconnect" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Refresh" }),
    ).not.toBeInTheDocument();
  });
});
