import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ProviderConfig } from "@/types/api";
import { ApiError } from "@/lib/api-client";

const {
  mockNavigate,
  mockDeleteAsync,
  mockToastError,
  mockToastSuccess,
  providerState,
} = vi.hoisted(() => ({
  mockNavigate: vi.fn(),
  mockDeleteAsync: vi.fn(),
  mockToastError: vi.fn(),
  mockToastSuccess: vi.fn(),
  // Mutable container so each test can swap the useProvider() result.
  providerState: {
    data: undefined as ProviderConfig | undefined,
    isLoading: false,
    error: null as unknown,
    deletePending: false,
  },
}));

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
  useParams: () => ({ providerId: "provider-1" }),
}));

vi.mock("@/hooks/use-providers", () => ({
  useProvider: () => ({
    data: providerState.data,
    isLoading: providerState.isLoading,
    error: providerState.error,
    refetch: vi.fn(),
  }),
  useDeleteProvider: () => ({
    mutateAsync: mockDeleteAsync,
    isPending: providerState.deletePending,
  }),
}));

vi.mock("@/components/layout/dashboard-layout", () => ({
  useBreadcrumbLabel: vi.fn(),
}));

vi.mock("sonner", () => ({
  toast: {
    success: mockToastSuccess,
    error: mockToastError,
  },
}));

import { ProviderDetailPage } from "./provider-detail";

function makeProvider(overrides: Partial<ProviderConfig> = {}): ProviderConfig {
  return {
    id: "provider-1",
    slug: "openai",
    name: "OpenAI",
    description: "OpenAI provider",
    provider_type: "oauth2",
    has_oauth_config: true,
    credential_mode: "both",
    default_scopes: ["read", "write"],
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
    documentation_url: null,
    is_active: true,
    created_at: "2026-04-20T00:00:00Z",
    updated_at: "2026-04-21T00:00:00Z",
    ...overrides,
  };
}

describe("ProviderDetailPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    providerState.data = makeProvider();
    providerState.isLoading = false;
    providerState.error = null;
    providerState.deletePending = false;
    mockDeleteAsync.mockResolvedValue(undefined);
  });

  it("renders skeletons (no header/sections) while the provider query is loading", () => {
    providerState.isLoading = true;
    providerState.data = undefined;

    const { container } = render(<ProviderDetailPage />);

    // No provider name heading yet; only skeleton placeholders.
    expect(screen.queryByText("OpenAI")).not.toBeInTheDocument();
    expect(container.querySelectorAll(".animate-pulse").length).toBeGreaterThan(0);
  });

  it("shows a Not Found banner with the ApiError message when the query errors", () => {
    providerState.data = undefined;
    providerState.error = new ApiError(404, {
      error: "not_found",
      error_code: 1,
      message: "provider gone",
    });

    render(<ProviderDetailPage />);

    expect(
      screen.getByRole("heading", { name: "Provider Not Found" }),
    ).toBeInTheDocument();
    expect(screen.getByText("provider gone")).toBeInTheDocument();
  });

  it("falls back to a generic message when error is not an ApiError", () => {
    providerState.data = undefined;
    providerState.error = new Error("boom");

    render(<ProviderDetailPage />);

    expect(
      screen.getByText(/does not exist or has been deleted/i),
    ).toBeInTheDocument();
  });

  it("renders the OAuth 2.0 configuration section and default-scope badges for an oauth2 provider", () => {
    providerState.data = makeProvider({
      provider_type: "oauth2",
      default_scopes: ["read:email", "write:repo"],
    });

    render(<ProviderDetailPage />);

    expect(screen.getByRole("heading", { name: "OpenAI" })).toBeInTheDocument();
    expect(
      screen.getByText("OAuth 2.0 Configuration"),
    ).toBeInTheDocument();
    // OAuth 2.0 type label maps via PROVIDER_TYPE_LABELS.
    expect(screen.getByText("OAuth 2.0")).toBeInTheDocument();
    // Each scope is rendered as its own badge.
    expect(screen.getByText("read:email")).toBeInTheDocument();
    expect(screen.getByText("write:repo")).toBeInTheDocument();
    // oauth2/device-code branch shows the Credential Mode row.
    expect(screen.getByText("Credential Mode")).toBeInTheDocument();
    expect(screen.getByText("Admin or User")).toBeInTheDocument();
  });

  it("renders the Device Code section and its URLs for a device_code provider", () => {
    providerState.data = makeProvider({
      provider_type: "device_code",
      device_code_url: "https://dev.example.com/code",
      device_token_url: "https://dev.example.com/token",
    });

    render(<ProviderDetailPage />);

    expect(
      screen.getByText("Device Code Configuration (RFC 8628)"),
    ).toBeInTheDocument();
    expect(screen.getByText("https://dev.example.com/code")).toBeInTheDocument();
    expect(screen.getByText("https://dev.example.com/token")).toBeInTheDocument();
    // OAuth 2.0 section must NOT render for a device_code provider.
    expect(
      screen.queryByText("OAuth 2.0 Configuration"),
    ).not.toBeInTheDocument();
  });

  it("renders the API Key section with instructions for an api_key provider", () => {
    providerState.data = makeProvider({
      provider_type: "api_key",
      api_key_instructions: "Paste your secret key here",
      api_key_url: "https://example.com/keys",
      credential_mode: "user",
    });

    render(<ProviderDetailPage />);

    expect(screen.getByText("API Key Configuration")).toBeInTheDocument();
    expect(screen.getByText("Paste your secret key here")).toBeInTheDocument();
    expect(screen.getByText("https://example.com/keys")).toBeInTheDocument();
    // api_key type label, and NO Credential Mode row (not oauth/device).
    expect(screen.getByText("API Key")).toBeInTheDocument();
    expect(screen.queryByText("Credential Mode")).not.toBeInTheDocument();
  });

  it("renders the Telegram Widget section with the bot username for a telegram provider", () => {
    providerState.data = makeProvider({
      provider_type: "telegram_widget",
      client_id_param_name: "MyBot",
    });

    render(<ProviderDetailPage />);

    expect(
      screen.getByText("Telegram Widget Configuration"),
    ).toBeInTheDocument();
    expect(screen.getByText("@MyBot")).toBeInTheDocument();
    expect(screen.getByText("Telegram Widget")).toBeInTheDocument();
  });

  it("navigates to the edit route when the Edit action is clicked", async () => {
    const user = userEvent.setup();
    render(<ProviderDetailPage />);

    await user.click(screen.getByRole("button", { name: /edit/i }));

    expect(mockNavigate).toHaveBeenCalledWith({
      to: "/providers/$providerId/edit",
      params: { providerId: "provider-1" },
    });
  });

  it("deletes the provider, toasts success, and navigates back to manage on confirm", async () => {
    const user = userEvent.setup();
    render(<ProviderDetailPage />);

    await user.click(screen.getByRole("button", { name: /delete/i }));
    // Dialog confirm button (second "Delete" — the one inside the dialog footer).
    const confirmButtons = screen.getAllByRole("button", { name: "Delete" });
    await user.click(confirmButtons[confirmButtons.length - 1]!);

    await waitFor(() => {
      expect(mockDeleteAsync).toHaveBeenCalledWith("provider-1");
    });
    expect(mockToastSuccess).toHaveBeenCalledWith(
      "Provider deleted successfully",
    );
    expect(mockNavigate).toHaveBeenCalledWith({ to: "/providers/manage" });
  });

  it("toasts an error and does not navigate when delete fails", async () => {
    mockDeleteAsync.mockRejectedValue(new Error("nope"));
    const user = userEvent.setup();
    render(<ProviderDetailPage />);

    await user.click(screen.getByRole("button", { name: /delete/i }));
    const confirmButtons = screen.getAllByRole("button", { name: "Delete" });
    await user.click(confirmButtons[confirmButtons.length - 1]!);

    await waitFor(() => {
      expect(mockToastError).toHaveBeenCalledWith("Failed to delete provider");
    });
    expect(mockNavigate).not.toHaveBeenCalledWith({ to: "/providers/manage" });
  });
});
