vi.mock("@/pages/settings", async (importOriginal) => {
  const original = await importOriginal<typeof import("@/pages/settings")>();
  return { ...original, SettingsPage: (props: Parameters<typeof original.SettingsPage>[0]) => <><original.SettingsPage {...props} /><button onClick={() => useCreditsDenialStore.getState().notify({ key: "account-action", payer: "self", actorId: "person" })}>Try account action</button></> };
});
import { ASSISTANT_SHELL_ROUTES, withAccountPanelSearch, validateSettingsSearch } from "@/lib/assistant/account-panel-search";
import { useCreditsDenialStore } from "@/stores/credits-denial-store";
import type { ComponentProps, ReactNode } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  createMemoryHistory,
  createRouter,
  RouterProvider,
} from "@tanstack/react-router";
import {
  act,
  cleanup,
  render,
  screen,
  within,
  waitFor,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { router as appRouter } from "@/router";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";
import {
  billingCatalog,
  billingUsage,
  billingWallet,
} from "@/test/billing-fixture";
import {
  FEATURE_FLAG,
  userHasFeature,
  type FeatureFlag,
} from "@/lib/feature-flags";

const state = vi.hoisted(() => ({
  mock: false,
  mockUser: {} as User,
  logout: vi.fn(),
  walletPending: false,
}));
const person: User = {
  id: "person",
  email: "person@example.com",
  display_name: "Person",
  avatar_url: null,
  email_verified: true,
  mfa_enabled: false,
  is_admin: false,
  is_active: true,
  created_at: "2026-01-01T00:00:00Z",
  capabilities: {
    billing_available: true,
    enabled_features: [FEATURE_FLAG.NYXAGENT_ENGINE],
  },
};
vi.mock("@/lib/mock-data", () => ({
  isMockMode: () => state.mock,
  getMockUser: () => state.mockUser,
}));
const query = (data: unknown) => ({
  data,
  isLoading: false,
  isSuccess: true,
  isFetching: false,
  isError: false,
  refetch: vi.fn(),
});
vi.mock("@/hooks/use-billing", () => ({
  useBillingWallet: () => ({ ...query(state.walletPending ? undefined : billingWallet()), isLoading: state.walletPending }),
  useBillingUsage: () => query(billingUsage()),
  useTopUpHistory: () => query({ topups: [], total: 0 }),
  useProvisionBillingWallet: () => ({ mutateAsync: vi.fn() }),
  useTopUpBilling: () => ({ mutateAsync: vi.fn() }),
}));
vi.mock("@/hooks/use-billing-credits", () => ({
  useActiveCreditGrants: () => query({ grants: [] }),
  useCurrentAllowances: () => query({ allowances: [] }),
}));
vi.mock("@/hooks/use-keys", () => ({
  useCatalog: () => query(billingCatalog),
}));
vi.mock("@/hooks/use-feature-flag", () => ({
  useFeature: (flag: FeatureFlag) =>
    useAuthStore((auth) => userHasFeature(auth.user, flag)),
}));
vi.mock("@/hooks/use-assistant-chat", () => ({
  useAssistantChat: () => ({ visibleConversations: [] }),
}));
vi.mock("@/hooks/use-assistant-direct", () => ({
  useDirectAssistantChat: () => ({ conversations: [] }),
}));
vi.mock("@/hooks/use-assistant-workspace", () => ({
  useAssistantWorkspaceCounts: () => ({
    data: { artifacts: 0, pendingApprovals: 0 },
  }),
}));
vi.mock("@/hooks/use-auth", () => ({
  useLogout: () => ({ mutateAsync: state.logout }),
  useUser: () => query(person),
  useMfaSetup: () => ({ mutateAsync: vi.fn() }),
  useMfaVerifySetup: () => ({ mutateAsync: vi.fn() }),
  useMfaDisable: () => ({ mutateAsync: vi.fn() }),
  useRevokeSession: () => ({ mutateAsync: vi.fn() }),
}));
vi.mock("@/hooks/use-theme", () => ({
  useApplyTheme: vi.fn(),
  useResolvedTheme: () => "dark",
}));
vi.mock("@/components/dashboard/theme-toggle", () => ({
  ThemeToggle: () => null,
}));
vi.mock("@/components/assistant/approvals-view", () => ({ ApprovalsView: () => <h2>Approvals workspace content</h2> }));
vi.mock("@/components/assistant/plugins-view", () => ({ PluginsView: () => <h2>Plugins workspace content</h2> }));
vi.mock("@/components/assistant/assistant-chat-page", async () => {
  const { AssistantShell } = await import("@/components/assistant/assistant-shell");
  const { AssistantSidebar } = await import("@/components/assistant/assistant-sidebar");
  const { NyxBotSettingsButton } = await import("@/components/assistant/nyxbot-settings-dialog");
  const Chat = () => <AssistantShell title="Chat" headerActions={<NyxBotSettingsButton />} sidebar={<div data-testid={`sidebar-${userHasFeature(useAuthStore.getState().user, FEATURE_FLAG.NYXAGENT_ENGINE) ? "nyxagent" : "actor"}`}><AssistantSidebar conversations={[]} activeConversationId={undefined} onNewChat={vi.fn()} onSelect={vi.fn()} onDelete={vi.fn()} /></div>}><h2>Chat background</h2></AssistantShell>;
  return { AssistantChatPage: Chat, DirectAssistantChatPage: Chat, NyxAgentAssistantChatPage: Chat };
});
vi.mock("@/components/assistant/assistant-wire-log-panel", () => ({
  AssistantWireLogAction: () => null,
}));
vi.mock("@/components/assistant/mock-scenarios-action", () => ({
  MockScenariosAction: () => null,
}));
vi.mock("@/components/assistant/assistant-http-fixture-page", async () => {
  const { NyxAgentAssistantChatPage } = await import("@/components/assistant/assistant-chat-page");
  return { AssistantHttpFixtureBoundary: ({ children }: { children: ReactNode }) => children, AssistantHttpFixturePage: NyxAgentAssistantChatPage };
});
vi.mock("@/components/assistant/assistant-engine-sidebar", async () => {
  const { AssistantSidebar } =
    await import("@/components/assistant/assistant-sidebar");
  return {
    AssistantEngineSidebar: ({
      engine,
      ...props
    }: ComponentProps<typeof AssistantSidebar> & { engine: string }) => (
      <div data-testid={`sidebar-${engine}`}>
        <AssistantSidebar {...props} />
      </div>
    ),
  };
});
vi.mock("@/pages/automations", () => ({
  AutomationsPage: () => <h2>Automation workspace content</h2>,
}));
vi.mock("@/pages/machines", () => ({
  MachinesPage: () => <h2>Machine workspace content</h2>,
}));
vi.mock("@/pages/machine-setup", () => ({
  MachineSetupPage: () => <h2>Setup workspace content</h2>,
  MachinePairPage: () => <h2>Pairing workspace content</h2>,
}));
vi.mock("@/pages/machine-desktop", () => ({
  MachineDesktopPage: () => <h2>Standalone desktop content</h2>,
}));
function setEngine(enabled: boolean) {
  useAuthStore.setState({
    user: {
      ...person,
      capabilities: {
        ...person.capabilities,
        enabled_features: enabled ? [FEATURE_FLAG.NYXAGENT_ENGINE] : [],
      },
    },
  });
}

const originalAuth = useAuthStore.getState();
beforeEach(() => {
  state.mock = false;
  state.walletPending = false;
  useCreditsDenialStore.getState().reset();
  vi.stubGlobal("fetch", vi.fn(async (input: RequestInfo | URL) => {
    const path = new URL(input instanceof Request ? input.url : String(input), "http://localhost").pathname;
    const fixtures: Record<string, unknown> = { "/api/v1/public/config": { mcp_url: "https://nyxid.test/mcp" }, "/api/v1/sessions": [], "/api/v1/orgs": { orgs: [] } };
    if (!(path in fixtures)) throw new Error(`Unexpected request: ${path}`);
    return new Response(JSON.stringify(fixtures[path]));
  }));
  state.mockUser = person;
  state.logout.mockReset();
  useAuthStore.setState({
    isAuthenticated: true,
    isLoading: false,
    user: person,
  });
});
afterEach(() => {
  cleanup();
  useAuthStore.setState(originalAuth);
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

async function open(path: string) {
  const history = createMemoryHistory({ initialEntries: [path] });
  const router = createRouter({ routeTree: appRouter.routeTree, history });
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  render(
    <QueryClientProvider client={client}>
      <RouterProvider router={router} />
    </QueryClientProvider>,
  );
  await act(() => router.load());
  await act(() => vi.dynamicImportSettled());
  return router;
}

for (const nyxagent of [true, false]) {
  it.each([
    ["automations", "Automation", "Automations"],
    ["machines", "Machine", "Machines"],
    ["machines/new", "Setup", "Machines"],
    ["machines/pair?code=ABCD-EFGH", "Pairing", "Machines"],
    ["machines?tab=logins", "Machine", "Machines"],
  ])(
    `renders %s in the ${nyxagent ? "NyxAgent" : "legacy"} assistant shell and mobile drawer`,
    async (path, content, active) => {
      setEngine(nyxagent);
      await open(`/assistant/${path}`);
      expect(
        await screen.findByRole("heading", {
          name: `${content} workspace content`,
        }),
      ).toBeInTheDocument();
      const sidebar = screen.getByTestId(
        `sidebar-${nyxagent ? "nyxagent" : "actor"}`,
      );
      const expected = [
        "Home",
        "Automations",
        "Machines",
        "Plugins",
        "Artifacts",
        "Approvals",
        "Activity",
      ];
      const labels = expected.map((label) => within(sidebar).getByText(label));
      for (let index = 1; index < labels.length; index++) {
        expect(labels[index - 1]!.compareDocumentPosition(labels[index]!)).toBe(
          Node.DOCUMENT_POSITION_FOLLOWING,
        );
      }
      expect(
        within(sidebar).getByRole("link", { name: active }),
      ).toHaveAttribute("aria-current", "page");
      expect(
        within(sidebar).queryByText("Devices & Nodes"),
      ).not.toBeInTheDocument();
      await userEvent.click(screen.getByRole("button", { name: "Open chats" }));
      const sidebars = screen.getAllByTestId(
        `sidebar-${nyxagent ? "nyxagent" : "actor"}`,
      );
      expect(sidebars).toHaveLength(2);
      expect(
        within(sidebars[1]!).getByRole("link", { name: active }),
      ).toHaveAttribute("aria-current", "page");
      await userEvent.click(
        within(sidebars[1]!).getByRole("link", { name: active }),
      );
      expect(
        screen.queryByRole("button", { name: "Close chats" }),
      ).not.toBeInTheDocument();
    },
  );
}

it("redirects the shipped automation URL preserving validated setup and agent", async () => {
  const setup = "11111111-1111-4111-8111-111111111111";
  const router = await open(
    `/automations?setup=${setup}&agent=coder&token=discard`,
  );
  expect(
    await screen.findByRole("heading", {
      name: "Automation workspace content",
    }),
  ).toBeInTheDocument();
  expect(router.state.location.pathname).toBe("/assistant/automations");
  expect(router.state.location.search).toEqual({ setup, agent: "coder" });
});

it("keeps desktop standalone and all assistant workspace routes under the shared human-session guard", async () => {
  await open(
    "/assistant/machines/node/desktop?conversation_id=nyxagent%3Athread",
  );
  expect(
    await screen.findByRole("heading", { name: "Standalone desktop content" }),
  ).toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Open chats" }),
  ).not.toBeInTheDocument();
  const routes = appRouter.routesByPath;
  for (const path of [
    "/assistant/automations",
    "/assistant/machines",
    "/assistant/machines/new",
    "/assistant/machines/pair",
    "/assistant/machines/$nodeId/desktop",
  ] as const) {
    expect(routes[path].options.beforeLoad).toBe(
      routes["/assistant/approvals"].options.beforeLoad,
    );
    expect(routes[path].options.validateSearch).toBeTypeOf("function");
    expect(routes[path].parentRoute.id).toBe("__root__");
  }
});

for (const nyxagent of [true, false]) {
  it.each([
    ["settings", "privacy", "Account Settings", "Privacy"],
    ["billing", "usage", "Billing & Usage", "Usage"],
  ])(`opens %s over the ${nyxagent ? "NyxAgent" : "legacy"} view`, async (panel, panelTab, title, tab) => {
    setEngine(nyxagent);
    const router = await open(`/assistant/plugins?panel=${panel}&panelTab=${panelTab}`);
    const dialog = await screen.findByRole("dialog", { name: title });
    expect(within(dialog).getByRole("tab", { name: tab })).toHaveAttribute("data-state", "active");
    expect(screen.getByRole("heading", { name: "Plugins workspace content", hidden: true })).toBeInTheDocument();
    expect(screen.getByTestId(`sidebar-${nyxagent ? "nyxagent" : "actor"}`)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Account menu", hidden: true })).toHaveClass("bg-overlay-strong");
    expect(router.state.location.pathname).toBe("/assistant/plugins");
  });
}

it.each([false, undefined])("strips unavailable billing capability %s with replace", async (billing_available) => {
  useAuthStore.setState({ user: { ...person, capabilities: { billing_available } } });
  const router = await open("/assistant?c=chat&panel=billing&panelTab=usage");
  await waitFor(() => expect(router.state.location.search).toEqual({ c: "chat" }));
  expect(router.history.length).toBe(1);
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});

it.each(["settings", "billing"])("preserves a signed-out %s panel for login return_to", async (panel) => {
  useAuthStore.setState({ user: null, isAuthenticated: false, isLoading: false });
  const assign = vi.spyOn(window.location, "assign").mockImplementation(() => undefined);
  const path = `/assistant?panel=${panel}&panelTab=security`;
  const router = await open(path);
  expect(assign).toHaveBeenCalledWith(`/login?return_to=${encodeURIComponent(`${window.location.origin}${path}`)}`);
  expect(router.state.location.searchStr).toBe(path.slice(path.indexOf("?")));
  expect(screen.getByRole("button", { name: "Open chats" })).toBeInTheDocument();
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});

it("retains DEV authentication and composes shared validators", async () => {
  state.mock = true;
  state.mockUser = { ...person, capabilities: { billing_available: true, enabled_features: [] } };
  useAuthStore.setState({ user: null, isAuthenticated: false, isLoading: false });
  const router = await open("/assistant?mock=1&panel=billing&panelTab=usage&panelPeriod=7d&panelServices=%5B%22example-llm%22%5D");
  expect(await screen.findByRole("tab", { name: "Usage" })).toHaveAttribute("data-state", "active");
  expect(useAuthStore.getState().isAuthenticated).toBe(true);
  expect(router.state.location.search).toMatchObject({ mock: 1, panel: "billing", panelTab: "usage", panelPeriod: "7d", panelServices: ["example-llm"] });
  expect(appRouter.routesByPath["/settings"].options.validateSearch).toBe(validateSettingsSearch);
  for (const path of ASSISTANT_SHELL_ROUTES) {
    expect(appRouter.routesByPath[path].parentRoute.id).toBe("__root__");
    expect(appRouter.routesByPath[path].options.beforeLoad).toBe(appRouter.routesByPath["/assistant/approvals"].options.beforeLoad);
    expect(appRouter.routesByPath[path].options.validateSearch).toBeTypeOf("function");
  }
  expect(withAccountPanelSearch(validateSettingsSearch)({ panel: "settings", panelTab: "privacy", tab: "route-tab" })).toEqual({ tab: "route-tab", panel: "settings", panelTab: "privacy" });
});

async function drawerAccountMenu() {
  await userEvent.click(screen.getByRole("button", { name: "Open chats" }));
  const sidebar = screen.getAllByTestId("sidebar-nyxagent")[1]!;
  await userEvent.click(within(sidebar).getByRole("button", { name: "Account menu" }));
  return screen.getByRole("menu");
}

it.each(["click", "keyboard"])("dismisses the drawer when opening a panel by %s", async (activation) => {
  const router = await open("/assistant/plugins");
  const menu = await drawerAccountMenu();
  const item = within(menu).getByRole("menuitem", { name: "Settings" });
  if (activation === "click") await userEvent.click(item);
  else { item.focus(); await userEvent.keyboard("{Enter}"); }
  expect(await screen.findByRole("dialog", { name: "Account Settings" })).toBeVisible();
  expect(screen.queryByRole("menu")).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Close chats", hidden: true })).not.toBeInTheDocument();
  expect(router.state.location.pathname).toBe("/assistant/plugins");
  await userEvent.keyboard("{Escape}");
  await waitFor(() => expect(screen.getByRole("button", { name: "User menu" })).toHaveFocus());
});

it.each(["Control", "Meta", "Shift", "Alt"])("keeps the drawer on %s-modified Studio navigation", async (modifier) => {
  const router = await open("/assistant/plugins");
  const menu = await drawerAccountMenu();
  const user = userEvent.setup();
  await user.keyboard(`{${modifier}>}`);
  await user.click(within(menu).getByRole("menuitem", { name: "Open Studio" }));
  await user.keyboard(`{/${modifier}}`);
  expect(screen.getAllByRole("button", { name: "Close chats" })).toHaveLength(2);
  expect(router.state.location.pathname).toBe("/assistant/plugins");
});

it("lets the menu consume Escape before the drawer and hands account focus to the header", async () => {
  await open("/assistant/plugins");
  await drawerAccountMenu();
  await userEvent.keyboard("{Escape}");
  expect(screen.queryByRole("menu")).not.toBeInTheDocument();
  expect(screen.getAllByRole("button", { name: "Close chats" })).toHaveLength(2);
  const trigger = within(screen.getAllByTestId("sidebar-nyxagent")[1]!).getByRole("button", { name: "Account menu" });
  await waitFor(() => expect(trigger).toHaveFocus());
  await userEvent.click(trigger);
  await userEvent.click(screen.getByRole("menuitem", { name: "Settings" }));
  expect(await screen.findByRole("dialog", { name: "Account Settings" })).toBeVisible();
  expect(screen.queryByRole("button", { name: "Close chats", hidden: true })).not.toBeInTheDocument();
  await userEvent.keyboard("{Escape}");
  await waitFor(() => expect(screen.getByRole("button", { name: "User menu" })).toHaveFocus());
});

it.each(["/assistant?panel=settings&panelTab=display", "/assistant?panel=billing&panelTab=usage", "/assistant/machines"])("dismisses the drawer on location change to %s", async (to) => {
  const router = await open("/assistant/plugins");
  await userEvent.click(screen.getByRole("button", { name: "Open chats" }));
  await act(() => router.navigate({ to }));
  await waitFor(() => expect(screen.queryByRole("button", { name: "Close chats", hidden: true })).not.toBeInTheDocument());
});

it.each([true, false])("shares labels, gating, Studio destinations and logout between menus (engine %s)", async (nyxagent) => {
  setEngine(nyxagent);
  await open("/assistant/plugins");
  await userEvent.click(screen.getByRole("button", { name: "Account menu" }));
  const items = () => screen.getAllByRole("menuitem").map((item) => ({ text: item.textContent, href: item.getAttribute("href") }));
  const sidebarItems = items();
  expect(sidebarItems.some((item) => item.text === "NyxBot settings")).toBe(false);
  await userEvent.keyboard("{Escape}");
  await userEvent.click(screen.getByRole("button", { name: "User menu" }));
  expect(items()).toEqual(sidebarItems);
  expect(screen.getByRole("menuitem", { name: "Notification settings (opens in Studio)" })).toHaveAttribute("href", "/approvals/settings");
  expect(screen.getByRole("menuitem", { name: "Open Studio" })).toHaveAttribute("href", "/dashboard");
  await userEvent.click(screen.getByRole("menuitem", { name: "Log out" }));
  expect(state.logout).toHaveBeenCalledOnce();
});

it("hides Billing and Usage in both menus when unavailable", async () => {
  useAuthStore.setState({ user: { ...person, capabilities: {} } });
  await open("/assistant/plugins");
  for (const trigger of ["Account menu", "User menu"]) {
    await userEvent.click(screen.getByRole("button", { name: trigger }));
    expect(screen.queryByRole("menuitem", { name: "Billing" })).not.toBeInTheDocument();
    expect(screen.queryByRole("menuitem", { name: "Usage" })).not.toBeInTheDocument();
    await userEvent.keyboard("{Escape}");
  }
});

it.each(["unknown", "%5B%22security%22%5D"])("uses the Settings Profile fallback for panelTab=%s", async (tab) => {
  await open(`/assistant?panel=settings&panelTab=${tab}`);
  expect(await screen.findByRole("tab", { name: "Profile" })).toHaveAttribute("data-state", "active");
});

it.each(["Account menu", "User menu"])("restores focus to %s after closing", async (name) => {
  const router = await open("/assistant/plugins");
  const trigger = screen.getByRole("button", { name });
  await userEvent.click(trigger);
  await userEvent.click(screen.getByRole("menuitem", { name: "Settings" }));
  const dialog = await screen.findByRole("dialog", { name: "Account Settings" });
  expect(screen.getByRole("button", { name: "Account menu", hidden: true })).toHaveClass("bg-overlay-strong");
  expect(router.state.location.pathname).toBe("/assistant/plugins");
  expect(document.title).toBe("nyxid - Plugins");
  await userEvent.click(within(dialog).getByRole("button", { name: "Close" }));
  await waitFor(() => expect(trigger).toHaveFocus());
  expect(router.state.location.search).toEqual({});
  await userEvent.click(screen.getByRole("button", { name: "User menu" }));
  expect(screen.getByRole("menu")).toBeVisible();
});

it.each(["settings", "billing"])("requires complete auth readiness for %s", async (panel) => {
  useAuthStore.setState({ user: null, isAuthenticated: true, isLoading: false });
  const router = await open(`/assistant?panel=${panel}`);
  expect(router.state.location.search).toMatchObject({ panel });
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  act(() => setEngine(true));
  expect(await screen.findByRole("dialog")).toBeVisible();
});

it.each(["billing"])("replaces rejected %s after hydration/refresh and live capability change", async (panel) => {
  for (const retainedUser of [false, true]) {
    useAuthStore.setState({ user: retainedUser ? person : null, isAuthenticated: retainedUser, isLoading: true });
    const router = await open(`/assistant?panel=${panel}`);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(router.state.location.search).toMatchObject({ panel });
    act(() => useAuthStore.setState({ user: person, isAuthenticated: true, isLoading: false }));
    expect(await screen.findByRole("dialog")).toBeVisible();
    act(() => useAuthStore.setState({ user: { ...person, capabilities: {} } }));
    await waitFor(() => expect(router.state.location.search).toEqual({}));
    expect(router.history.length).toBe(1);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    cleanup();
  }
});

it.each(["/assistant/settings", "/assistant/settings/nyxbot", "/assistant/billing"])("returns unknown-path 404 for removed %s", async (path) => {
  const router = await open(path);
  expect(router.state.location.pathname).toBe(path);
  expect(router.state.matches.some((match) => match.globalNotFound)).toBe(true);
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});

it.each([
  ["/assistant", "c=chat&draft=true&agent=a&g=group&mock=1"],
  ["/assistant/plugins", "mock=1"],
  ["/assistant/approvals", "mock=1"],
  ["/assistant/automations", "agent=a&setup=s&mock=1"],
  ["/assistant/machines", "tab=logins&machine=n&mock=1"],
  ["/assistant/machines/new", "setup=s&mock=1"],
  ["/assistant/machines/pair", "code=ABCD-EFGH&mock=1"],
])("preserves %s keys through open/update/close/Back", async (path, keys) => {
  const router = await open(`${path}?${keys}`);
  const initial = router.state.location.search;
  await userEvent.click(screen.getByRole("button", { name: "User menu" }));
  await userEvent.click(screen.getByRole("menuitem", { name: "Settings" }));
  const dialog = await screen.findByRole("dialog", { name: "Account Settings" });
  expect(router.state.location.search).toMatchObject(initial);
  await userEvent.click(within(dialog).getByRole("tab", { name: "Security" }));
  await waitFor(() => expect(router.state.location.search).toMatchObject({ ...initial, panel: "settings", panelTab: "security" }));
  expect(router.history.length).toBe(2);
  await act(() => router.history.back());
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  expect(router.state.location.search).toEqual(initial);
  await act(() => router.history.forward());
  await screen.findByRole("dialog", { name: "Account Settings" });
  await userEvent.keyboard("{Escape}");
  await waitFor(() => expect(router.state.location.search).toEqual(initial));
  expect(router.history.length).toBe(3);
  await act(() => router.history.back());
  expect(await screen.findByRole("dialog", { name: "Account Settings" })).toBeVisible();
});

it("consumes pending top-up before close so Back cannot replay after wallet resolution", async () => {
  state.walletPending = true;
  const router = await open("/assistant?c=chat&panel=billing&panelAction=topup");
  await screen.findByRole("dialog", { name: "Billing & Usage" });
  await userEvent.keyboard("{Escape}");
  await waitFor(() => expect(router.state.location.search).toEqual({ c: "chat" }));
  act(() => { state.walletPending = false; });
  await act(() => router.history.back());
  expect(await screen.findByRole("dialog", { name: "Billing & Usage" })).toBeVisible();
  expect(router.state.location.search).toEqual({ c: "chat", panel: "billing" });
  expect(screen.queryByRole("dialog", { name: "Add credits" })).not.toBeInTheDocument();
});

it.each(["panel-first", "credits-first", "refresh"])("keeps credits visually and logically topmost (%s)", async (order) => {
  const router = await open(order === "credits-first" ? "/assistant/plugins" : "/assistant/plugins?panel=settings");
  if (order !== "credits-first") await screen.findByRole("dialog", { name: "Account Settings" });
  act(() => useCreditsDenialStore.getState().notify({ key: "denial", payer: "self", actorId: "person" }));
  const credits = await screen.findByRole("dialog", { name: "Not enough credits to continue" });
  if (order === "credits-first") await act(() => router.navigate({ to: "/assistant/plugins", search: { panel: "settings" } }));
  if (order === "refresh") { act(() => useAuthStore.setState({ isLoading: true })); act(() => useAuthStore.setState({ isLoading: false })); }
  expect(credits).toHaveStyle({ zIndex: "120" });
  await userEvent.keyboard("{Tab}");
  expect(credits.contains(document.activeElement)).toBe(true);
  await userEvent.keyboard("{Escape}");
  await waitFor(() => expect(screen.queryByRole("dialog", { name: "Not enough credits to continue" })).not.toBeInTheDocument());
  const panel = await screen.findByRole("dialog", { name: "Account Settings" });
  expect(panel).toHaveStyle({ zIndex: "90" });
  await userEvent.click(within(panel).getByRole("button", { name: "Close" }));
  await waitFor(() => expect(router.state.location.search).toEqual({}));
});

it.each(["dismiss", "purchase"])("hands credits focus back after an account action (%s)", async (action) => {
  await open("/assistant/plugins?panel=settings");
  const nyxbot = await screen.findByRole("dialog", { name: "Account Settings" });
  const trigger = within(nyxbot).getByRole("button", { name: "Try account action" });
  await userEvent.click(trigger);
  const denial = await screen.findByRole("dialog", { name: "Not enough credits to continue" });
  if (action === "dismiss") {
    await userEvent.click(within(denial).getByRole("button", { name: "Not now" }));
    await waitFor(() => expect(trigger).toHaveFocus());
  } else {
    await userEvent.click(within(denial).getByRole("button", { name: "Purchase credits" }));
    const topup = await screen.findByRole("dialog", { name: "Add credits" });
    expect(topup).toHaveStyle({ zIndex: "100" });
    await userEvent.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Add credits" })).not.toBeInTheDocument());
    expect(screen.getByRole("dialog", { name: "Billing & Usage" })).toBeVisible();
    await userEvent.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    await userEvent.click(screen.getByRole("button", { name: "User menu" }));
    expect(screen.getByRole("menu")).toBeVisible();
  }
});

it.each(["settings", "billing"])("preserves signed-out %s deep links in login return_to", async (panel) => {
  useAuthStore.setState({ user: null, isAuthenticated: false, isLoading: false });
  const assign = vi.spyOn(window.location, "assign").mockImplementation(() => undefined);
  const path = `/assistant?panel=${panel}`;
  const router = await open(path);
  expect(assign).toHaveBeenCalledWith(`/login?return_to=${encodeURIComponent(`${window.location.origin}${path}`)}`);
  expect(router.state.location.search).toMatchObject({ panel });
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});
