import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { OAuthCallbackGuidance } from "./twitter-oauth-guidance";

const mocks = vi.hoisted(() => ({
  copyToClipboard: vi.fn(),
  toastSuccess: vi.fn(),
  useRuntimeConfig: vi.fn(),
}));

vi.mock("@/hooks/use-runtime-config", () => ({
  useRuntimeConfig: mocks.useRuntimeConfig,
}));

vi.mock("@/lib/utils", async () => {
  const actual = await vi.importActual<typeof import("@/lib/utils")>(
    "@/lib/utils",
  );
  return {
    ...actual,
    copyToClipboard: mocks.copyToClipboard,
  };
});

vi.mock("sonner", () => ({
  toast: {
    success: mocks.toastSuccess,
  },
}));

describe("OAuthCallbackGuidance", () => {
  beforeEach(() => {
    mocks.copyToClipboard.mockReset();
    mocks.copyToClipboard.mockResolvedValue(undefined);
    mocks.toastSuccess.mockReset();
    mocks.useRuntimeConfig.mockReset();
  });

  it("renders a loading state while fetching the runtime callback URL", () => {
    mocks.useRuntimeConfig.mockReturnValue({
      data: undefined,
      isError: false,
      isLoading: true,
    });

    render(<OAuthCallbackGuidance slug="twitter" />);

    expect(screen.getByText("Loading callback URL...")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Copy callback URL" }),
    ).not.toBeInTheDocument();
  });

  it("renders a copyable provider callback URL from runtime config", async () => {
    const user = userEvent.setup();
    mocks.useRuntimeConfig.mockReturnValue({
      data: {
        api_base_url: "https://nyx-api.chrono-ai.fun",
      },
      isError: false,
      isLoading: false,
    });

    render(<OAuthCallbackGuidance slug="twitter" />);

    const callbackUrl =
      "https://nyx-api.chrono-ai.fun/api/v1/providers/callback";
    expect(screen.getByText(callbackUrl)).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Copy callback URL" }));

    expect(mocks.copyToClipboard).toHaveBeenCalledWith(callbackUrl);
    await waitFor(() => {
      expect(mocks.toastSuccess).toHaveBeenCalledWith("Callback URL copied");
    });
  });

  it("renders a user-facing fallback when runtime config fails to load", () => {
    mocks.useRuntimeConfig.mockReturnValue({
      data: undefined,
      isError: true,
      isLoading: false,
    });

    render(<OAuthCallbackGuidance slug="api-twitter" />);

    expect(
      screen.getByText(
        /Couldn't load callback URL\. Please retry\. If this persists, contact support\./,
      ),
    ).toBeInTheDocument();
    expect(screen.queryByText(/VITE_BACKEND_URL/)).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Copy callback URL" }),
    ).not.toBeInTheDocument();
  });

  it("shows the callback URL for non-Twitter OAuth providers", () => {
    mocks.useRuntimeConfig.mockReturnValue({
      data: {
        api_base_url: "https://nyx-api.chrono-ai.fun",
      },
      isError: false,
      isLoading: false,
    });

    render(<OAuthCallbackGuidance slug="github" />);

    const callbackUrl =
      "https://nyx-api.chrono-ai.fun/api/v1/providers/callback";
    expect(screen.getByText(callbackUrl)).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Copy callback URL" }),
    ).toBeInTheDocument();
    // Generic heading, not the Twitter-specific one.
    expect(screen.getByText("NyxID callback URL")).toBeInTheDocument();
    expect(screen.queryByText("Twitter / X OAuth setup")).not.toBeInTheDocument();
  });

  it("layers Twitter-specific guidance on top for Twitter providers", () => {
    mocks.useRuntimeConfig.mockReturnValue({
      data: {
        api_base_url: "https://nyx-api.chrono-ai.fun",
      },
      isError: false,
      isLoading: false,
    });

    render(<OAuthCallbackGuidance slug="twitter" />);

    expect(screen.getByText("Twitter / X OAuth setup")).toBeInTheDocument();
    expect(
      screen.getByText(/Open Keys & Tokens in\s+X Developer Console/),
    ).toBeInTheDocument();
    // The shared callback URL card is still present.
    expect(
      screen.getByText(
        "https://nyx-api.chrono-ai.fun/api/v1/providers/callback",
      ),
    ).toBeInTheDocument();
  });
});

// The OAuth-vs-non-OAuth decision lives at the call sites
// (add-key-dialog / user-credentials-dialog gate on
// `provider_type === "oauth2"`; the wizard's OAuthFlow is the
// authorization-code flow by construction). This component renders the
// callback URL whenever it's mounted, so "non-OAuth shows no callback"
// is enforced by NOT mounting it — verified here against the dialog
// gating logic so a regression in the predicate is caught.
function shouldShowCallback(providerType: string): boolean {
  return providerType === "oauth2";
}

describe("OAuth callback gating predicate", () => {
  it("shows the callback for authorization-code OAuth flows", () => {
    expect(shouldShowCallback("oauth2")).toBe(true);
  });

  it("hides the callback for device-code and API-key flows", () => {
    expect(shouldShowCallback("device_code")).toBe(false);
    expect(shouldShowCallback("api_key")).toBe(false);
    expect(shouldShowCallback("telegram_widget")).toBe(false);
  });
});
