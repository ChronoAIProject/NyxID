import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import {
  connectLinkErrorMessage,
  connectLinkNeedsOAuthCredentials,
  connectLinkNeedsSetupForm,
  connectLinkProviderError,
} from "@/lib/connect-link-page";
import type { ConnectLinkPreview } from "@/schemas/connect-links";
import { ConnectLinkDetailRow } from "@/pages/connect-link";

function preview(
  overrides: Partial<ConnectLinkPreview> = {},
): ConnectLinkPreview {
  return {
    service_name: "GitHub",
    service_slug: "github",
    label: null,
    requested_by: "codex",
    created_at: "2026-08-05T10:00:00Z",
    expires_at: "2026-08-05T10:15:00Z",
    status: "pending",
    connect_method: "oauth",
    auth_key_name: "Authorization",
    credential_mode: "admin",
    has_platform_oauth_credentials: true,
    requires_gateway_url: false,
    api_key_url: null,
    api_key_instructions: null,
    ...overrides,
  };
}

describe("connect link page error handling", () => {
  it("surfaces safe Error messages and uses a stable fallback", () => {
    expect(connectLinkErrorMessage(new Error("Link expired"))).toBe(
      "Link expired",
    );
    expect(connectLinkErrorMessage({ secret: "must not render" })).toBe(
      "The connection request failed.",
    );
  });

  it("shows setup fields for API keys, gateway URLs, and BYO OAuth", () => {
    expect(
      connectLinkNeedsSetupForm(preview({ connect_method: "api_key" })),
    ).toBe(true);
    expect(
      connectLinkNeedsSetupForm(preview({ requires_gateway_url: true })),
    ).toBe(true);
    expect(
      connectLinkNeedsOAuthCredentials(
        preview({
          credential_mode: "user",
          has_platform_oauth_credentials: false,
        }),
      ),
    ).toBe(true);
    expect(
      connectLinkNeedsSetupForm(
        preview({
          credential_mode: "both",
          has_platform_oauth_credentials: false,
        }),
      ),
    ).toBe(true);
    expect(connectLinkNeedsSetupForm(preview())).toBe(false);
  });

  it("recognizes OAuth provider errors on the hosted return route", () => {
    expect(
      connectLinkProviderError(
        "?provider_status=error&message=Authorization%20was%20denied",
      ),
    ).toBe("Authorization was denied");
    expect(connectLinkProviderError("?provider_status=success")).toBeNull();
  });

  it("capitalizes only the status detail value", () => {
    const { rerender } = render(
      <ConnectLinkDetailRow label="Requested by" value="codex-agent" />,
    );
    expect(screen.getByText("codex-agent")).not.toHaveClass("capitalize");

    rerender(
      <ConnectLinkDetailRow label="Status" value="pending" capitalizeValue />,
    );
    expect(screen.getByText("pending")).toHaveClass("capitalize");
  });
});
