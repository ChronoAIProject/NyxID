import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  fireEvent,
  render,
  screen,
  waitFor,
  cleanup,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { NyxbotOnboardingPage } from "./nyxbot-onboarding";
import { nyxbotI18n } from "@/features/nyxbot-onboarding/i18n";
import { GOOGLE_WORKSPACE_SCOPES } from "@/schemas/nyxbot-onboarding";

const { get, post, redirect, auth } = vi.hoisted(() => ({
  get: vi.fn(),
  post: vi.fn(),
  redirect: vi.fn(),
  auth: { user: { id: "owner" }, isAuthenticated: true },
}));
vi.mock("@/lib/api-client", () => ({ api: { get, post } }));
vi.mock("@/lib/navigation", () => ({
  hardRedirect: redirect,
  openExternal: redirect,
}));
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (selector: (s: typeof auth) => unknown) => selector(auth),
}));
vi.mock("@/components/auth/web-device-login", () => ({
  WebDeviceLogin: () => <button>Continue with the NyxID app</button>,
}));

const googleKey = {
  id: "google-1",
  catalog_service_slug: "api-google",
  status: "active",
  is_active: true,
  granted_scopes: GOOGLE_WORKSPACE_SCOPES,
};
const catalog = {
  slug: "api-google",
  provider_config_id: "google-provider",
  credential_mode: "both",
  has_platform_oauth_credentials: true,
  platform_scope_allowlist: GOOGLE_WORKSPACE_SCOPES,
};
let keys: object[];
let catalogResponse: typeof catalog;
let client: QueryClient;
function mount() {
  client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <NyxbotOnboardingPage />
    </QueryClientProvider>,
  );
}

beforeEach(async () => {
  vi.clearAllMocks();
  sessionStorage.clear();
  window.history.replaceState(null, "", "/onboarding?channel=telegram");
  auth.isAuthenticated = true;
  keys = [googleKey];
  catalogResponse = catalog;
  await nyxbotI18n.changeLanguage("en");
  get.mockImplementation(async (path: string) => {
    if (path === "/keys") return { keys };
    if (path === "/keys/google-1") return keys[0];
    if (path === "/catalog/api-google") return catalogResponse;
    if (path === "/public/config")
      return {
        social_providers: ["google", "github", "apple"],
        email_auth_enabled: false,
      };
    if (path.startsWith("/providers/google-provider/connect/oauth"))
      return {
        authorization_url:
          "https://accounts.google.com/o/oauth2/v2/auth?state=test",
      };
    if (path === "/channel-bots/managed-onboarding/whatsapp")
      return { available: true, feature_types: [""] };
    if (path === "/channel-bots/business-bot")
      return {
        id: "business-bot",
        platform: "telegram",
        status: "active",
        is_active: true,
        webhook_registered: true,
      };
    throw new Error(`Unexpected request: ${path}`);
  });
  post.mockImplementation(async (path: string) => {
    if (path === "/keys") {
      keys = [{ ...googleKey, status: "pending_auth", granted_scopes: [] }];
      return keys[0];
    }
    if (path === "/channel-bots")
      return { id: "business-bot", platform: "telegram", status: "active" };
    throw new Error(`Unexpected mutation: ${path}`);
  });
});
afterEach(() => {
  cleanup();
  client?.clear();
});

async function toChannel() {
  await screen.findByText("Google Workspace connected");
  await userEvent.click(
    screen.getByRole("button", { name: "Continue", exact: true }),
  );
  await screen.findByRole("heading", { name: "Set up channel" });
}

describe("Nyxbot onboarding", () => {
  it("hands off social login with a same-origin return URL preserving channel", async () => {
    auth.isAuthenticated = false;
    mount();
    await userEvent.click(
      await screen.findByRole("button", { name: "Continue with Google" }),
    );
    const target = new URL(redirect.mock.calls[0]![0], window.location.origin);
    expect(target.pathname).toBe("/api/v1/auth/social/google");
    expect(target.searchParams.get("return_to")).toBe(
      `${window.location.origin}/onboarding?channel=telegram`,
    );
    expect(post).not.toHaveBeenCalled();
  });
  it("preselects a referral, registers Telegram once, and never fabricates a pairing code", async () => {
    mount();
    await toChannel();
    expect(screen.getByRole("radio", { name: /Telegram/ })).toBeChecked();
    const submit = screen.getByRole("button", { name: "Connect channel" });
    expect(submit).toBeDisabled();
    const token = screen.getByLabelText("Bot token", { exact: true });
    expect(token).toHaveAttribute("type", "password");
    await userEvent.type(token, "123456:test-token");
    await userEvent.click(submit);
    await screen.findByRole("heading", { name: "Almost there" });
    expect(post).toHaveBeenCalledWith("/channel-bots", {
      platform: "telegram",
      label: "Nyxbot Telegram",
      bot_token: "123456:test-token",
    });
    expect(post).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("button", { name: "Copy code" })).toBeDisabled();
    expect(
      screen.getByRole("button", { name: "Open Telegram" }),
    ).toBeDisabled();
    expect(screen.getByPlaceholderText("Not available yet")).toHaveValue("");
    expect(sessionStorage.getItem("nyxbot-onboarding:owner")).not.toContain(
      "test-token",
    );
    expect(
      await screen.findByRole("link", { name: "Manage channel" }),
    ).toHaveAttribute("href", "/channel-bots/business-bot");
  });
  it("leaves the channel unselected for a direct visitor and clears a token on channel switch", async () => {
    window.history.replaceState(null, "", "/onboarding");
    mount();
    await toChannel();
    expect(screen.getByRole("radio", { name: /Telegram/ })).not.toBeChecked();
    expect(screen.getByRole("radio", { name: /WhatsApp/ })).not.toBeChecked();
    await userEvent.click(screen.getByRole("radio", { name: /Telegram/ }));
    await userEvent.type(
      screen.getByLabelText("Bot token", { exact: true }),
      "unsent-token",
    );
    await userEvent.click(screen.getByRole("radio", { name: /WhatsApp/ }));
    await userEvent.click(screen.getByRole("radio", { name: /Telegram/ }));
    expect(screen.getByLabelText("Bot token", { exact: true })).toHaveValue("");
  });
  it("preserves Google authorization when token verification fails and redacts provider errors", async () => {
    post.mockRejectedValueOnce(new Error("Bad token: secret-test-value"));
    mount();
    await toChannel();
    await userEvent.type(
      screen.getByLabelText("Bot token", { exact: true }),
      "secret-test-value",
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Connect channel" }),
    );
    await screen.findByText(
      "Your channel could not be connected. Check the token and retry.",
    );
    expect(
      screen.queryByText("Bad token: secret-test-value"),
    ).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Back" }));
    await screen.findByText("Google Workspace connected");
  });
  it("keeps WhatsApp activation disabled without a server-enforced spending cap", async () => {
    mount();
    await toChannel();
    await userEvent.click(screen.getByRole("radio", { name: /WhatsApp/ }));
    expect(
      screen.getByRole("button", { name: "Connect WhatsApp" }),
    ).toBeDisabled();
    await userEvent.click(
      screen.getByRole("button", { name: "Review spending cap" }),
    );
    expect(screen.getByRole("radio", { name: /SGD 120/ })).toBeChecked();
    await userEvent.click(screen.getByRole("radio", { name: /SGD 250/ }));
    expect(
      screen.getByRole("button", { name: "Set cap & connect" }),
    ).toBeDisabled();
    expect(post).not.toHaveBeenCalled();
  });
  it("checks real scopes after OAuth instead of trusting a success URL", async () => {
    keys = [{ ...googleKey, granted_scopes: ["openid", "email", "profile"] }];
    window.history.replaceState(null, "", "/onboarding?status=success");
    mount();
    const button = await screen.findByRole("button", {
      name: "Connect Google",
    });
    await waitFor(() => expect(button).toBeEnabled());
    expect(
      screen.queryByText("Google Workspace connected"),
    ).not.toBeInTheDocument();
    await userEvent.click(button);
    await waitFor(() => expect(redirect).toHaveBeenCalled());
    const oauthCall = get.mock.calls.find(([path]) =>
      String(path).startsWith("/providers/google-provider/connect/oauth"),
    );
    const url = new URL(oauthCall![0], "http://localhost");
    expect(url.searchParams.get("key_id")).toBe("google-1");
    expect(url.searchParams.get("scope_override")?.split(",")).toEqual([
      ...GOOGLE_WORKSPACE_SCOPES,
    ]);
    expect(url.searchParams.get("redirect_path")).toBe("/onboarding");
    expect(
      JSON.parse(sessionStorage.getItem("nyxbot-onboarding:owner")!)
        .googleKeyId,
    ).toBe("google-1");
  });
  it("does not create a placeholder when the platform permits identity-only Google scopes", async () => {
    keys = [];
    catalogResponse = {
      ...catalog,
      platform_scope_allowlist: [
        "openid",
        "email",
        "profile",
      ] as unknown as typeof GOOGLE_WORKSPACE_SCOPES,
    };
    mount();
    await screen.findByText(/Google Workspace authorization is not available/);
    expect(
      screen.getByRole("button", { name: "Connect Google" }),
    ).toBeDisabled();
    expect(post).not.toHaveBeenCalled();
  });
  it("restores the saved channel from the API after a refresh", async () => {
    sessionStorage.setItem(
      "nyxbot-onboarding:owner",
      JSON.stringify({ botId: "business-bot", channel: "telegram" }),
    );
    mount();
    await screen.findByText(/Your channel is saved/);
    expect(get).toHaveBeenCalledWith("/channel-bots/business-bot");
    expect(post).not.toHaveBeenCalled();
  });
  it("opens contextual help and switches the feature's language", async () => {
    mount();
    await screen.findByText("Google Workspace connected");
    await userEvent.click(
      screen.getByRole("button", { name: "Why this step?" }),
    );
    await screen.findByText(
      /Connect Google Workspace to authorize Drive and Calendar together/,
    );
    fireEvent.change(screen.getByLabelText("Language"), {
      target: { value: "zh-CN" },
    });
    await screen.findByRole("heading", { name: "连接数据源" });
  });

  it("lets the owner recover when a previously registered bot was deleted", async () => {
    sessionStorage.setItem(
      "nyxbot-onboarding:owner",
      JSON.stringify({ botId: "deleted-bot", channel: "telegram" }),
    );
    const previousGet = get.getMockImplementation()!;
    get.mockImplementation((path: string) =>
      path === "/channel-bots/deleted-bot"
        ? Promise.reject(Object.assign(new Error("Not found"), { status: 404 }))
        : previousGet(path),
    );
    mount();
    await screen.findByText("We couldn't load your connections. Please retry.");
    await userEvent.click(screen.getByRole("button", { name: "Back" }));
    await toChannel();
    expect(
      JSON.parse(sessionStorage.getItem("nyxbot-onboarding:owner")!).botId,
    ).toBeNull();
  });

  it("drops a deleted OAuth placeholder on retry instead of trapping the user", async () => {
    sessionStorage.setItem(
      "nyxbot-onboarding:owner",
      JSON.stringify({ googleKeyId: "deleted-key" }),
    );
    const previousGet = get.getMockImplementation()!;
    get.mockImplementation((path: string) =>
      path === "/keys/deleted-key"
        ? Promise.reject(Object.assign(new Error("Not found"), { status: 404 }))
        : previousGet(path),
    );
    mount();
    await screen.findByText("We couldn't load your connections. Please retry.");
    await userEvent.click(screen.getByRole("button", { name: "Retry" }));
    await screen.findByText("Google Workspace connected");
    expect(
      JSON.parse(sessionStorage.getItem("nyxbot-onboarding:owner")!)
        .googleKeyId,
    ).toBeNull();
  });
});
