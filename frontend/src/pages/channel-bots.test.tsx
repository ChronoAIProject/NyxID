import { platformFixture, platformFixtures } from "@/test/fixtures/channel-platforms";
import { StrictMode } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  RouterProvider,
} from "@tanstack/react-router";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { ChannelBotsPage } from "./channel-bots";
import { ChannelBotSetupLinksPage, ChannelBotSetupPage } from "./channel-bot-setup";
import { channelBotSetupRewrite, parseChannelBotSetupPageSearch } from "@/schemas/channel-bot-setup";
import type { TelegramNewRequest } from "@/schemas/telegram-new";

const { get, post, remove, useOrgs, authState } = vi.hoisted(() => ({
  get: vi.fn(),
  post: vi.fn(),
  remove: vi.fn(),
  useOrgs: vi.fn(),
  authState: { isAuthenticated: true, user: { id: "actor-1" } },
}));
vi.mock("@/lib/api-client", async (original) => ({
  ...(await original<object>()),
  api: { get, post, delete: remove },
}));
vi.mock("@/components/layout/dashboard-layout", () => ({ useBreadcrumbLabel: () => {} }));
vi.mock("@/hooks/use-orgs", () => ({ useOrgs }));
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (selector: (state: typeof authState) => unknown) =>
    selector(authState),
}));
vi.mock("@/components/shared/org-scope-select", () => ({
  OrgScopeSelect: ({
    value,
    onChange,
    disabled,
    allowAll,
    label = "Scope",
    personalLabel = "Personal",
  }: {
    value: string | null;
    onChange: (next: string | null) => void;
    disabled?: boolean;
    allowAll?: boolean;
    label?: string;
    personalLabel?: string;
  }) => (
    <select
      aria-label={label}
      value={value ?? ""}
      disabled={disabled}
      onChange={(event) => onChange(event.target.value || null)}
    >
      {allowAll && <option value="all">View all</option>}
      <option value="">{personalLabel}</option>
      <option value="cf8812f3-ff2a-46a9-8c6b-63f7bc1f5d65">Support team</option>
    </select>
  ),
}));

const root = "/channel-bots/telegram-new";
const orgId = "cf8812f3-ff2a-46a9-8c6b-63f7bc1f5d65";
const request: TelegramNewRequest = {
  id: "3c638c7f-210a-44fc-9b67-f6c878d67c54",
  status: "waiting_telegram",
  revision: 1,
  owner_user_id: orgId,
  label: "Saved support",
  expires_at: "2026-09-16T10:00:00Z",
  telegram_bot_id: null,
  bot_username: null,
  channel_bot_id: null,
  auto_connect: true,
};
let saved: TelegramNewRequest | null;
let available: boolean;

async function setup(url = "/channel-bots?connect=telegram-new") {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  const rootRoute = createRootRoute();
  const dashboard = createRoute({ getParentRoute: () => rootRoute, id: "dashboard" });
  const list = createRoute({
    getParentRoute: () => dashboard,
    path: "/channel-bots",
    component: ChannelBotsPage,
    validateSearch: (search) => search,
  });
  const detail = createRoute({
    getParentRoute: () => dashboard,
    path: "/channel-bots/$botId",
    component: () => <h1>Bot details</h1>,
  });
  const history = createMemoryHistory({ initialEntries: [url] });
  const setupLinks = createRoute({
    getParentRoute: () => dashboard,
    path: "/channel-bots/connect",
    component: ChannelBotSetupLinksPage,
  });
  const setupPage = createRoute({
    getParentRoute: () => rootRoute,
    path: "/channel-bots/connect/$platform",
    validateSearch: parseChannelBotSetupPageSearch,
    component: ChannelBotSetupPage,
  });
  const router = createRouter({
    routeTree: rootRoute.addChildren([setupPage, dashboard.addChildren([list, detail, setupLinks])]),
    history,
    rewrite: channelBotSetupRewrite,
  });
  const view = render(
    <StrictMode>
      <QueryClientProvider client={client}>
        <RouterProvider router={router} />
      </QueryClientProvider>
    </StrictMode>,
  );
  await act(() => router.load());
  return { ...view, client, router, history };
}

beforeEach(() => {
  vi.resetAllMocks();
  authState.isAuthenticated = true;
  useOrgs.mockReturnValue({ data: [{ id: orgId, your_role: "admin" }], isError: false });
  saved = null;
  available = true;
  get.mockImplementation(async (path: string) => {
    if (path === "/orgs") return { orgs: [{ id: orgId, display_name: "Support team", your_role: "admin" }] };
    if (path === "/channel-platforms") return { platforms: platformFixtures };
    if (path.startsWith(root))
      return { available, manager_username: "NyxSetupBot", request: saved };
    if (path.startsWith("/channel-bots")) return { bots: [], total: 0 };
    if (path.startsWith("/channel-conversations"))
      return { conversations: [], total: 0 };
    if (path === "/api-keys") return { keys: [] };
    throw new Error(`Unexpected GET ${path}`);
  });
  vi.spyOn(window, "open").mockReturnValue(null);
});

it("defaults to all bots, narrows by owner, and creates personal bots from View all", async () => {
  const user = userEvent.setup();
  await setup("/channel-bots");
  const scope = screen.getByRole("combobox", { name: "Scope" });
  expect(scope).toHaveValue("all");
  await waitFor(() => expect(get).toHaveBeenCalledWith("/channel-bots?scope=all"));
  await user.selectOptions(scope, "");
  expect(within(scope).getByRole("option", { selected: true })).toHaveTextContent("User");
  await waitFor(() => expect(get).toHaveBeenCalledWith("/channel-bots?scope=user"));
  await user.selectOptions(scope, orgId);
  await waitFor(() => expect(get).toHaveBeenCalledWith(`/channel-bots?org_id=${orgId}`));
  await user.selectOptions(scope, "all");
  const deviceScope = screen.getByRole("combobox", { name: "Device channel scope" });
  expect(deviceScope).toHaveValue("");
  await user.selectOptions(deviceScope, orgId);
  await waitFor(() => expect(get).toHaveBeenCalledWith(`/channel-conversations?org_id=${orgId}`));
  expect(scope).toHaveValue("all");
  await user.click(screen.getByRole("button", { name: "Add Bot" }));
  const dialog = within(await screen.findByRole("dialog"));
  expect(dialog.getByRole("combobox", { name: "Scope" })).toHaveValue("");
  expect(dialog.queryByRole("option", { name: "View all" })).not.toBeInTheDocument();
});

it("identifies personal and organization owners in the combined list", async () => {
  const user = userEvent.setup();
  const fallbackGet = get.getMockImplementation()!;
  get.mockImplementation(async (path: string) => {
    if (path === "/channel-bots?scope=all") return {
      bots: [
        { id: "personal-bot", label: "Personal bot", user_id: "actor-1" },
        { id: "org-bot", label: "Support bot", user_id: orgId },
      ].map((bot) => ({
        ...bot,
        platform: "telegram",
        platform_bot_username: "bot",
        credential_source: "user",
        status: "active",
        is_active: true,
        webhook_registered: true,
        created_at: "2026-09-16T10:00:00Z",
      })),
      total: 2,
    };
    return fallbackGet(path);
  });
  await setup("/channel-bots");
  await screen.findAllByText("Support bot");
  await user.click(screen.getByRole("button", { name: "Table view" }));
  expect(within(screen.getByRole("row", { name: /Personal bot/ }))
    .getByRole("cell", { name: "User" })).toBeInTheDocument();
  expect(within(screen.getByRole("row", { name: /Support bot/ }))
    .getByRole("cell", { name: "Support team" })).toBeInTheDocument();
});

it("persists a draft from the shared modal fields with replacement navigation and restores it on remount", async () => {
  const user = userEvent.setup();
  const view = await setup();
  const dialog = within(await screen.findByRole("dialog"));
  const label = dialog.getByLabelText("Label", { exact: true });
  await waitFor(() => expect(label).toBeEnabled());
  await user.type(label, "Draft support");
  await user.selectOptions(
    dialog.getByRole("combobox", { name: "Scope" }),
    orgId,
  );
  await waitFor(() =>
    expect(view.router.state.location.search).toEqual({
      connect: "telegram-new",
      label: "Draft support",
      target_org_id: orgId,
    }),
  );
  expect(view.history.length).toBe(1);
  const url = view.router.state.location.href;
  view.unmount();
  view.client.clear();
  await setup(url);
  const resumed = within(await screen.findByRole("dialog"));
  expect(resumed.getByLabelText("Label", { exact: true })).toHaveValue(
    "Draft support",
  );
  expect(resumed.getByRole("combobox", { name: "Scope" })).toHaveValue(orgId);
  expect(
    screen.getByRole("heading", {
      name: "Channel Bots",
      hidden: true,
    }),
  ).toBeInTheDocument();
  expect(
    get.mock.calls.some(([path]) => path.includes("managed-onboarding")),
  ).toBe(false);
});

it.each([orgId, "actor-1"])(
  "restores request_id and locks the server-saved label and owner %s over URL drafts",
  async (owner) => {
    saved = { ...request, owner_user_id: owner };
    const view = await setup(
      `/channel-bots?connect=telegram-new&label=Stale&target_org_id=${orgId}&request_id=${request.id}`,
    );
    const dialog = within(await screen.findByRole("dialog"));
    await waitFor(() =>
      expect(dialog.getByLabelText("Label", { exact: true })).toHaveValue(
        request.label,
      ),
    );
    expect(dialog.getByLabelText("Label", { exact: true })).toBeDisabled();
    expect(dialog.getByRole("combobox", { name: "Scope" })).toHaveValue(
      owner === orgId ? orgId : "",
    );
    expect(dialog.getByRole("combobox", { name: "Scope" })).toBeDisabled();
    expect(
      dialog.getByText(/To change them, choose Cancel setup/),
    ).toBeVisible();
    expect(get).toHaveBeenCalledWith(`${root}?request_id=${request.id}`);
    await waitFor(() =>
      expect(view.router.state.location.search).toEqual({
        connect: "telegram-new",
        label: request.label,
        target_org_id: owner === orgId ? orgId : undefined,
        request_id: request.id,
      }),
    );
    expect(post).not.toHaveBeenCalled();
  },
);

it("records a started request without putting its launch challenge in the URL", async () => {
  post.mockImplementation(async () => {
    saved = request;
    return {
      request,
      launch_url: "https://t.me/NyxSetupBot?start=private-challenge",
    };
  });
  const user = userEvent.setup();
  const view = await setup(
    `/channel-bots?connect=telegram-new&label=Saved%20support&target_org_id=${orgId}`,
  );
  const dialog = within(await screen.findByRole("dialog"));
  await user.click(
    await dialog.findByRole("button", { name: "Continue in Telegram" }),
  );
  await waitFor(() =>
    expect(view.router.state.location.search.request_id).toBe(request.id),
  );
  expect(post).toHaveBeenCalledExactlyOnceWith(root, {
    label: request.label,
    target_org_id: orgId,
    auto_connect: true,
  });
  expect(view.router.state.location.href).not.toContain("challenge");
  expect(dialog.getByLabelText("Label", { exact: true })).toBeDisabled();
});

it("dismisses without cancelling or reopening, then resumes through a later search change", async () => {
  saved = request;
  const user = userEvent.setup();
  const view = await setup();
  await screen.findByText(/To change them, choose Cancel setup/);
  await user.click(
    within(screen.getByRole("dialog")).getByRole("button", { name: "Close" }),
  );
  await waitFor(() =>
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
  );
  expect(view.router.state.location.search.connect).toBeUndefined();
  expect(view.router.state.location.search.request_id).toBe(request.id);
  await act(() =>
    view.client.invalidateQueries({ queryKey: ["telegram-new"] }),
  );
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(remove).not.toHaveBeenCalled();
  await user.click(
    screen.getByRole("button", { name: "Resume Telegram setup" }),
  );
  const dialog = within(await screen.findByRole("dialog"));
  await waitFor(() =>
    expect(dialog.getByLabelText("Label", { exact: true })).toHaveValue(
      request.label,
    ),
  );
  expect(remove).not.toHaveBeenCalled();
  expect(post).not.toHaveBeenCalled();
});

it("only Cancel setup cancels the request, removes request_id, and unlocks the shared fields", async () => {
  saved = request;
  remove.mockImplementation(async () => {
    saved = null;
  });
  const user = userEvent.setup();
  const view = await setup(
    `/channel-bots?connect=telegram-new&request_id=${request.id}`,
  );
  const dialog = within(await screen.findByRole("dialog"));
  await user.click(await dialog.findByRole("button", { name: "Cancel setup" }));
  await waitFor(() =>
    expect(dialog.getByLabelText("Label", { exact: true })).toBeEnabled(),
  );
  expect(dialog.getByRole("combobox", { name: "Scope" })).toBeEnabled();
  expect(dialog.getByLabelText("Label", { exact: true })).toHaveValue(
    request.label,
  );
  await waitFor(() =>
    expect(view.router.state.location.search.request_id).toBeUndefined(),
  );
  expect(view.router.state.location.search.connect).toBe("telegram-new");
  expect(remove).toHaveBeenCalledExactlyOnceWith(
    `${root}/requests/${request.id}`,
  );
});

it("follows a completed request_id to bot details and closes the modal", async () => {
  saved = {
    ...request,
    status: "connected",
    channel_bot_id: "connected-bot",
    bot_username: "SupportBot",
    telegram_bot_id: "900",
  };
  const view = await setup(
    `/channel-bots?connect=telegram-new&request_id=${request.id}`,
  );
  await screen.findByRole("heading", { name: "Bot details" });
  expect(view.router.state.location.pathname).toBe(
    "/channel-bots/connected-bot",
  );
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(get).toHaveBeenCalledWith(`${root}?request_id=${request.id}`);
  expect(post).not.toHaveBeenCalled();
});

it("shows the manager configuration message inside the modal without managed onboarding", async () => {
  available = false;
  await setup();
  const dialog = within(await screen.findByRole("dialog"));
  expect(
    await dialog.findByText(
      /An administrator needs to configure a manager bot/,
    ),
  ).toBeVisible();
  expect(
    get.mock.calls.some(([path]) => path.includes("managed-onboarding")),
  ).toBe(false);
  expect(dialog.queryByLabelText("Bot token")).not.toBeInTheDocument();
});

it("locks the destination and label while the start request is still being saved", async () => {
  let finish!: () => void;
  post.mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = () => {
          saved = request;
          resolve({
            request,
            launch_url: "https://t.me/NyxSetupBot?start=private-challenge",
          });
        };
      }),
  );
  const user = userEvent.setup();
  await setup(
    `/channel-bots?connect=telegram-new&label=Saved&target_org_id=${orgId}`,
  );
  const dialog = within(await screen.findByRole("dialog"));
  await user.click(
    await dialog.findByRole("button", { name: "Continue in Telegram" }),
  );
  try {
    expect(dialog.getByLabelText("Label", { exact: true })).toBeDisabled();
    expect(dialog.getByRole("combobox", { name: "Scope" })).toBeDisabled();
  } finally {
    await act(async () => finish());
  }
});

it("does not reopen a dismissed modal when a slow cancellation completes", async () => {
  saved = request;
  let finish!: () => void;
  remove.mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = () => {
          saved = null;
          resolve({});
        };
      }),
  );
  const user = userEvent.setup();
  const view = await setup(
    `/channel-bots?connect=telegram-new&request_id=${request.id}`,
  );
  const dialog = within(await screen.findByRole("dialog"));
  await user.click(await dialog.findByRole("button", { name: "Cancel setup" }));
  await user.click(dialog.getByRole("button", { name: "Close" }));
  await waitFor(() =>
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
  );
  await act(async () => finish());
  await waitFor(() => expect(view.client.isMutating()).toBe(0));
  expect(view.router.state.location.search.connect).toBeUndefined();
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(remove).toHaveBeenCalledExactlyOnceWith(
    `${root}/requests/${request.id}`,
  );
});

it("honors a later Telegram entry while the Add Bot dialog is already open", async () => {
  const user = userEvent.setup();
  const view = await setup("/channel-bots");
  await user.click(screen.getByRole("button", { name: "Add Bot" }));
  expect(
    within(await screen.findByRole("dialog")).getByLabelText("Bot token"),
  ).toBeVisible();
  await act(() =>
    view.router.navigate({
      to: "/channel-bots",
      search: {
        connect: "telegram-new",
        label: "Incoming draft",
        target_org_id: orgId,
      },
    }),
  );
  const dialog = within(screen.getByRole("dialog"));
  expect(
    await dialog.findByRole("button", { name: "Continue in Telegram" }),
  ).toBeVisible();
  expect(dialog.getByLabelText("Label", { exact: true })).toHaveValue(
    "Incoming draft",
  );
  expect(dialog.getByRole("combobox", { name: "Scope" })).toHaveValue(orgId);
});

it("a slow start finishing after dismissal stays saved without reopening or cancelling", async () => {
  let finish!: () => void;
  post.mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = () => {
          saved = request;
          resolve({
            request,
            launch_url: "https://t.me/NyxSetupBot?start=private-challenge",
          });
        };
      }),
  );
  const user = userEvent.setup();
  const view = await setup(
    `/channel-bots?connect=telegram-new&label=Saved&target_org_id=${orgId}`,
  );
  const dialog = within(await screen.findByRole("dialog"));
  await user.click(
    await dialog.findByRole("button", { name: "Continue in Telegram" }),
  );
  await user.keyboard("{Escape}");
  await waitFor(() =>
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
  );
  await act(async () => finish());
  await waitFor(() => expect(view.client.isMutating()).toBe(0));
  expect(view.router.state.location.search.connect).toBeUndefined();
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(remove).not.toHaveBeenCalled();
  await user.click(
    await screen.findByRole("button", { name: "Resume Telegram setup" }),
  );
  expect(
    within(await screen.findByRole("dialog")).getByLabelText("Label", {
      exact: true,
    }),
  ).toHaveValue(request.label);
  expect(post).toHaveBeenCalledTimes(1);
});

it("re-seeds scope when reopening Add Bot after changing the list scope", async () => {
  const user = userEvent.setup();
  await setup("/channel-bots");
  await user.click(screen.getByRole("button", { name: "Add Bot" }));
  let dialog = within(await screen.findByRole("dialog"));
  expect(dialog.getByRole("combobox", { name: "Scope" })).toHaveValue("");
  await user.click(dialog.getByRole("button", { name: /^Cancel$/ }));
  await user.selectOptions(
    screen.getByRole("combobox", { name: "Scope" }),
    orgId,
  );
  await user.click(screen.getByRole("button", { name: "Add Bot" }));
  dialog = within(await screen.findByRole("dialog"));
  expect(dialog.getByRole("combobox", { name: "Scope" })).toHaveValue(orgId);
  await user.type(dialog.getByLabelText("Label", { exact: true }), "Org bot");
  await user.type(
    dialog.getByLabelText("Bot token", { exact: true }),
    "fixture-token",
  );
  post.mockResolvedValue({ id: "org-bot", platform_bot_username: "OrgBot" });
  await user.click(
    dialog.getByRole("button", { name: /^Add Bot$/ }),
  );
  expect(post).toHaveBeenCalledExactlyOnceWith("/channel-bots", {
    platform: "telegram",
    label: "Org bot",
    target_org_id: orgId,
    bot_token: "fixture-token",
  });
});


it.each(["lark", "feishu"])("registers %s with app credentials and no bot token input", async (platform) => {
  post.mockResolvedValue({ id: "app-bot", platform, status: "active" });
  const user = userEvent.setup();
  await setup(`/channel-bots?connect=${platform}`);
  const dialog = within(await screen.findByRole("dialog"));
  await dialog.findByLabelText("app_id");
  expect(dialog.queryByLabelText("Bot token")).not.toBeInTheDocument();
  await user.type(dialog.getByLabelText("Label", { exact: true }), "Support");
  await user.type(dialog.getByLabelText("app_id"), "cli_test");
  await user.type(dialog.getByLabelText("app_secret"), "app-secret");
  const submit = dialog.getByRole("button", { name: "Add Bot" });
  expect(submit).toBeDisabled();
  await user.type(dialog.getByLabelText("verification_token"), "verification-token");
  await waitFor(() => expect(submit).toBeEnabled());
  await user.click(submit);
  await waitFor(() => expect(post).toHaveBeenCalledExactlyOnceWith("/channel-bots", {
    platform, label: "Support", target_org_id: undefined,
    app_id: "cli_test", app_secret: "app-secret", verification_token: "verification-token",
  }));
});

it("renders and validates required secret fields from the catalog, including new fields", async () => {
  const getDefault = get.getMockImplementation()!;
  get.mockImplementation(async (path: string) => {
    if (path === "/channel-platforms") return { platforms: [{
      ...platformFixtures[0], display_name: "Catalog-defined Telegram",
      registration: { ...platformFixtures[0]!.registration, setup_instructions: ["Enable your workspace before connecting."], fields: [{
        name: "future_secret", label: "Workspace credential", secret: true, required: true,
        hint: "Copy the credential from workspace settings.",
        patchable: false, clearable: false, storage: "future_secret_encrypted", webhook_secret: false, platform_fallback: null,
      }] },
    }] };
    return getDefault(path);
  });
  post.mockResolvedValue({ id: "new-bot", platform: "telegram", status: "active" });
  const user = userEvent.setup();
  await setup("/channel-bots?connect=telegram");
  const dialog = within(await screen.findByRole("dialog"));
  const secret = await dialog.findByLabelText("Workspace credential");
  expect(secret).toHaveAttribute("type", "password");
  expect(dialog.getByText("Copy the credential from workspace settings.")).toBeVisible();
  expect(dialog.getByText("Enable your workspace before connecting.")).toBeVisible();
  expect(dialog.queryByLabelText("Bot token")).not.toBeInTheDocument();
  const submit = dialog.getByRole("button", { name: "Add Bot" });
  await user.type(dialog.getByLabelText("Label", { exact: true }), "Catalog bot");
  expect(submit).toBeDisabled();
  await user.type(secret, "private-field");
  await waitFor(() => expect(submit).toBeEnabled());
  await user.click(submit);
  await waitFor(() => expect(post).toHaveBeenCalledWith("/channel-bots", expect.objectContaining({ future_secret: "private-field", label: "Catalog bot" })));
});

it("generates shareable setup links for enabled catalog entries, including new platforms", async () => {
  const future = { ...platformFixture("future-chat"), display_name: "Future Chat" };
  get.mockResolvedValue({ platforms: [...platformFixtures, future, { ...platformFixture("disabled-chat"), enabled: false }] });
  const user = userEvent.setup();
  const clipboard = vi.spyOn(navigator.clipboard, "writeText");
  await setup("/channel-bots/connect");
  for (const platform of [...platformFixtures, future]) {
    expect(await screen.findByRole("link", { name: `Set up ${platform.display_name}` }))
      .toHaveAttribute("href", `/channel-bots/connect/${platform.platform}`);
  }
  expect(screen.queryByText("disabled-chat")).not.toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Copy Future Chat setup link" }));
  expect(clipboard).toHaveBeenCalledWith(`${window.location.origin}/channel-bots/connect/future-chat`);
  expect(post).not.toHaveBeenCalled();
});

it.each(platformFixtures)("opens a full setup page for $platform without a dialog or platform picker", async (platform) => {
  await setup(`/channel-bots/connect/${platform.platform}`);
  expect(await screen.findByRole("heading", { name: `Create your ${platform.platform.startsWith("telegram") ? "Telegram" : platform.display_name} channel bot` })).toBeVisible();
  expect(await screen.findByLabelText("Bot name", { exact: true })).toBeVisible();
  expect(screen.getByRole("img", { name: /^NyxID connects to/ })).toBeVisible();
  expect(screen.queryByRole("list", { name: "Connection progress" })).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: /^Cancel/ })).not.toBeInTheDocument();
  expect(screen.queryByText("How setup works")).not.toBeInTheDocument();
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(screen.queryByRole("combobox", { name: "Platform" })).not.toBeInTheDocument();
  expect(post).not.toHaveBeenCalled();
});

it("does not render the connection page or fetch its catalog before sign-in", async () => {
  authState.isAuthenticated = false;
  await setup("/channel-bots/connect/telegram");
  expect(screen.queryByRole("main")).not.toBeInTheDocument();
  expect(get).not.toHaveBeenCalled();
});

it.each([[], [{ id: orgId, your_role: "member" }], [{ id: orgId, your_role: "viewer" }]])("hides ownership when Personal is the only eligible scope (%j)", async (...orgs) => {
  useOrgs.mockReturnValue({ data: orgs, isError: false });
  const user = userEvent.setup();
  post.mockResolvedValue({ id: "personal-bot", platform: "telegram", status: "active" });
  await setup("/channel-bots/connect/telegram?bot_token=fixture-token");
  expect(await screen.findByLabelText("Bot name")).toHaveValue("Telegram bot");
  expect(screen.queryByRole("combobox", { name: "Create for" })).not.toBeInTheDocument();
  const submit = screen.getByRole("button", { name: "Create channel bot" });
  await waitFor(() => expect(submit).toBeEnabled());
  await user.click(submit);
  expect(post).toHaveBeenCalledExactlyOnceWith("/channel-bots", {
    platform: "telegram", label: "Telegram bot", bot_token: "fixture-token", target_org_id: undefined,
  });
});

it("shows ownership when an admin organization is available and preserves its prefill", async () => {
  await setup(`/channel-bots/connect/telegram?target_org_id=${orgId}`);
  expect(await screen.findByRole("combobox", { name: "Create for" })).toHaveValue(orgId);
});

it.each(["", "%20%20"])("uses the default bot name for a blank label=%s", async (label) => {
  await setup(`/channel-bots/connect/telegram?label=${label}`);
  expect(await screen.findByLabelText("Bot name", { exact: true })).toHaveValue("Telegram bot");
});

it("keeps an explicitly selected organization visible when membership cannot be confirmed", async () => {
  useOrgs.mockReturnValue({ data: [], isError: true });
  await setup(`/channel-bots/connect/telegram?target_org_id=${orgId}`);
  expect(await screen.findByRole("combobox", { name: "Create for" })).toHaveValue(orgId);
});

it("starts managed Telegram from a plain link with one click and no required edits", async () => {
  useOrgs.mockReturnValue({ data: [], isError: false });
  post.mockImplementation(async () => {
    saved = { ...request, owner_user_id: "actor-1", label: "Telegram bot" };
    return { request: saved, launch_url: "https://t.me/NyxSetupBot?start=fixture" };
  });
  const user = userEvent.setup();
  const view = await setup("/channel-bots/connect/telegram-new");
  const submit = await screen.findByRole("button", { name: "Continue in Telegram" });
  expect(submit).toBeEnabled();
  expect(screen.queryByRole("list", { name: "Telegram setup steps" })).not.toBeInTheDocument();
  await user.click(submit);
  await waitFor(() => expect(post).toHaveBeenCalledExactlyOnceWith(root, {
    label: "Telegram bot", target_org_id: undefined, auto_connect: true,
  }));
  expect(await screen.findByRole("button", { name: "Reopen Telegram" })).toBeVisible();
  expect(screen.queryByRole("button", { name: /Cancel/ })).not.toBeInTheDocument();
  expect(view.router.state.location.search.request_id).toBe(request.id);
});

it("registers a newly described platform on its dedicated page using catalog fields and instructions", async () => {
  const future = platformFixture("future-chat");
  get.mockResolvedValue({ platforms: [{ ...future, registration: { ...future.registration, setup_instructions: ["Enable your workspace before connecting."] } }] });
  post.mockResolvedValue({ id: "future-bot", platform: "future-chat", status: "active" });
  const user = userEvent.setup();
  const view = await setup(`/channel-bots/connect/future-chat?label=Team%20bot&target_org_id=${orgId}`);
  const secret = await screen.findByLabelText("Bot token");
  await user.click(screen.getByText("Setup instructions"));
  expect(screen.getByText("Enable your workspace before connecting.")).toBeVisible();
  expect(screen.getByRole("button", { name: "Create channel bot" })).toBeDisabled();
  await user.type(secret, "fixture-token");
  await user.click(screen.getByRole("button", { name: "Create channel bot" }));
  expect(await screen.findByRole("heading", { name: "Your future-chat channel bot is created" })).toBeVisible();
  expect(view.router.state.location.pathname).toBe("/channel-bots/connect/future-chat");
  await user.click(screen.getByRole("button", { name: "Open channel bot" }));
  await screen.findByRole("heading", { name: "Bot details" });
  expect(post).toHaveBeenCalledExactlyOnceWith("/channel-bots", {
    platform: "future-chat", label: "Team bot", target_org_id: orgId, bot_token: "fixture-token",
  });
  expect(view.history.length).toBe(2);
});

it("shows one-time verification secrets on the full page until the user continues", async () => {
  post.mockResolvedValue({ id: "created-bot", platform: "telegram", webhook_url: "https://example.test/webhook", webhook_secret: "one-time-secret", webhook_secret_label: "Verify Token", setup_instructions: ["Save this token in the platform console."] });
  const user = userEvent.setup();
  const view = await setup("/channel-bots/connect/telegram?label=Support");
  await user.type(await screen.findByLabelText("Bot token"), "fixture-token");
  await user.click(screen.getByRole("button", { name: "Create channel bot" }));
  expect(await screen.findByText("one-time-secret")).toBeVisible();
  expect(screen.getByText("Save this token in the platform console.")).toBeVisible();
  expect(view.router.state.location.pathname).toBe("/channel-bots/connect/telegram");
  expect(screen.queryByLabelText("Bot token")).not.toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Open channel bot" }));
  await screen.findByRole("heading", { name: "Bot details" });
  expect(view.router.state.location.pathname).toBe("/channel-bots/created-bot");
  expect(view.history.length).toBe(2);
});

it.each(["unknown-chat", "disabled-chat"])("does not offer creation for %s", async (platform) => {
  get.mockResolvedValue({ platforms: [{ ...platformFixture("disabled-chat"), enabled: false }] });
  await setup(`/channel-bots/connect/${platform}`);
  expect(await screen.findByRole("link", { name: "Choose another channel" })).toBeVisible();
  expect(screen.queryByLabelText("Bot name", { exact: true })).not.toBeInTheDocument();
  expect(post).not.toHaveBeenCalled();
});

it("retries catalog failures before displaying a setup form", async () => {
  get.mockRejectedValueOnce(new Error("Unavailable"));
  const user = userEvent.setup();
  await setup("/channel-bots/connect/discord");
  expect(await screen.findByText("Unable to load channel setup.")).toBeVisible();
  expect(screen.queryByLabelText("Bot name", { exact: true })).not.toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Retry" }));
  expect(await screen.findByLabelText("public_key")).toBeVisible();
});

it("keeps Telegram drafts on the dedicated page across reloads", async () => {
  const user = userEvent.setup();
  const view = await setup("/channel-bots/connect/telegram-new");
  const label = await screen.findByLabelText("Bot name", { exact: true });
  await waitFor(() => expect(label).toBeEnabled());
  await user.clear(label);
  await user.type(label, "Draft support");
  await user.selectOptions(screen.getByRole("combobox", { name: "Create for" }), orgId);
  await waitFor(() => expect(view.router.state.location.search).toEqual({ label: "Draft support", target_org_id: orgId }));
  expect(view.router.state.location.pathname).toBe("/channel-bots/connect/telegram-new");
  expect(view.history.length).toBe(1);
  const url = view.router.state.location.href;
  view.unmount();
  await setup(url);
  expect(await screen.findByLabelText("Bot name", { exact: true })).toHaveValue("Draft support");
  expect(screen.getByRole("combobox", { name: "Create for" })).toHaveValue(orgId);
});

it("restores a saved Telegram request on its dedicated page", async () => {
  saved = { ...request };
  const view = await setup(`/channel-bots/connect/telegram-new?request_id=${request.id}`);
  const label = await screen.findByLabelText("Bot name", { exact: true });
  await waitFor(() => expect(label).toHaveValue(request.label));
  expect(label).toBeDisabled();
  expect(screen.getByRole("combobox", { name: "Create for" })).toHaveValue(orgId);
  expect(view.router.state.location.pathname).toBe("/channel-bots/connect/telegram-new");
  expect(view.router.state.location.search.request_id).toBe(request.id);
});

it("keeps managed Telegram completion in the connection wizard until the user continues", async () => {
  saved = { ...request, status: "connected", channel_bot_id: "created-telegram", bot_username: "SupportBot" };
  const user = userEvent.setup();
  const view = await setup(`/channel-bots/connect/telegram-new?request_id=${request.id}`);
  expect(await screen.findByRole("heading", { name: "Your Telegram channel bot is created" })).toBeVisible();
  expect(view.router.state.location.pathname).toBe("/channel-bots/connect/telegram-new");
  expect(screen.queryByLabelText("Bot name")).not.toBeInTheDocument();

  await user.click(screen.getByRole("button", { name: "Open channel bot" }));
  await screen.findByRole("heading", { name: "Bot details" });
  expect(view.router.state.location.pathname).toBe("/channel-bots/created-telegram");
});

it("opens the selected channel's full page from the existing dialog", async () => {
  const user = userEvent.setup();
  const view = await setup(`/channel-bots?connect=discord&label=Support&target_org_id=${orgId}`);
  const dialog = within(await screen.findByRole("dialog"));
  await user.type(await dialog.findByLabelText("Bot token"), "private-token");
  await user.click(dialog.getByRole("link", { name: "Open full setup page" }));
  await screen.findByRole("heading", { name: "Create your discord channel bot" });
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(view.router.state.location.search).toEqual({ label: "Support", target_org_id: orgId });
  expect(screen.getByLabelText("Bot token")).toHaveValue("");
});

it("prefills credential fields from a URL, removes secrets from the address, and waits for submission", async () => {
  const user = userEvent.setup();
  post.mockResolvedValue({ id: "prefilled-bot", platform: "lark", status: "active" });
  const query = new URLSearchParams({ label: "Support", app_id: "90071992547409931234", app_secret: "secret+/=&value", verification_token: "verify-token", bot_token: "not-a-lark-field" });
  const view = await setup(`/channel-bots/connect/lark?${query}`);
  await waitFor(() => expect(screen.getByLabelText("app_secret")).toHaveValue("secret+/=&value"));
  expect(screen.getByLabelText("app_id")).toHaveValue("90071992547409931234");
  expect(screen.getByLabelText("verification_token")).toHaveValue("verify-token");
  expect(screen.getByLabelText("app_secret")).toHaveAttribute("type", "password");
  expect(post).not.toHaveBeenCalled();
  await waitFor(() => expect(view.router.state.location.search.app_secret).toBeUndefined());
  expect(view.router.state.location.search.verification_token).toBeUndefined();
  expect(view.history.length).toBe(1);
  const submit = screen.getByRole("button", { name: "Create channel bot" });
  await waitFor(() => expect(submit).toBeEnabled());
  await user.click(submit);
  await waitFor(() => expect(post).toHaveBeenCalledExactlyOnceWith("/channel-bots", {
    platform: "lark", label: "Support", target_org_id: undefined,
    app_id: "90071992547409931234", app_secret: "secret+/=&value", verification_token: "verify-token",
  }));
});

it("prefills future catalog fields without hardcoding query parameter names", async () => {
  const future = platformFixture("future-chat");
  get.mockResolvedValue({ platforms: [{ ...future, registration: { ...future.registration, fields: [
    { ...future.registration.fields[0]!, name: "field1", label: "Workspace", secret: false },
    { ...future.registration.fields[0]!, name: "field2", label: "Credential", secret: true },
  ] } }] });
  const user = userEvent.setup();
  post.mockResolvedValue({ id: "future-bot", platform: "future-chat", status: "active" });
  await setup("/channel-bots/connect/future-chat?label=Future&field1=team&field2=fixture-secret");
  await waitFor(() => expect(screen.getByLabelText("Workspace")).toHaveValue("team"));
  expect(screen.getByLabelText("Credential")).toHaveValue("fixture-secret");
  await user.click(screen.getByRole("button", { name: "Create channel bot" }));
  expect(post).toHaveBeenCalledExactlyOnceWith("/channel-bots", {
    platform: "future-chat", label: "Future", target_org_id: undefined, field1: "team", field2: "fixture-secret",
  });
});

it("validates invalid prefills and allows corrections before registration", async () => {
  const user = userEvent.setup();
  await setup("/channel-bots/connect/whatsapp?label=Support&bot_token=fixture-token&app_secret=fixture-secret&phone_number_id=not-numeric");
  await waitFor(() => expect(screen.getByLabelText("phone_number_id")).toHaveValue("not-numeric"));
  expect(screen.getByRole("button", { name: "Create channel bot" })).toBeDisabled();
  expect(post).not.toHaveBeenCalled();
  await user.clear(screen.getByLabelText("phone_number_id"));
  await user.type(screen.getByLabelText("phone_number_id"), "1234567");
  await waitFor(() => expect(screen.getByRole("button", { name: "Create channel bot" })).toBeEnabled());
});

it("shows prefilled manual credentials alongside an available managed connection", async () => {
  const getDefault = get.getMockImplementation()!;
  get.mockImplementation((path: string) => path.includes("managed-onboarding/whatsapp")
    ? Promise.resolve({ available: true, flow: "oauth_connection", provider_slug: "example" })
    : path === "/channel-platforms"
      ? Promise.resolve({ platforms: [{ ...platformFixtures.find((entry) => entry.platform === "whatsapp")!, managed_onboarding: { flow: "oauth_connection", provider: "example", bootstrap_fields: [], completion_fields: [] } }] })
      : getDefault(path));
  await setup("/channel-bots/connect/whatsapp?label=Support&bot_token=prefilled-token");
  expect(await screen.findByRole("button", { name: "Connect whatsapp account" })).toBeVisible();
  expect(screen.getByLabelText("Bot token")).toHaveValue("prefilled-token");
});
