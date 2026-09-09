import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  act,
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
import { ApiError } from "@/lib/api-client";
import { AevatarAuthError } from "@/lib/nyxbot-aevatar-auth";
import {
  AEVATAR_CHANNELS_PATH,
  AEVATAR_WEBHOOK_BASE_URL,
} from "@/lib/nyxbot-channels";

const { get, post, redirect, auth, telegram, authorizer } = vi.hoisted(() => ({
  get: vi.fn(),
  post: vi.fn(),
  redirect: vi.fn(),
  telegram: vi.fn(),
  authorizer: vi.fn(),
  auth: {
    user: { id: "owner", display_name: "Avery", email: "avery@example.com" },
    isAuthenticated: true,
  },
}));
vi.mock("@/lib/nyxbot-aevatar-auth", () => ({
  AEVATAR_ORIGIN: "https://aevatar-console-backend-api.aevatar.ai",
  AEVATAR_CHANNELS_URL:
    "https://aevatar-console-backend-api.aevatar.ai/channels",
  getAevatarAuthorization: authorizer,
  clearAevatarAuthorization: vi.fn(),
  AevatarAuthError: class extends Error {
    constructor(readonly code: string) {
      super(code);
    }
  },
}));
vi.mock("@/lib/api-client", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/api-client")>()),
  api: { get, post },
  apiClient: (path: string, options?: { method?: string; body?: unknown }) =>
    options?.method === "POST" ? post(path, options.body) : get(path),
}));
vi.mock("@/lib/navigation", () => ({
  hardRedirect: redirect,
  openExternal: redirect,
}));
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (selector: (s: typeof auth) => unknown) => selector(auth),
}));
vi.mock("@/components/auth/web-device-login", () => ({
  WebDeviceLogin: ({ returnTo }: { returnTo: string }) => (
    <button data-return-to={returnTo}>Continue with the NyxID app</button>
  ),
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
// Synthetic, syntax-valid fixture; API calls in these tests are mocked.
const telegramToken = `123456:${"aB_9-".repeat(7)}`;
const registrationReceipt = {
  status: "accepted",
  registration_id: "aevatar-registration",
  nyx_channel_bot_id: "business-bot",
  platform: "telegram",
};
let keys: object[];
let catalogResponse: typeof catalog;
let publicConfig: { social_providers: string[]; email_auth_enabled: boolean };
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
  authorizer.mockReset().mockResolvedValue("Bearer test-oauth-access");
  vi.stubGlobal("fetch", telegram);
  telegram.mockResolvedValue({
    ok: true,
    status: 200,
    json: async () => ({
      ok: true,
      result: { id: 123456, is_bot: true, first_name: "My Shop Bot" },
    }),
  });
  sessionStorage.clear();
  window.history.replaceState(
    null,
    "",
    "/onboarding?step=source&channel=telegram",
  );
  auth.isAuthenticated = true;
  keys = [googleKey];
  catalogResponse = catalog;
  publicConfig = {
    social_providers: ["google", "github", "apple"],
    email_auth_enabled: false,
  };
  await nyxbotI18n.changeLanguage("en");
  get.mockImplementation(async (path: string) => {
    if (path === "/keys") return { keys };
    if (path === "/keys/google-1") return keys[0];
    if (path === "/catalog/api-google") return catalogResponse;
    if (path === "/public/config") return publicConfig;
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
    if (path === `${AEVATAR_CHANNELS_PATH}/aevatar-registration/status`)
      return {
        registration_id: "aevatar-registration",
        nyx_channel_bot_id: "business-bot",
        status: "active",
      };
    throw new Error(`Unexpected request: ${path}`);
  });
  post.mockImplementation(async (path: string) => {
    if (path === "/keys") {
      keys = [{ ...googleKey, status: "pending_auth", granted_scopes: [] }];
      return keys[0];
    }
    if (path === AEVATAR_CHANNELS_PATH) return registrationReceipt;
    throw new Error(`Unexpected mutation: ${path}`);
  });
});
afterEach(() => {
  cleanup();
  client?.clear();
  vi.unstubAllGlobals();
});

async function toChannel() {
  await screen.findByRole("heading", { name: "Set up channel" });
}

describe("Nyxbot onboarding", () => {
  it.each([
    "/onboarding?step=source",
    "/onboarding?step=source&provider_status=success",
    "/onboarding?provider_status=success",
  ])(
    "automatically advances from %s when Drive and Calendar are already authorized",
    async (path) => {
      window.history.replaceState(null, "", path);
      mount();
      await toChannel();
      expect(
        screen.queryByRole("heading", { name: "Connect a data source" }),
      ).not.toBeInTheDocument();
      expect(post).not.toHaveBeenCalled();
      expect(redirect).not.toHaveBeenCalled();
    },
  );
  it("waits for the pending connection query before advancing, even with a connected key in the list", async () => {
    sessionStorage.setItem(
      "nyxbot-onboarding:owner",
      JSON.stringify({ googleKeyId: "google-1" }),
    );
    const previousGet = get.getMockImplementation()!;
    let resolveAuthorization: ((key: typeof googleKey) => void) | undefined;
    get.mockImplementation((path: string) =>
      path === "/keys/google-1"
        ? new Promise((resolve) => {
            resolveAuthorization = resolve;
          })
        : previousGet(path),
    );
    mount();
    await waitFor(() => expect(resolveAuthorization).toBeTypeOf("function"));
    expect(
      screen.getByRole("heading", { name: "Connect a data source" }),
    ).toBeVisible();
    expect(
      screen.getByRole("button", { name: "Connect Google" }),
    ).toBeDisabled();
    expect(
      screen.queryByRole("heading", { name: "Set up channel" }),
    ).not.toBeInTheDocument();
    await act(async () => resolveAuthorization?.(googleKey));
    await toChannel();
    expect(post).not.toHaveBeenCalled();
  });
  it("automatically advances when a pending connection gains both permissions", async () => {
    keys = [{ ...googleKey, status: "pending_auth", granted_scopes: [] }];
    sessionStorage.setItem(
      "nyxbot-onboarding:owner",
      JSON.stringify({ googleKeyId: "google-1" }),
    );
    mount();
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Connect Google" }),
      ).toBeEnabled(),
    );
    keys = [googleKey];
    await act(async () => {
      await client.invalidateQueries({ queryKey: ["keys"] });
    });
    await toChannel();
    expect(post).not.toHaveBeenCalled();
  });
  it("keeps a provider error on data source with sign-in preserved", async () => {
    keys = [];
    window.history.replaceState(null, "", "/onboarding?provider_status=error");
    mount();
    await screen.findByText(
      /Google authorization was cancelled or could not be completed/,
    );
    expect(screen.getByText("Signed in to NyxID")).toBeVisible();
    expect(
      screen.queryByRole("heading", { name: "Set up channel" }),
    ).not.toBeInTheDocument();
    expect(post).not.toHaveBeenCalled();
  });
  it("keeps a saved channel behind the grant check when data source loading fails", async () => {
    sessionStorage.setItem(
      "nyxbot-onboarding:owner",
      JSON.stringify({ botId: "business-bot", channel: "telegram" }),
    );
    const previousGet = get.getMockImplementation()!;
    get.mockImplementation((path: string) =>
      path === "/keys"
        ? Promise.reject(new Error("Unavailable"))
        : previousGet(path),
    );
    mount();
    await screen.findByText("We couldn't load your connections. Please retry.");
    expect(get).not.toHaveBeenCalledWith("/channel-bots/business-bot");
    expect(
      screen.queryByRole("heading", { name: "Almost there" }),
    ).not.toBeInTheDocument();
  });
  it("starts with all account choices and enters data source only after explicit account continuation", async () => {
    window.history.replaceState(null, "", "/onboarding?channel=telegram");
    keys = [];
    mount();
    await screen.findByRole("heading", { name: "Sign in to NyxID" });
    for (const provider of ["Google", "GitHub", "Apple", "the NyxID app"]) {
      expect(
        screen.getByRole("button", { name: `Continue with ${provider}` }),
      ).toBeVisible();
    }
    expect(get).not.toHaveBeenCalledWith("/keys");
    expect(
      screen.queryByRole("heading", { name: "Connect a data source" }),
    ).not.toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: "Continue as Avery" }),
    );
    await screen.findByRole("heading", { name: "Connect a data source" });
    expect(screen.getByText("Signed in to NyxID")).toBeVisible();
    expect(screen.getByText("Avery")).toBeVisible();
    expect(screen.getByText("Google Drive")).toBeVisible();
    expect(screen.getByText("Google Calendar")).toBeVisible();
    expect(
      screen.getByText(/This permission is separate from signing in to NyxID/),
    ).toBeVisible();
    expect(screen.getByText("Data source").closest("li")).toHaveAttribute(
      "aria-current",
      "step",
    );
    expect(
      screen.queryByText("Google Workspace connected"),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Continue with Google" }),
    ).not.toBeInTheDocument();
    expect(post).not.toHaveBeenCalled();
    expect(redirect).not.toHaveBeenCalled();
  });
  it("returns to account from data source and keeps the account-data-channel order", async () => {
    window.history.replaceState(null, "", "/onboarding");
    mount();
    await userEvent.click(
      screen.getByRole("button", { name: "Continue as Avery" }),
    );
    await toChannel();
    await userEvent.click(screen.getByRole("button", { name: "Back" }));
    await screen.findByRole("heading", { name: "Connect a data source" });
    await act(async () => {
      await client.invalidateQueries({ queryKey: ["keys"] });
    });
    expect(screen.getByText("Google Workspace connected")).toBeVisible();
    expect(
      screen.queryByRole("heading", { name: "Set up channel" }),
    ).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Back" }));
    await screen.findByRole("heading", { name: "Sign in to NyxID" });
    expect(
      screen.getByRole("button", { name: "Continue as Avery" }),
    ).toBeEnabled();
    expect(
      screen.getByRole("button", { name: "Continue with Google" }),
    ).toBeVisible();
    expect(post).not.toHaveBeenCalled();
  });
  it("keeps an unauthenticated return on account and points QR and registration back to data source", async () => {
    auth.isAuthenticated = false;
    mount();
    await screen.findByRole("heading", { name: "Sign in to NyxID" });
    const returnTo = `${window.location.origin}/onboarding?step=source&channel=telegram`;
    expect(
      screen.getByRole("button", { name: "Continue with the NyxID app" }),
    ).toHaveAttribute("data-return-to", returnTo);
    const registration = new URL(
      screen.getByRole("link", { name: "Sign up" }).getAttribute("href")!,
      window.location.origin,
    );
    expect(registration.searchParams.get("return_to")).toBe(returnTo);
    expect(
      screen.queryByRole("button", { name: /Continue as/ }),
    ).not.toBeInTheDocument();
    expect(get).not.toHaveBeenCalledWith("/keys");
  });
  it.each([
    ["google", "Google"],
    ["github", "GitHub"],
    ["apple", "Apple"],
  ])(
    "hands off configured %s login with the onboarding return URL",
    async (id, name) => {
      auth.isAuthenticated = false;
      mount();
      const provider = await screen.findByRole("button", {
        name: `Continue with ${name}`,
      });
      await waitFor(() =>
        expect(provider).toHaveAttribute("aria-disabled", "false"),
      );
      await userEvent.click(provider);
      const target = new URL(
        redirect.mock.calls[0]![0],
        window.location.origin,
      );
      expect(target.pathname).toBe(`/api/v1/auth/social/${id}`);
      expect(target.searchParams.get("return_to")).toBe(
        `${window.location.origin}/onboarding?step=source&channel=telegram`,
      );
      expect(post).not.toHaveBeenCalled();
    },
  );
  it("keeps all four design sign-in methods visible when social providers are unconfigured", async () => {
    auth.isAuthenticated = false;
    publicConfig = { social_providers: [], email_auth_enabled: true };
    mount();
    await screen.findByRole("link", { name: "Sign in with email" });
    expect(
      screen.getByRole("heading", { name: "Sign in to NyxID" }),
    ).toBeVisible();
    expect(screen.getByText("Continue to Nyxbot")).toBeVisible();
    for (const name of ["Google", "GitHub", "Apple", "the NyxID app"]) {
      expect(
        screen.getByRole("button", { name: `Continue with ${name}` }),
      ).toBeVisible();
    }
    expect(screen.queryByRole("progressbar")).not.toBeInTheDocument();
    expect(screen.getByRole("contentinfo")).toHaveTextContent("Back to Nyxbot");
    expect(
      screen.getByRole("button", { name: "Back to Nyxbot" }),
    ).toBeDisabled();
    const google = screen.getByRole("button", { name: "Continue with Google" });
    expect(google).toHaveAttribute("aria-disabled", "true");
    await userEvent.click(google);
    expect(screen.getByRole("status")).toHaveTextContent(
      "Google sign-in is not available in this environment yet",
    );
    expect(redirect).not.toHaveBeenCalled();
    expect(post).not.toHaveBeenCalled();
  });
  it("only redirects enabled sign-in methods while retaining the other rows", async () => {
    auth.isAuthenticated = false;
    publicConfig.social_providers = ["github"];
    mount();
    const github = screen.getByRole("button", { name: "Continue with GitHub" });
    await waitFor(() =>
      expect(github).toHaveAttribute("aria-disabled", "false"),
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Continue with Apple" }),
    );
    expect(redirect).not.toHaveBeenCalled();
    await userEvent.click(github);
    expect(redirect).toHaveBeenCalledTimes(1);
    expect(redirect.mock.calls[0]![0]).toContain("/api/v1/auth/social/github?");
  });
  it("preselects a referral, registers Telegram once, and never fabricates a pairing code", async () => {
    mount();
    await toChannel();
    expect(screen.getByRole("radio", { name: /Telegram/ })).toBeChecked();
    const submit = screen.getByRole("button", { name: "Connect channel" });
    expect(submit).toBeDisabled();
    const token = screen.getByLabelText("Bot token", { exact: true });
    expect(token).toHaveAttribute("type", "password");
    await userEvent.type(token, `  ${telegramToken}  `);
    await userEvent.click(submit);
    await screen.findByRole("heading", { name: "Almost there" });
    expect(post).toHaveBeenCalledWith(AEVATAR_CHANNELS_PATH, {
      platform: "telegram",
      label: "My_Shop_Bot_nyxid_bot",
      bot_token: telegramToken,
      webhook_base_url: AEVATAR_WEBHOOK_BASE_URL,
    });
    expect(post).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("button", { name: "Copy code" })).toBeDisabled();
    expect(
      screen.getByRole("button", { name: "Open Telegram" }),
    ).toBeDisabled();
    expect(screen.getByPlaceholderText("Not available yet")).toHaveValue("");
    expect(sessionStorage.getItem("nyxbot-onboarding:owner")).not.toContain(
      telegramToken,
    );
    expect(
      JSON.parse(sessionStorage.getItem("nyxbot-onboarding:owner")!),
    ).toMatchObject({
      botId: "business-bot",
      registrationId: "aevatar-registration",
    });
    expect(get).toHaveBeenCalledWith(
      `${AEVATAR_CHANNELS_PATH}/aevatar-registration/status`,
    );
    expect(telegram.mock.invocationCallOrder[0]).toBeLessThan(
      post.mock.invocationCallOrder[0]!,
    );
    expect(
      await screen.findByRole("link", { name: "Manage channel" }),
    ).toHaveAttribute("href", `${AEVATAR_WEBHOOK_BASE_URL}/channels`);
  });
  it.each(["en", "zh-CN"])(
    "blocks invalid tokens, shows localized feedback and accepts correction in %s",
    async (language) => {
      await nyxbotI18n.changeLanguage(language);
      const t = nyxbotI18n.t.bind(nyxbotI18n);
      mount();
      await screen.findByRole("heading", { name: t("channelTitle") });
      const token = screen.getByLabelText(t("token"), { exact: true });
      const submit = screen.getByRole("button", { name: t("connectChannel") });
      expect(submit).toBeDisabled();

      await userEvent.type(token, "not-a-bot-token");
      expect(await screen.findByRole("alert")).toHaveTextContent(
        t("tokenInvalid"),
      );
      expect(token).toHaveAttribute("aria-invalid", "true");
      expect(submit).toBeDisabled();
      await userEvent.type(token, "{Enter}");
      // The resolver also guards form submission independently of the disabled button.
      fireEvent.submit(token.closest("form")!);
      await waitFor(() => expect(submit).toBeDisabled());
      expect(post).not.toHaveBeenCalled();
      expect(telegram).not.toHaveBeenCalled();

      await userEvent.clear(token);
      await waitFor(() =>
        expect(screen.getByRole("alert")).toHaveTextContent(t("tokenRequired")),
      );
      await userEvent.type(token, telegramToken);
      await waitFor(() => expect(submit).toBeEnabled());
      expect(screen.queryByRole("alert")).not.toBeInTheDocument();
      expect(post).not.toHaveBeenCalled();
    },
  );
  it("waits for server verification and prevents duplicate submissions", async () => {
    let resolveRegistration: ((value: object) => void) | undefined;
    post.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveRegistration = resolve;
        }),
    );
    mount();
    await toChannel();
    const token = screen.getByLabelText("Bot token", { exact: true });
    const submit = screen.getByRole("button", { name: "Connect channel" });
    await userEvent.type(token, telegramToken);
    await userEvent.click(submit);
    await waitFor(() => expect(post).toHaveBeenCalledTimes(1));
    expect(submit).toBeDisabled();
    expect(token).toBeDisabled();
    expect(screen.getByRole("button", { name: "Back" })).toBeDisabled();
    expect(
      screen.getByRole("heading", { name: "Set up channel" }),
    ).toBeVisible();
    await userEvent.click(submit);
    await act(async () => {
      fireEvent.submit(token.closest("form")!);
      fireEvent.submit(token.closest("form")!);
    });
    expect(post).toHaveBeenCalledTimes(1);
    await act(async () => resolveRegistration?.(registrationReceipt));
    await screen.findByRole("heading", { name: "Almost there" });
  });
  it("offers first-time Aevatar consent and allows retry with the same bot token", async () => {
    authorizer.mockRejectedValueOnce(
      new AevatarAuthError("channelConsentRequired"),
    );
    mount();
    await toChannel();
    await userEvent.type(
      screen.getByLabelText("Bot token", { exact: true }),
      telegramToken,
    );
    const submit = screen.getByRole("button", { name: "Connect channel" });
    await userEvent.click(submit);
    expect(
      await screen.findByRole("link", { name: "Authorize Aevatar" }),
    ).toHaveAttribute(
      "href",
      "https://aevatar-console-backend-api.aevatar.ai/channels",
    );
    expect(post).not.toHaveBeenCalled();
    expect(submit).toBeEnabled();
    expect(auth.isAuthenticated).toBe(true);
    await userEvent.click(submit);
    await screen.findByRole("heading", { name: "Almost there" });
    expect(post).toHaveBeenCalledTimes(1);
  });
  it("leaves the channel unselected for a direct visitor and ignores the disabled WhatsApp option", async () => {
    window.history.replaceState(null, "", "/onboarding");
    mount();
    await userEvent.click(
      screen.getByRole("button", { name: "Continue as Avery" }),
    );
    await toChannel();
    expect(screen.getByRole("radio", { name: /Telegram/ })).not.toBeChecked();
    expect(screen.getByRole("radio", { name: /WhatsApp/ })).not.toBeChecked();
    await userEvent.click(screen.getByRole("radio", { name: /Telegram/ }));
    await userEvent.type(
      screen.getByLabelText("Bot token", { exact: true }),
      "unsent-token",
    );
    await userEvent.click(screen.getByRole("radio", { name: /WhatsApp/ }));
    expect(screen.getByRole("radio", { name: /Telegram/ })).toBeChecked();
    expect(screen.getByRole("radio", { name: /WhatsApp/ })).not.toBeChecked();
    expect(screen.getByLabelText("Bot token", { exact: true })).toHaveValue(
      "unsent-token",
    );
  });
  it("preserves Google authorization when token verification fails and redacts provider errors", async () => {
    post.mockRejectedValueOnce(new Error(`Bad token: ${telegramToken}`));
    mount();
    await toChannel();
    await userEvent.type(
      screen.getByLabelText("Bot token", { exact: true }),
      telegramToken,
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Connect channel" }),
    );
    await screen.findByText(
      "Your bot was verified, but channel registration couldn't be confirmed. Check Channels in Aevatar before retrying.",
    );
    expect(
      screen.queryByText(`Bad token: ${telegramToken}`),
    ).not.toBeInTheDocument();
    expect(post).toHaveBeenCalledTimes(1);
    expect(
      screen.getByRole("heading", { name: "Set up channel" }),
    ).toBeVisible();
    await userEvent.click(screen.getByRole("button", { name: "Back" }));
    await screen.findByText("Google Workspace connected");
  });
  it("stops at the token field when Telegram rejects the token, without calling Aevatar", async () => {
    telegram.mockResolvedValue({ ok: false, status: 401 });
    mount();
    await toChannel();
    await userEvent.type(
      screen.getByLabelText("Bot token", { exact: true }),
      telegramToken,
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Connect channel" }),
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Telegram rejected this token.",
    );
    expect(post).not.toHaveBeenCalled();
    expect(
      screen.getByRole("heading", { name: "Set up channel" }),
    ).toBeVisible();
    expect(
      sessionStorage.getItem("nyxbot-onboarding:owner") ?? "",
    ).not.toContain(telegramToken);
  });
  it("waits for Aevatar status after acceptance and retries only status reads", async () => {
    let ready = false;
    const previousGet = get.getMockImplementation()!;
    get.mockImplementation((path: string) =>
      path === `${AEVATAR_CHANNELS_PATH}/aevatar-registration/status`
        ? Promise.resolve({
            ...registrationReceipt,
            status: ready ? "active" : "pending_webhook",
          })
        : previousGet(path),
    );
    mount();
    await toChannel();
    await userEvent.type(
      screen.getByLabelText("Bot token", { exact: true }),
      telegramToken,
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Connect channel" }),
    );
    await screen.findByText(
      "Your channel registration was accepted. We're still waiting for it to be ready. Retry to check its status.",
    );
    expect(
      screen.queryByText(/^Your channel is saved/),
    ).not.toBeInTheDocument();
    ready = true;
    await userEvent.click(screen.getByRole("button", { name: "Retry" }));
    await screen.findByText(/^Your channel is saved/);
    expect(post).toHaveBeenCalledTimes(1);
    expect(telegram).toHaveBeenCalledTimes(1);
  });
  it.each(["referral", "saved"])(
    "disables WhatsApp setup even with a %s preselection",
    async (entry) => {
      if (entry === "referral") {
        window.history.replaceState(
          null,
          "",
          "/onboarding?step=source&channel=whatsapp",
        );
      } else {
        sessionStorage.setItem(
          "nyxbot-onboarding:owner",
          JSON.stringify({ channel: "whatsapp" }),
        );
      }
      mount();
      await toChannel();
      const whatsapp = screen.getByRole("radio", { name: /WhatsApp/ });
      expect(whatsapp).toBeDisabled();
      expect(whatsapp).not.toBeChecked();
      expect(whatsapp.closest("label")).toHaveTextContent("coming soon");
      await userEvent.click(whatsapp);
      expect(
        screen.queryByRole("button", { name: "Connect WhatsApp" }),
      ).not.toBeInTheDocument();
      expect(
        screen.queryByRole("button", { name: "Review spending cap" }),
      ).not.toBeInTheDocument();
      expect(
        screen.getByRole("button", { name: "Connect channel" }),
      ).toBeDisabled();
      expect(get).not.toHaveBeenCalledWith(
        "/channel-bots/managed-onboarding/whatsapp",
      );
      await userEvent.click(screen.getByRole("radio", { name: /Telegram/ }));
      expect(screen.getByLabelText("Bot token", { exact: true })).toBeVisible();
      expect(post).not.toHaveBeenCalled();
    },
  );
  it.each(["status", "provider_status"])(
    "checks real scopes after OAuth instead of trusting %s=success",
    async (parameter) => {
      keys = [{ ...googleKey, granted_scopes: ["openid", "email", "profile"] }];
      window.history.replaceState(null, "", `/onboarding?${parameter}=success`);
      mount();
      const button = await screen.findByRole("button", {
        name: "Connect Google",
      });
      await waitFor(() => expect(button).toBeEnabled());
      expect(
        screen.queryByText("Google Workspace connected"),
      ).not.toBeInTheDocument();
      expect(
        screen.queryByRole("heading", { name: "Set up channel" }),
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
      expect(url.searchParams.get("redirect_path")).toBe(
        "/onboarding?step=source",
      );
      expect(
        JSON.parse(sessionStorage.getItem("nyxbot-onboarding:owner")!)
          .googleKeyId,
      ).toBe("google-1");
    },
  );
  it("requests Drive and Calendar consent on click without using catalog configuration as a grant prerequisite", async () => {
    keys = [];
    catalogResponse = {
      ...catalog,
      credential_mode: "user",
      has_platform_oauth_credentials: false,
      platform_scope_allowlist: [
        "openid",
        "email",
        "profile",
      ] as unknown as typeof GOOGLE_WORKSPACE_SCOPES,
    };
    mount();
    const button = screen.getByRole("button", { name: "Connect Google" });
    await waitFor(() => expect(button).toBeEnabled());
    expect(post).not.toHaveBeenCalled();
    await userEvent.click(button);
    await waitFor(() => expect(redirect).toHaveBeenCalledTimes(1));
    const oauthCall = get.mock.calls.find(([path]) =>
      String(path).startsWith("/providers/google-provider/connect/oauth"),
    );
    const url = new URL(oauthCall![0], "http://localhost");
    expect(url.searchParams.get("scope_override")?.split(",")).toEqual([
      ...GOOGLE_WORKSPACE_SCOPES,
    ]);
    expect(redirect).toHaveBeenCalledWith(
      "https://accounts.google.com/o/oauth2/v2/auth?state=test",
    );
  });
  it("shows an initiation rejection and retries the same connection without claiming user cancellation or consent", async () => {
    keys = [];
    const message =
      "Requested scopes are not enabled for the shared Google OAuth app.";
    const previousGet = get.getMockImplementation()!;
    let attempts = 0;
    let rejectInitiation: (() => void) | undefined;
    get.mockImplementation((path: string) => {
      if (
        path.startsWith("/providers/google-provider/connect/oauth") &&
        attempts++ === 0
      ) {
        return new Promise((_, reject) => {
          rejectInitiation = () =>
            reject(
              new ApiError(400, {
                error: "validation_error",
                error_code: 1008,
                message,
              }),
            );
        });
      }
      return previousGet(path);
    });
    mount();
    const button = screen.getByRole("button", { name: "Connect Google" });
    await waitFor(() => expect(button).toBeEnabled());
    await userEvent.click(button);
    await waitFor(() => expect(rejectInitiation).toBeTypeOf("function"));
    expect(button).toBeDisabled();
    await userEvent.click(button);
    expect(attempts).toBe(1);
    await act(async () => rejectInitiation?.());
    await screen.findByText(message);
    expect(screen.getByRole("alert")).toHaveTextContent(
      "We couldn't open Google authorization.",
    );
    expect(screen.getByText("Signed in to NyxID")).toBeVisible();
    expect(
      screen.queryByText(/authorization was cancelled/),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByText("Google Workspace connected"),
    ).not.toBeInTheDocument();
    expect(redirect).not.toHaveBeenCalled();
    await waitFor(() => expect(button).toBeEnabled());
    await userEvent.click(button);
    await waitFor(() => expect(redirect).toHaveBeenCalledTimes(1));
    expect(post).toHaveBeenCalledTimes(1);
  });
  it("requires a Google OAuth provider route before starting consent", async () => {
    keys = [];
    catalogResponse = { ...catalog, provider_config_id: "" };
    mount();
    await screen.findByText(
      /Google Drive and Calendar authorization is not available/,
    );
    expect(
      screen.getByRole("button", { name: "Connect Google" }),
    ).toBeDisabled();
    expect(post).not.toHaveBeenCalled();
  });
  it("confirms the account and verifies grants before automatically resuming a saved channel", async () => {
    window.history.replaceState(null, "", "/onboarding");
    sessionStorage.setItem(
      "nyxbot-onboarding:owner",
      JSON.stringify({ botId: "business-bot", channel: "telegram" }),
    );
    mount();
    await screen.findByRole("heading", { name: "Sign in to NyxID" });
    expect(get).not.toHaveBeenCalledWith("/channel-bots/business-bot");
    await userEvent.click(
      screen.getByRole("button", { name: "Continue as Avery" }),
    );
    await screen.findByText(/Your channel is saved/);
    expect(get).toHaveBeenCalledWith("/channel-bots/business-bot");
    expect(post).not.toHaveBeenCalled();
  });
  it("opens contextual help and switches the feature's language", async () => {
    mount();
    await toChannel();
    await userEvent.click(
      screen.getByRole("button", { name: "Why this step?" }),
    );
    await screen.findByText(/Connect your business's customer-facing channel/);
    fireEvent.change(screen.getByLabelText("Language"), {
      target: { value: "zh-CN" },
    });
    await screen.findByRole("heading", { name: "设置渠道" });
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
    await userEvent.click(
      screen.getByRole("button", { name: "Continue", exact: true }),
    );
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
    await toChannel();
    expect(
      JSON.parse(sessionStorage.getItem("nyxbot-onboarding:owner")!)
        .googleKeyId,
    ).toBeNull();
  });
});
