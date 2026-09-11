import {
  focusManager,
  QueryClient,
  QueryClientProvider,
} from "@tanstack/react-query";
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  RouterProvider,
} from "@tanstack/react-router";
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
import {
  GOOGLE_WORKSPACE_SCOPES,
  nyxbotSearchSchema,
} from "@/schemas/nyxbot-onboarding";
import { ApiError } from "@/lib/api-client";
import type { UserServiceResponse } from "@/schemas/keys";
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
    isLoading: false,
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
let userServices: Pick<
  UserServiceResponse,
  "id" | "slug" | "is_active" | "credential_source"
>[];
let client: QueryClient;
function makeRouter() {
  const root = createRootRoute();
  const onboarding = createRoute({
    getParentRoute: () => root,
    path: "/onboarding",
    validateSearch: (search) => nyxbotSearchSchema.parse(search),
    component: NyxbotOnboardingPage,
  });
  return createRouter({
    routeTree: root.addChildren([onboarding]),
    history: createMemoryHistory({
      initialEntries: [window.location.pathname + window.location.search],
    }),
    defaultPendingMinMs: 0,
  });
}
let router: ReturnType<typeof makeRouter>;
async function mount() {
  client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  router = makeRouter();
  await router.load();
  const page = render(
    <QueryClientProvider client={client}>
      <RouterProvider router={router} />
    </QueryClientProvider>,
  );
  await waitFor(() => expect(router.state.status).toBe("idle"));
  return page;
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
      result: {
        id: 123456,
        is_bot: true,
        first_name: "My Shop Bot",
        username: "my_shop_bot",
      },
    }),
  });
  sessionStorage.clear();
  window.history.replaceState(
    null,
    "",
    "/onboarding?step=channel&channel=telegram",
  );
  auth.isAuthenticated = true;
  auth.isLoading = false;
  keys = [googleKey];
  catalogResponse = catalog;
  publicConfig = {
    social_providers: ["google", "github", "apple"],
    email_auth_enabled: false,
  };
  userServices = [
    {
      id: "workspace-service",
      slug: "api-google-workspace",
      is_active: true,
      credential_source: { type: "personal" },
    },
    {
      id: "ornn-service",
      slug: "ornn-api",
      is_active: true,
      credential_source: {
        type: "org",
        allowed: true,
        org_id: "org-1",
        org_name: "Shared",
        role: "member",
      },
    },
    {
      id: "llm-service",
      slug: "chrono-llm-public",
      is_active: true,
      credential_source: { type: "personal" },
    },
  ];
  await nyxbotI18n.changeLanguage("en");
  get.mockImplementation(async (path: string) => {
    if (path === "/keys") return { keys };
    if (path === "/keys/google-1") return keys[0];
    if (path === "/catalog/api-google") return catalogResponse;
    if (path === "/public/config") return publicConfig;
    if (path === "/user-services") return { services: userServices };
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
  router?.history.destroy();
  client?.clear();
  focusManager.setFocused(undefined);
  vi.unstubAllGlobals();
});

async function toChannel() {
  await screen.findByRole("heading", { name: "Connect a customer channel" });
}

function expectNoGoogleRequests() {
  expect(
    get.mock.calls.filter(([path]) =>
      /^\/(keys(?:\/|$)|catalog\/api-google$)/.test(String(path)),
    ),
  ).toEqual([]);
}

describe("Nyxbot onboarding", () => {
  it.each([
    "/onboarding?channel=telegram",
    "/onboarding?step=unknown&channel=telegram",
  ])(
    "normalizes %s to the account URL without dropping the referral",
    async (path) => {
      window.history.replaceState(null, "", path);
      await mount();
      await screen.findByRole("heading", { name: "Sign in to NyxID" });
      expect(router.state.location.search).toEqual({
        step: "account",
        channel: "telegram",
      });
      expect(router.history.length).toBe(1);
      expect(get).not.toHaveBeenCalledWith("/keys");
    },
  );
  it.each([
    ["account", "Sign in to NyxID"],
    ["source", "Connect a data source"],
    ["channel", "Connect a customer channel"],
  ] as const)(
    "opens and reloads the distinct %s URL",
    async (step, heading) => {
      window.history.replaceState(
        null,
        "",
        `/onboarding?step=${step}&channel=telegram`,
      );
      sessionStorage.setItem(
        "nyxbot-onboarding:owner",
        JSON.stringify({
          channel: "telegram",
        }),
      );
      const first = await mount();
      await screen.findByRole("heading", { name: heading });
      expect(router.state.location.search.step).toBe(step);
      first.unmount();
      client.clear();
      router.history.destroy();
      await mount();
      await screen.findByRole("heading", { name: heading });
      expect(router.state.location.search.step).toBe(step);
      expect(post).not.toHaveBeenCalled();
    },
  );
  it("keeps browser Back/Forward in sync with the visible step and grants", async () => {
    window.history.replaceState(
      null,
      "",
      "/onboarding?step=source&channel=telegram",
    );
    await mount();
    await userEvent.click(
      await screen.findByRole("button", { name: "Continue", exact: true }),
    );
    await waitFor(() => expect(redirect).toHaveBeenCalledTimes(1));
    expect(router.state.location.search.step).toBe("source");
    await router.navigate({
      to: "/onboarding",
      search: { step: "channel", channel: "telegram" },
    });
    await toChannel();
    expect(router.state.location.search).toEqual({
      step: "channel",
      channel: "telegram",
    });
    await act(async () => router.history.back());
    await screen.findByRole("heading", { name: "Connect a data source" });
    expect(router.state.location.search.step).toBe("source");
    await act(async () => {
      await client.invalidateQueries({ queryKey: ["keys"] });
    });
    expect(
      screen.getByRole("heading", { name: "Connect a data source" }),
    ).toBeVisible();
    await act(async () => router.history.forward());
    await toChannel();
    expect(router.state.location.search.step).toBe("channel");
    await userEvent.click(screen.getByRole("button", { name: "Back" }));
    await screen.findByRole("heading", { name: "Connect a data source" });
    expect(router.state.location.search.step).toBe("source");
    expect(post).toHaveBeenCalledWith(
      "/keys",
      expect.objectContaining({ service_slug: "api-google" }),
    );
  });
  it.each(["source", "channel", "success", "link"])(
    "guards an unauthenticated %s URL",
    async (step) => {
      auth.isAuthenticated = false;
      window.history.replaceState(
        null,
        "",
        `/onboarding?step=${step}&channel=telegram`,
      );
      await mount();
      await screen.findByRole("heading", { name: "Sign in to NyxID" });
      expect(router.state.location.search).toEqual({
        step: "account",
        channel: "telegram",
      });
      expect(router.history.length).toBe(1);
      expect(get).not.toHaveBeenCalledWith("/keys");
      expect(post).not.toHaveBeenCalled();
    },
  );
  it.each(["channel"])(
    "keeps %s independent of Google requests, including saved pending keys and reload/focus",
    async (step) => {
      keys = [];
      window.history.replaceState(
        null,
        "",
        `/onboarding?step=${step}&channel=telegram`,
      );
      sessionStorage.setItem(
        "nyxbot-onboarding:owner",
        JSON.stringify({
          googleKeyId: "google-1",
        }),
      );
      const first = await mount();
      const heading = "Connect a customer channel";
      await screen.findByRole("heading", { name: heading });
      expectNoGoogleRequests();
      first.unmount();
      client.clear();
      router.history.destroy();
      await mount();
      await screen.findByRole("heading", { name: heading });
      await act(async () => {
        focusManager.setFocused(false);
        focusManager.setFocused(true);
        await client.invalidateQueries();
      });
      expect(router.state.location.search.step).toBe(step);
      expect(router.history.length).toBe(1);
      expectNoGoogleRequests();
      expect(post).not.toHaveBeenCalled();
      expect(telegram).not.toHaveBeenCalled();
      if (step === "channel") {
        expect(
          get.mock.calls.every(([path]) => path === "/user-services"),
        ).toBe(true);
        expect(authorizer).not.toHaveBeenCalled();
      }
    },
  );
  it("unmounts data-source queries when continuing to channel and remounts them only on Back", async () => {
    window.history.replaceState(null, "", "/onboarding?step=source");
    sessionStorage.setItem(
      "nyxbot-onboarding:owner",
      JSON.stringify({ googleKeyId: "google-1", channel: "telegram" }),
    );
    await mount();
    await screen.findByText("Google Workspace connected");
    await waitFor(() => expect(client.isFetching()).toBe(0));
    for (const path of ["/keys", "/catalog/api-google", "/keys/google-1"])
      expect(get).toHaveBeenCalledWith(path);
    await userEvent.click(
      screen.getByRole("button", { name: "Continue", exact: true }),
    );
    await waitFor(() => expect(redirect).toHaveBeenCalledTimes(1));
    expect(router.state.location.search.step).toBe("source");
    await router.navigate({
      to: "/onboarding",
      search: { step: "channel", channel: "telegram" },
    });
    await toChannel();
    get.mockClear();
    await act(async () => {
      focusManager.setFocused(false);
      focusManager.setFocused(true);
      await client.invalidateQueries();
    });
    expect(get.mock.calls.every(([path]) => path === "/user-services")).toBe(
      true,
    );
    expect(post).not.toHaveBeenCalled();
    expect(telegram).not.toHaveBeenCalled();
    expect(authorizer).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: "Back" }));
    await screen.findByText("Google Workspace connected");
    for (const path of ["/keys", "/catalog/api-google", "/keys/google-1"])
      expect(get).toHaveBeenCalledWith(path);
    expect(router.state.location.search.step).toBe("source");
  });
  it("replaces a link URL with channel when no registered bot exists", async () => {
    window.history.replaceState(
      null,
      "",
      "/onboarding?step=link&botId=forged&completed=true",
    );
    await mount();
    await toChannel();
    expect(router.state.location.search).toEqual({ step: "channel" });
    expect(router.history.length).toBe(1);
    expect(get).not.toHaveBeenCalledWith("/channel-bots/forged");
    expect(post).not.toHaveBeenCalled();
  });
  it("returns from link to channel and browser Back restores link without another registration", async () => {
    window.history.replaceState(null, "", "/onboarding?step=link");
    sessionStorage.setItem("nyxbot-onboarding:owner", JSON.stringify({}));
    await mount();
    await screen.findByRole("heading", { name: "Connect a customer channel" });
    expect(router.state.location.search.step).toBe("channel");
    expect(router.history.length).toBe(1);
    expect(post).not.toHaveBeenCalled();
  });
  it.each([true, false])(
    "resolves a restored session before showing the account step (authenticated: %s)",
    async (authenticated) => {
      auth.isAuthenticated = false;
      auth.isLoading = true;
      const page = await mount();
      expect(screen.getByRole("status")).toHaveTextContent(
        "Checking your account and connected services.",
      );
      expect(
        screen.queryByRole("heading", { name: "Sign in to NyxID" }),
      ).not.toBeInTheDocument();
      expect(get).not.toHaveBeenCalledWith("/keys");
      auth.isAuthenticated = authenticated;
      auth.isLoading = false;
      page.rerender(
        <QueryClientProvider client={client}>
          <RouterProvider key={String(authenticated)} router={router} />
        </QueryClientProvider>,
      );
      await screen.findByRole("heading", {
        name: authenticated ? "Connect a customer channel" : "Sign in to NyxID",
      });
    },
  );
  it.each(["/keys", "/catalog/api-google"])(
    "does not render a provisional step while %s is loading",
    async (path) => {
      window.history.replaceState(null, "", "/onboarding?step=source");
      const previousGet = get.getMockImplementation()!;
      let resolveRequest: (() => void) | undefined;
      get.mockImplementation((requested: string) =>
        requested === path
          ? new Promise((resolve) => {
              resolveRequest = () => resolve(previousGet(requested));
            })
          : previousGet(requested),
      );
      await mount();
      await waitFor(() => expect(resolveRequest).toBeTypeOf("function"));
      expect(screen.getByRole("status")).toHaveTextContent(
        "Checking your account and connected services.",
      );
      expect(
        screen.queryByRole("heading", { name: "Connect a data source" }),
      ).not.toBeInTheDocument();
      expect(
        screen.queryByRole("heading", { name: "Connect a customer channel" }),
      ).not.toBeInTheDocument();
      expect(
        screen.queryByRole("button", { name: "Connect Google" }),
      ).not.toBeInTheDocument();
      await act(async () => resolveRequest?.());
      await screen.findByRole("heading", { name: "Connect a data source" });
      expect(screen.getByText("Account").closest("li")).toHaveAttribute(
        "data-state",
        "complete",
      );
      expect(screen.getByText("Data source").closest("li")).toHaveAttribute(
        "aria-current",
        "step",
      );
      expect(post).not.toHaveBeenCalled();
    },
  );
  it.each([
    "/onboarding?step=channel&provider_status=success",
    "/onboarding?step=source&provider_status=success",
    "/onboarding?provider_status=success",
  ])(
    "automatically advances from %s when Drive and Calendar are already authorized",
    async (path) => {
      window.history.replaceState(null, "", path);
      await mount();
      await toChannel();
      expect(router.state.location.search).toEqual({ step: "channel" });
      expect(
        screen.queryByRole("heading", { name: "Connect a data source" }),
      ).not.toBeInTheDocument();
      expect(post).not.toHaveBeenCalled();
      expect(redirect).not.toHaveBeenCalled();
    },
  );
  it("waits for the pending connection query before advancing, even with a connected key in the list", async () => {
    window.history.replaceState(
      null,
      "",
      "/onboarding?step=source&provider_status=success",
    );
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
    await mount();
    await waitFor(() => expect(resolveAuthorization).toBeTypeOf("function"));
    expect(
      screen.queryByRole("heading", { name: "Connect a data source" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Connect Google" }),
    ).not.toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent(
      "Checking your account and connected services.",
    );
    expect(
      screen.queryByRole("heading", { name: "Connect a customer channel" }),
    ).not.toBeInTheDocument();
    await act(async () => resolveAuthorization?.(googleKey));
    await toChannel();
    expect(post).not.toHaveBeenCalled();
  });
  it("keeps the explicit source URL after permissions become effective until Continue", async () => {
    window.history.replaceState(null, "", "/onboarding?step=source");
    keys = [{ ...googleKey, status: "pending_auth", granted_scopes: [] }];
    sessionStorage.setItem(
      "nyxbot-onboarding:owner",
      JSON.stringify({ googleKeyId: "google-1" }),
    );
    await mount();
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Connect Google" }),
      ).toBeEnabled(),
    );
    keys = [googleKey];
    await act(async () => {
      await client.invalidateQueries({ queryKey: ["keys"] });
    });
    await screen.findByText("Google Workspace connected");
    expect(router.state.location.search.step).toBe("source");
    await userEvent.click(
      screen.getByRole("button", { name: "Continue", exact: true }),
    );
    await waitFor(() => expect(redirect).toHaveBeenCalledTimes(1));
    expect(router.state.location.search.step).toBe("source");
    expect(post).not.toHaveBeenCalled();
  });
  it("keeps a provider error on data source with sign-in preserved", async () => {
    keys = [];
    window.history.replaceState(null, "", "/onboarding?provider_status=error");
    await mount();
    await screen.findByText(
      /Google authorization was cancelled or could not be completed/,
    );
    expect(screen.getByText("Signed in to NyxID")).toBeVisible();
    expect(
      screen.queryByRole("heading", { name: "Connect a customer channel" }),
    ).not.toBeInTheDocument();
    expect(post).not.toHaveBeenCalled();
  });
  it("keeps data source retryable when its grant check fails, without loading a saved channel", async () => {
    window.history.replaceState(null, "", "/onboarding?step=source");
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
    await mount();
    await screen.findByText("We couldn't load your connections. Please retry.");
    expect(get).not.toHaveBeenCalledWith("/channel-bots/business-bot");
    expect(
      screen.queryByRole("heading", { name: "Almost there" }),
    ).not.toBeInTheDocument();
  });
  it("starts with all account choices and enters data source only after explicit account continuation", async () => {
    window.history.replaceState(null, "", "/onboarding?channel=telegram");
    keys = [];
    await mount();
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
    await mount();
    await userEvent.click(
      screen.getByRole("button", { name: "Continue as Avery" }),
    );
    await userEvent.click(
      await screen.findByRole("button", { name: "Continue", exact: true }),
    );
    await waitFor(() => expect(redirect).toHaveBeenCalledTimes(1));
    expect(router.state.location.search.step).toBe("source");
    await router.navigate({
      to: "/onboarding",
      search: { step: "channel", channel: "telegram" },
    });
    await toChannel();
    await userEvent.click(screen.getByRole("button", { name: "Back" }));
    await screen.findByRole("heading", { name: "Connect a data source" });
    expect(
      screen.queryByRole("heading", { name: "Connect a customer channel" }),
    ).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Back" }));
    await screen.findByRole("heading", { name: "Sign in to NyxID" });
    expect(
      screen.getByRole("button", { name: "Continue as Avery" }),
    ).toBeEnabled();
    expect(
      screen.getByRole("button", { name: "Continue with Google" }),
    ).toBeVisible();
    expect(post).toHaveBeenCalledWith(
      "/keys",
      expect.objectContaining({ service_slug: "api-google" }),
    );
  });
  it("keeps an unauthenticated return on account and points QR and registration back to data source", async () => {
    auth.isAuthenticated = false;
    await mount();
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
      await mount();
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
    await mount();
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
    await mount();
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
    await mount();
    await toChannel();
    expect(get).toHaveBeenCalledWith("/user-services");
    expect(authorizer).not.toHaveBeenCalled();
    expect(telegram).not.toHaveBeenCalled();
    expect(screen.getByRole("radio", { name: /Telegram/ })).toBeChecked();
    const submit = screen.getByRole("button", { name: "Connect channel" });
    expect(submit).toBeDisabled();
    const token = screen.getByLabelText("Customer bot token", { exact: true });
    expect(token).toHaveAttribute("type", "password");
    await userEvent.type(token, `  ${telegramToken}  `);
    await userEvent.click(submit);
    await screen.findByRole("heading", { name: "Channel connected" });
    expect(router.state.location.search.step).toBe("success");
    expect(
      screen.queryByLabelText("Customer bot token", { exact: true }),
    ).not.toBeInTheDocument();
    expect(screen.queryByText("Link chat")).not.toBeInTheDocument();
    const steps = screen.getByRole("navigation", { name: "Nyxbot onboarding" });
    expect(steps.querySelectorAll("li")).toHaveLength(3);
    expect(steps.querySelectorAll('[data-state="complete"]')).toHaveLength(3);
    expect(post).toHaveBeenCalledWith(AEVATAR_CHANNELS_PATH, {
      platform: "telegram",
      label: "My_Shop_Bot_nyxid_bot",
      bot_token: telegramToken,
      webhook_base_url: AEVATAR_WEBHOOK_BASE_URL,
      service_ids: ["workspace-service", "ornn-service", "llm-service"],
    });
    expect(post).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("link", { name: "Open Telegram" })).toHaveAttribute(
      "href",
      "https://t.me/my_shop_bot",
    );
    expect(sessionStorage.getItem("nyxbot-onboarding:owner")).not.toContain(
      telegramToken,
    );
    expect(
      JSON.parse(sessionStorage.getItem("nyxbot-onboarding:owner")!),
    ).toMatchObject({
      botId: "business-bot",
      registrationId: "aevatar-registration",
    });
    expect(telegram.mock.invocationCallOrder[0]).toBeLessThan(
      post.mock.invocationCallOrder[0]!,
    );
  });
  it("can return to Channel and connect a second bot with its own result link", async () => {
    await mount();
    await toChannel();
    await userEvent.type(
      screen.getByLabelText("Customer bot token", { exact: true }),
      telegramToken,
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Connect channel" }),
    );
    await screen.findByRole("heading", { name: "Channel connected" });
    await act(async () => router.history.back());
    await toChannel();
    expect(
      screen.queryByRole("link", { name: "Open Telegram" }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByLabelText("Customer bot token", { exact: true }),
    ).toHaveValue("");
    await act(async () => router.history.forward());
    expect(
      await screen.findByRole("link", { name: "Open Telegram" }),
    ).toHaveAttribute("href", "https://t.me/my_shop_bot");
    expect(post).toHaveBeenCalledTimes(1);
    await userEvent.click(
      screen.getByRole("button", { name: "Connect another bot" }),
    );
    await toChannel();
    telegram.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: async () => ({
        ok: true,
        result: {
          id: 654321,
          is_bot: true,
          first_name: "Another Shop",
          username: "another_shop_bot",
        },
      }),
    });
    post.mockResolvedValueOnce({
      ...registrationReceipt,
      registration_id: "second-registration",
      nyx_channel_bot_id: "second-bot",
    });
    const secondToken = `654321:${"zY_8-".repeat(7)}`;
    await userEvent.type(
      screen.getByLabelText("Customer bot token", { exact: true }),
      secondToken,
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Connect channel" }),
    );
    await screen.findByRole("heading", { name: "Channel connected" });
    expect(router.state.location.search.step).toBe("success");
    expect(screen.getByRole("link", { name: "Open Telegram" })).toHaveAttribute(
      "href",
      "https://t.me/another_shop_bot",
    );
    expect(screen.queryByText("@my_shop_bot")).not.toBeInTheDocument();
    expect(post).toHaveBeenCalledTimes(2);
    expect(post).toHaveBeenLastCalledWith(
      AEVATAR_CHANNELS_PATH,
      expect.objectContaining({
        bot_token: secondToken,
        label: "Another_Shop_nyxid_bot",
        service_ids: ["workspace-service", "ornn-service", "llm-service"],
      }),
    );
    expect(sessionStorage.getItem("nyxbot-onboarding:owner")).not.toContain(
      secondToken,
    );
  });
  it.each(["en", "zh-CN"])(
    "restores the separate result URL in %s without submitting or fetching services",
    async (language) => {
      await nyxbotI18n.changeLanguage(language);
      const t = nyxbotI18n.t.bind(nyxbotI18n);
      window.history.replaceState(null, "", "/onboarding?step=success");
      sessionStorage.setItem(
        "nyxbot-onboarding:owner",
        JSON.stringify({
          channel: "telegram",
          botId: "business-bot",
          registrationId: "aevatar-registration",
          channelUrl: "https://t.me/my_shop_bot",
        }),
      );
      const first = await mount();
      await screen.findByRole("heading", { name: t("channelConnectedTitle") });
      first.unmount();
      client.clear();
      router.history.destroy();
      await mount();
      const link = await screen.findByRole("link", { name: t("openTelegram") });
      expect(link).toHaveAttribute("href", "https://t.me/my_shop_bot");
      expect(link).toHaveAttribute("rel", "noopener noreferrer");
      expect(router.state.location.search.step).toBe("success");
      expect(get).not.toHaveBeenCalled();
      expect(post).not.toHaveBeenCalled();
      expect(telegram).not.toHaveBeenCalled();
      await userEvent.click(
        screen.getByRole("button", { name: t("connectAnotherBot") }),
      );
      expect(
        await screen.findByLabelText(t("token"), { exact: true }),
      ).toHaveValue("");
      expect(router.state.location.search.step).toBe("channel");
      expect(
        screen.queryByRole("link", { name: t("openTelegram") }),
      ).not.toBeInTheDocument();
    },
  );
  it.each([
    {},
    { botId: "old-bot" },
    {
      botId: "old-bot",
      registrationId: "old-registration",
      channelUrl: "https://example.com/other_bot",
    },
  ])("returns an incomplete result URL to the form: %j", async (progress) => {
    window.history.replaceState(
      null,
      "",
      "/onboarding?step=success&botId=forged&completed=true",
    );
    sessionStorage.setItem(
      "nyxbot-onboarding:owner",
      JSON.stringify({ channel: "telegram", ...progress }),
    );
    await mount();
    await toChannel();
    expect(router.state.location.search.step).toBe("channel");
    expect(router.history.length).toBe(1);
    expect(post).not.toHaveBeenCalled();
  });
  it("only includes active, permitted IDs for the three requested slugs", async () => {
    userServices.push(
      {
        id: "unrelated-service",
        slug: "api-google",
        is_active: true,
        credential_source: { type: "personal" },
      },
      {
        id: "disabled-workspace",
        slug: "api-google-workspace",
        is_active: false,
        credential_source: { type: "personal" },
      },
      {
        id: "denied-ornn",
        slug: "ornn-api",
        is_active: true,
        credential_source: {
          type: "org",
          allowed: false,
          org_id: "org-2",
          org_name: "Read only",
          role: "viewer",
        },
      },
    );
    await mount();
    await toChannel();
    await userEvent.type(
      screen.getByLabelText("Customer bot token", { exact: true }),
      telegramToken,
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Connect channel" }),
    );
    await screen.findByRole("heading", { name: "Channel connected" });
    expect(post).toHaveBeenCalledWith(
      AEVATAR_CHANNELS_PATH,
      expect.objectContaining({
        service_ids: ["workspace-service", "ornn-service", "llm-service"],
      }),
    );
  });
  it.each([{ availableIds: [] }, { availableIds: ["ornn-service"] }])(
    "sends available service IDs explicitly when requested services are absent: $availableIds",
    async ({ availableIds }) => {
      userServices = userServices.filter((service) =>
        availableIds.includes(service.id),
      );
      await mount();
      await toChannel();
      await userEvent.type(
        screen.getByLabelText("Customer bot token", { exact: true }),
        telegramToken,
      );
      await userEvent.click(
        screen.getByRole("button", { name: "Connect channel" }),
      );
      await screen.findByRole("heading", {
        name: "Channel connected",
      });
      expect(post).toHaveBeenCalledWith(
        AEVATAR_CHANNELS_PATH,
        expect.objectContaining({ service_ids: availableIds }),
      );
    },
  );
  it("blocks submission until services load and retries a failed lookup without losing the token", async () => {
    const previousGet = get.getMockImplementation()!;
    let rejectServices: ((error: Error) => void) | undefined;
    get.mockImplementation((path: string) =>
      path === "/user-services"
        ? new Promise((_, reject) => {
            rejectServices = reject;
          })
        : previousGet(path),
    );
    await mount();
    await toChannel();
    const token = screen.getByLabelText("Customer bot token", { exact: true });
    await userEvent.type(token, telegramToken);
    const submit = screen.getByRole("button", { name: "Connect channel" });
    expect(submit).toBeDisabled();
    fireEvent.submit(token.closest("form")!);
    await act(async () => rejectServices?.(new Error("Service lookup failed")));
    await screen.findByText("We couldn't load your connections. Please retry.");
    expect(submit).toBeDisabled();
    expect(telegram).not.toHaveBeenCalled();
    expect(authorizer).not.toHaveBeenCalled();
    expect(post).not.toHaveBeenCalled();
    get.mockImplementation(previousGet);
    await userEvent.click(screen.getByRole("button", { name: "Retry" }));
    await waitFor(() => expect(submit).toBeEnabled());
    expect(token).toHaveValue(telegramToken);
    await userEvent.click(submit);
    await screen.findByRole("heading", { name: "Channel connected" });
    expect(post).toHaveBeenCalledWith(
      AEVATAR_CHANNELS_PATH,
      expect.objectContaining({
        service_ids: ["workspace-service", "ornn-service", "llm-service"],
      }),
    );
  });
  it.each(["en", "zh-CN"])(
    "blocks invalid tokens, shows localized feedback and accepts correction in %s",
    async (language) => {
      await nyxbotI18n.changeLanguage(language);
      const t = nyxbotI18n.t.bind(nyxbotI18n);
      await mount();
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
    await mount();
    await toChannel();
    const token = screen.getByLabelText("Customer bot token", { exact: true });
    const submit = screen.getByRole("button", { name: "Connect channel" });
    await userEvent.type(token, telegramToken);
    await userEvent.click(submit);
    await waitFor(() => expect(post).toHaveBeenCalledTimes(1));
    expect(submit).toBeDisabled();
    expect(token).toBeDisabled();
    expect(screen.getByRole("button", { name: "Back" })).toBeDisabled();
    expect(
      screen.getByRole("heading", { name: "Connect a customer channel" }),
    ).toBeVisible();
    await userEvent.click(submit);
    await act(async () => {
      fireEvent.submit(token.closest("form")!);
      fireEvent.submit(token.closest("form")!);
    });
    expect(post).toHaveBeenCalledTimes(1);
    await act(async () => resolveRegistration?.(registrationReceipt));
    await screen.findByRole("heading", { name: "Channel connected" });
  });
  it("offers first-time Aevatar consent and allows retry with the same bot token", async () => {
    authorizer.mockRejectedValueOnce(
      new AevatarAuthError("channelConsentRequired"),
    );
    await mount();
    await toChannel();
    await userEvent.type(
      screen.getByLabelText("Customer bot token", { exact: true }),
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
    await screen.findByRole("heading", { name: "Channel connected" });
    expect(post).toHaveBeenCalledTimes(1);
  });
  it("leaves the channel unselected for a direct visitor and ignores the disabled WhatsApp option", async () => {
    window.history.replaceState(null, "", "/onboarding");
    await mount();
    await userEvent.click(
      screen.getByRole("button", { name: "Continue as Avery" }),
    );
    await userEvent.click(
      await screen.findByRole("button", { name: "Continue", exact: true }),
    );
    await waitFor(() => expect(redirect).toHaveBeenCalledTimes(1));
    expect(router.state.location.search.step).toBe("source");
    await router.navigate({
      to: "/onboarding",
      search: { step: "channel", channel: "telegram" },
    });
    await toChannel();
    expect(screen.getByRole("radio", { name: /Telegram/ })).not.toBeChecked();
    expect(screen.getByRole("radio", { name: /WhatsApp/ })).not.toBeChecked();
    await userEvent.click(screen.getByRole("radio", { name: /Telegram/ }));
    await userEvent.type(
      screen.getByLabelText("Customer bot token", { exact: true }),
      "unsent-token",
    );
    await userEvent.click(screen.getByRole("radio", { name: /WhatsApp/ }));
    expect(screen.getByRole("radio", { name: /Telegram/ })).toBeChecked();
    expect(screen.getByRole("radio", { name: /WhatsApp/ })).not.toBeChecked();
    expect(
      screen.getByLabelText("Customer bot token", { exact: true }),
    ).toHaveValue("unsent-token");
  });
  it("preserves Google authorization when token verification fails and redacts provider errors", async () => {
    post.mockRejectedValueOnce(new Error(`Bad token: ${telegramToken}`));
    await mount();
    await toChannel();
    await userEvent.type(
      screen.getByLabelText("Customer bot token", { exact: true }),
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
      screen.getByRole("heading", { name: "Connect a customer channel" }),
    ).toBeVisible();
    await userEvent.click(screen.getByRole("button", { name: "Back" }));
    await screen.findByText("Google Workspace connected");
  });
  it("stops at the token field when Telegram rejects the token, without calling Aevatar", async () => {
    telegram.mockResolvedValue({ ok: false, status: 401 });
    await mount();
    await toChannel();
    await userEvent.type(
      screen.getByLabelText("Customer bot token", { exact: true }),
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
      screen.getByRole("heading", { name: "Connect a customer channel" }),
    ).toBeVisible();
    expect(
      sessionStorage.getItem("nyxbot-onboarding:owner") ?? "",
    ).not.toContain(telegramToken);
  });
  it("shows the Telegram link after Aevatar accepts the registration", async () => {
    await mount();
    await toChannel();
    await userEvent.type(
      screen.getByLabelText("Customer bot token", { exact: true }),
      telegramToken,
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Connect channel" }),
    );
    await screen.findByRole("heading", { name: "Channel connected" });
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
          "/onboarding?step=channel&channel=whatsapp",
        );
      } else {
        sessionStorage.setItem(
          "nyxbot-onboarding:owner",
          JSON.stringify({ channel: "whatsapp" }),
        );
      }
      await mount();
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
      expect(
        screen.getByLabelText("Customer bot token", { exact: true }),
      ).toBeVisible();
      expect(post).not.toHaveBeenCalled();
    },
  );
  it.each([
    "/onboarding?status=success",
    "/onboarding?provider_status=success",
    "/onboarding?step=source&provider_status=success",
    "/onboarding?step=channel&provider_status=success",
  ])("checks real scopes after OAuth instead of trusting %s", async (path) => {
    keys = [{ ...googleKey, granted_scopes: ["openid", "email", "profile"] }];
    window.history.replaceState(null, "", path);
    await mount();
    const button = await screen.findByRole("button", {
      name: "Connect Google",
    });
    await waitFor(() => expect(button).toBeEnabled());
    expect(
      screen.queryByText("Google Workspace connected"),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("heading", { name: "Connect a customer channel" }),
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
  });
  it("requests Drive and Calendar consent on click without using catalog configuration as a grant prerequisite", async () => {
    window.history.replaceState(null, "", "/onboarding?step=source");
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
    await mount();
    const button = await screen.findByRole("button", {
      name: "Connect Google",
    });
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
    window.history.replaceState(null, "", "/onboarding?step=source");
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
    await mount();
    const button = await screen.findByRole("button", {
      name: "Connect Google",
    });
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
    window.history.replaceState(null, "", "/onboarding?step=source");
    keys = [];
    catalogResponse = { ...catalog, provider_config_id: "" };
    await mount();
    await screen.findByText(
      /Google Drive and Calendar authorization is not available/,
    );
    expect(
      screen.getByRole("button", { name: "Connect Google" }),
    ).toBeDisabled();
    expect(post).not.toHaveBeenCalled();
  });
  it("continues from account to data source before resuming a saved channel", async () => {
    window.history.replaceState(null, "", "/onboarding");
    sessionStorage.setItem(
      "nyxbot-onboarding:owner",
      JSON.stringify({ botId: "business-bot", channel: "telegram" }),
    );
    await mount();
    await screen.findByRole("heading", { name: "Sign in to NyxID" });
    expect(get).not.toHaveBeenCalledWith("/channel-bots/business-bot");
    await userEvent.click(
      screen.getByRole("button", { name: "Continue as Avery" }),
    );
    await screen.findByText("Google Workspace connected");
    expect(router.state.location.search.step).toBe("source");
    expect(get).not.toHaveBeenCalledWith("/channel-bots/business-bot");
    await userEvent.click(
      screen.getByRole("button", { name: "Continue", exact: true }),
    );
    await waitFor(() => expect(redirect).toHaveBeenCalledTimes(1));
    expect(router.state.location.search.step).toBe("source");
    expect(get).not.toHaveBeenCalledWith("/channel-bots/business-bot");
    expect(post).toHaveBeenCalledWith(
      "/keys",
      expect.objectContaining({
        service_slug: "api-google",
      }),
    );
  });
  it("updates channel content and step navigation with the feature's language", async () => {
    await mount();
    await toChannel();
    await act(async () => {
      await nyxbotI18n.changeLanguage("zh-CN");
    });
    await screen.findByRole("heading", { name: "连接客户渠道" });
    expect(screen.getByText("渠道").closest("li")).toHaveAttribute(
      "aria-current",
      "step",
    );
    expect(
      screen.getByLabelText("客户机器人 token", { exact: true }),
    ).toBeVisible();
    expect(screen.getByRole("button", { name: "连接渠道" })).toBeDisabled();
  });

  it("redirects legacy link URLs to the channel step", async () => {
    window.history.replaceState(null, "", "/onboarding?step=link");
    sessionStorage.setItem(
      "nyxbot-onboarding:owner",
      JSON.stringify({ channel: "telegram" }),
    );
    await mount();
    await toChannel();
    expect(router.state.location.search.step).toBe("channel");
    expect(get).not.toHaveBeenCalledWith("/channel-bots/deleted-bot");
  });

  it("drops a deleted OAuth placeholder on retry instead of trapping the user", async () => {
    window.history.replaceState(null, "", "/onboarding?step=source");
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
    await mount();
    await screen.findByText("We couldn't load your connections. Please retry.");
    await userEvent.click(screen.getByRole("button", { name: "Retry" }));
    await userEvent.click(
      await screen.findByRole("button", { name: "Continue", exact: true }),
    );
    await waitFor(() => expect(redirect).toHaveBeenCalledTimes(1));
    expect(router.state.location.search.step).toBe("source");
    expect(
      JSON.parse(sessionStorage.getItem("nyxbot-onboarding:owner")!)
        .googleKeyId,
    ).toBe("google-1");
  });
});
