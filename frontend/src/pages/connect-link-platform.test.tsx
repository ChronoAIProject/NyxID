import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, afterEach, expect, it, vi } from "vitest";
import { ConnectLinkPage } from "./connect-link";

const mocks = vi.hoisted(() => ({
  complete: vi.fn(),
  navigate: vi.fn(),
  available: true,
  managedAurinko: false,
  choice: undefined as boolean | undefined,
  scopes: [] as string[],
}));
vi.mock("@tanstack/react-router", () => ({
  useParams: () => ({ token: "hosted-token" }),
  useNavigate: () => mocks.navigate,
}));
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: () => ({ isAuthenticated: true, isLoading: false }),
}));
vi.mock("@/hooks/use-keys", () => ({
  useCatalogEntry: () => ({
    data: {
      platform_key: { available: mocks.available, pricing: null },
      byok_pricing: null,
    },
  }),
}));
vi.mock("@/hooks/use-connect-links", () => ({
  usePreviewConnectLink: () => ({
    isPending: false,
    data: {
      status: "pending",
      service_name: "xAI",
      service_slug: "llm-xai",
      scopes: mocks.scopes,
      requested_by: "cli",
      expires_at: "2099-01-01T00:00:00Z",
      connect_method: mocks.managedAurinko ? "aurinko_account_code" : "api_key",
      managed_onboarding: mocks.managedAurinko ? "aurinko_account_code" : null,
      auth_key_name: "Authorization",
      use_platform_key: mocks.choice,
    },
  }),
  useCompleteConnectLink: () => ({
    mutateAsync: mocks.complete,
    isPending: false,
  }),
  useCancelHostedConnectLink: () => ({ isPending: false }),
  useConnectLinkStatus: () => ({}),
  connectLinkStorageKey: (id: string) => id,
}));
beforeEach(() => {
  vi.clearAllMocks();
  mocks.available = true;
  mocks.managedAurinko = false;
  mocks.choice = undefined;
  mocks.scopes = [];
  mocks.complete.mockResolvedValue({ status: "completed" });
  let now = 100_000;
  vi.spyOn(Date, "now").mockImplementation(() => (now += 1000));
});
afterEach(() => vi.restoreAllMocks());
it("defaults to platform and completes without any secret", async () => {
  render(<ConnectLinkPage />);
  expect(screen.getByRole("radio", { name: /Use NyxID's key/ })).toBeChecked();
  await userEvent.click(screen.getByRole("button", { name: "Connect" }));
  expect(mocks.complete).toHaveBeenCalledWith({
    token: "hosted-token",
    values: { use_platform_key: true },
  });
});
it.each([false, true])(
  "submits an explicit own-key choice when the creator choice is %s and access is revoked",
  async (choice) => {
    mocks.choice = choice;
    mocks.available = false;
    render(<ConnectLinkPage />);
    expect(screen.queryByRole("radio")).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Connect" }));
    await userEvent.type(
      screen.getByLabelText("Authorization"),
      "personal-secret",
    );
    await userEvent.click(screen.getByRole("button", { name: "Connect" }));
    expect(mocks.complete).toHaveBeenCalledWith({
      token: "hosted-token",
      values: expect.objectContaining({
        use_platform_key: false,
        credential: "personal-secret",
      }),
    });
  },
);
it("retains creator-requested OAuth scope setup without defaulting to a platform key", async () => {
  mocks.scopes = ["read:org"];
  render(<ConnectLinkPage />);
  expect(screen.queryByRole("radio")).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Connect" }));
  expect(screen.getByLabelText("Authorization")).toBeInTheDocument();
  expect(mocks.complete).not.toHaveBeenCalled();
});

it.each([["IMAP / SMTP", "IMAP"], ["Zoho Mail", "Zoho"]])("hosted mailbox request selects %s without application credentials", async (label, provider) => {
  mocks.available = false;
  mocks.managedAurinko = true;
  render(<ConnectLinkPage />);
  await userEvent.click(screen.getByRole("combobox", { name: "Email provider" }));
  await userEvent.click(screen.getByRole("option", { name: label }));
  await userEvent.click(screen.getByRole("button", { name: "Connect" }));
  expect(mocks.complete).toHaveBeenCalledWith({ token: "hosted-token", values: { aurinko_provider: provider, use_platform_key: false } });
  expect(screen.queryByLabelText(/client secret|client id/i)).not.toBeInTheDocument();
});
it("retains manual account-token entry alongside hosted mailbox sign-in", async () => {
  mocks.available = false;
  mocks.managedAurinko = true;
  render(<ConnectLinkPage />);
  await userEvent.click(screen.getByRole("button", { name: "Use an existing Aurinko account token" }));
  await userEvent.type(screen.getByLabelText("Authorization"), "mailbox-account-token");
  await userEvent.click(screen.getByRole("button", { name: "Connect" }));
  expect(mocks.complete).toHaveBeenCalledWith({ token: "hosted-token", values: expect.objectContaining({ credential: "mailbox-account-token", use_platform_key: false }) });
});
