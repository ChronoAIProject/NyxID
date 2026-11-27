import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  RouterProvider,
} from "@tanstack/react-router";
import { afterEach, expect, it, vi } from "vitest";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";
import { DashboardLayout } from "./dashboard-layout";

const { mockOnboarding } = vi.hoisted(() => ({ mockOnboarding: vi.fn() }));

vi.mock("@/hooks/use-auth", () => ({
  useUser: () => ({ data: undefined }),
  useLogout: () => ({ mutateAsync: vi.fn() }),
}));
vi.mock("@/hooks/use-onboarding", () => ({
  useShouldShowOnboarding: () => mockOnboarding() ?? { status: "hidden" },
  useCompleteOnboarding: () => ({ isPending: false }),
}));
vi.mock("@/components/dashboard/add-key-dialog", () => ({
  AddKeyDialog: () => null,
}));
vi.mock("@/components/auth/mfa-setup-dialog", () => ({
  MfaSetupDialog: ({ onOpenChange }: { onOpenChange: (open: boolean) => void }) => (
    <div role="dialog" aria-label="MFA enrollment">
      <button onClick={() => onOpenChange(false)}>Done</button>
    </div>
  ),
}));
vi.mock("@/hooks/use-theme", () => ({
  useApplyTheme: vi.fn(),
  useResolvedTheme: () => "dark",
}));
vi.mock("@/components/chrome/ambient-status-line", () => ({
  AmbientStatusLine: () => null,
}));
vi.mock("@/components/dashboard/theme-toggle", () => ({
  ThemeToggle: () => null,
}));
vi.mock("@/components/navigation/command-palette", () => ({
  CommandPalette: () => null,
  ALL_ITEMS: [],
}));
vi.mock("@/components/dashboard/sidebar", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/components/dashboard/sidebar")>()),
  Sidebar: () => null,
  AssistantNavEntry: () => null,
}));

function account(overrides: Partial<User>): User {
  return {
    id: "mobile-user",
    display_name: "Local user",
    email: "local@example.test",
    is_admin: false,
    role: "user",
    ...overrides,
  } as User;
}

async function openDashboard(user: User) {
  useAuthStore.setState({ user });
  const root = createRootRoute({ component: DashboardLayout });
  const page = createRoute({ getParentRoute: () => root, path: "/dashboard" });
  const router = createRouter({
    routeTree: root.addChildren([page]),
    history: createMemoryHistory({ initialEntries: ["/dashboard"] }),
  });
  await router.load();
  render(<RouterProvider router={router} />);
}

async function openMobileMenu(user: User) {
  await openDashboard(user);
  await userEvent.click(
    await screen.findByRole("button", { name: "Open menu" }),
  );
  return within(screen.getByRole("navigation", { name: "" }));
}

afterEach(() => {
  useAuthStore.setState({ user: null });
  mockOnboarding.mockReset();
});

it.each([
  ["admin", account({ role: "admin", mfa_enabled: false }), true],
  ["operator", account({ role: "operator", mfa_enabled: false }), true],
  ["usage viewer", account({ capabilities: { read_admin_usage: true }, mfa_enabled: false }), true],
  ["enrolled admin", account({ role: "admin", mfa_enabled: true }), false],
  ["ordinary user", account({ capabilities: { billing_available: true }, mfa_enabled: false }), false],
] as const)("shows the MFA enrollment banner for eligible accounts only (%s)", async (_, user, visible) => {
  await openDashboard(user);
  await screen.findByRole("button", { name: "Open menu" });
  expect(Boolean(screen.queryByRole("button", { name: "Set up MFA" }))).toBe(visible);
});

it("keeps enrollment open after the profile refreshes so the admin can save recovery codes", async () => {
  const user = account({ role: "admin", mfa_enabled: false });
  await openDashboard(user);
  await userEvent.click(await screen.findByRole("button", { name: "Set up MFA" }));
  act(() => useAuthStore.setState({ user: { ...user, mfa_enabled: true } }));

  expect(screen.queryByRole("button", { name: "Set up MFA" })).toBeNull();
  expect(screen.getByRole("dialog", { name: "MFA enrollment" })).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Done" }));
  expect(screen.queryByRole("dialog", { name: "MFA enrollment" })).toBeNull();
});

it("closes enrollment if the signed-in account changes", async () => {
  await openDashboard(account({ role: "admin", mfa_enabled: false }));
  await userEvent.click(await screen.findByRole("button", { name: "Set up MFA" }));
  act(() => useAuthStore.setState({
    user: account({ id: "other-admin", role: "admin", mfa_enabled: false }),
  }));

  expect(screen.queryByRole("dialog", { name: "MFA enrollment" })).toBeNull();
  expect(screen.getByRole("button", { name: "Set up MFA" })).toBeInTheDocument();
});

it("offers MFA enrollment during first-run onboarding", async () => {
  mockOnboarding.mockReturnValue({ status: "show" });
  await openDashboard(account({ role: "admin", mfa_enabled: false }));
  expect(await screen.findByRole("heading", { name: "Welcome, Local user" })).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Set up MFA" }));
  expect(screen.getByRole("dialog", { name: "MFA enrollment" })).toBeInTheDocument();
});

it.each([
  ["ordinary user", account({}), false],
  ["Operator", account({ role: "operator" }), false],
  [
    "Catalog Manager",
    account({ role: "operator", capabilities: { manage_catalog: true } }),
    false,
  ],
  [
    "Tools Manager",
    account({ role: "operator", capabilities: { manage_tools: true } }),
    true,
  ],
  [
    "both roles",
    account({
      role: "operator",
      capabilities: { manage_tools: true, manage_catalog: true },
    }),
    true,
  ],
  ["Admin fallback", account({ is_admin: true, role: "admin" }), true],
] as const)(
  "renders mobile Tool management access for %s",
  async (_, user, allowed) => {
    const menu = await openMobileMenu(user);
    const toolLinks = menu.queryAllByRole("link", { name: "Tools" });
    const managementLink = toolLinks.find(
      (link) => link.getAttribute("href") === "/admin/tools",
    );
    expect(Boolean(managementLink)).toBe(allowed);
    expect(
      toolLinks.some((link) => link.getAttribute("href") === "/tools"),
    ).toBe(true);
  },
);

it("removes the management link from an already open mobile menu after revocation", async () => {
  const user = account({
    role: "operator",
    capabilities: { manage_tools: true },
  });
  const menu = await openMobileMenu(user);
  expect(menu.getAllByRole("link", { name: "Tools" })).toHaveLength(2);
  act(() => {
    useAuthStore.setState({
      user: { ...user, capabilities: { manage_tools: false } },
    });
  });
  expect(menu.getAllByRole("link", { name: "Tools" })).toHaveLength(1);
  expect(menu.getByRole("link", { name: "Tools" })).toHaveAttribute(
    "href",
    "/tools",
  );
});
