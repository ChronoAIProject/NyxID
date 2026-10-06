import type { ComponentProps, ReactNode } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  createMemoryHistory,
  createRouter,
  RouterProvider,
} from "@tanstack/react-router";
import { act, cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { router as appRouter } from "@/router";
import { useAuthStore } from "@/stores/auth-store";
import { FEATURE_FLAG } from "@/lib/feature-flags";

const state = vi.hoisted(() => ({ nyxagent: true }));
vi.mock("@/hooks/use-feature-flag", () => ({
  useFeature: (flag: string) =>
    flag === FEATURE_FLAG.NYXAGENT_ENGINE && state.nyxagent,
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
  useLogout: () => ({ mutateAsync: vi.fn() }),
}));
vi.mock("@/hooks/use-theme", () => ({
  useApplyTheme: vi.fn(),
  useResolvedTheme: () => "dark",
}));
vi.mock("@/components/dashboard/theme-toggle", () => ({
  ThemeToggle: () => null,
}));
vi.mock("@/components/assistant/nyxbot-settings-dialog", () => ({
  NyxBotSettingsButton: () => null,
}));
vi.mock("@/components/assistant/assistant-wire-log-panel", () => ({
  AssistantWireLogAction: () => null,
}));
vi.mock("@/components/assistant/mock-scenarios-action", () => ({
  MockScenariosAction: () => null,
}));
vi.mock("@/components/assistant/assistant-http-fixture-page", () => ({
  AssistantHttpFixtureBoundary: ({ children }: { children: ReactNode }) =>
    children,
}));
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
const originalAuth = useAuthStore.getState();
beforeEach(() =>
  useAuthStore.setState({ isAuthenticated: true, isLoading: false }),
);
afterEach(() => {
  cleanup();
  useAuthStore.setState(originalAuth);
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
      state.nyxagent = nyxagent;
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
