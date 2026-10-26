import { withAccountPanelSearch, validateSettingsSearch } from "@/lib/assistant/account-panel-search";
import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  RouterProvider,
} from "@tanstack/react-router";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { TooltipProvider } from "@/components/ui/tooltip";
import { SETTINGS_TABS } from "@/lib/url-tabs";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";
import { SettingsPage } from "./settings";

vi.mock("@/hooks/use-auth", () => ({
  useUser: () => ({
    data: {
      display_name: "Reader",
      email: "reader@example.com",
      email_verified: true,
    },
  }),
  useMfaDisable: () => ({ mutateAsync: vi.fn() }),
  useRevokeSession: () => ({ mutateAsync: vi.fn() }),
}));
vi.mock("@/hooks/use-public-config", () => ({
  usePublicConfig: () => ({ data: { mcp_url: "https://nyxid.test/mcp" } }),
}));
vi.mock("@/components/auth/mfa-setup-dialog", () => ({
  MfaSetupDialog: () => null,
}));

const initialAuth = useAuthStore.getState();
const transport = vi.fn<typeof fetch>();
const unexpectedRequests: string[] = [];
beforeEach(() => {
  useAuthStore.setState({ user: { id: "owner", email: "reader@example.com" } as User });
  unexpectedRequests.length = 0;
  transport.mockReset().mockImplementation(async (url, options) => {
    const headers = { "Content-Type": "application/json" };
    if (url === "/api/v1/onboarding/bindings" && options?.method === "GET") {
      return new Response("[]", { headers });
    }
    if (url === "/api/v1/users/me" && options?.method === "DELETE") {
      return new Response(JSON.stringify({
        error: "validation_error", error_code: 1000, message: "Fixture denial after request capture",
      }), { status: 400, headers });
    }
    const request = `${options?.method ?? "GET"} ${String(url)}`;
    unexpectedRequests.push(request);
    throw new Error(`Unexpected request: ${request}`);
  });
  vi.stubGlobal("fetch", transport);
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  useAuthStore.setState(initialAuth);
  expect(unexpectedRequests).toEqual([]);
});

async function open(
  route: "/settings" | "/assistant",
  tab = "profile",
) {
  const root = createRootRoute();
  const page = createRoute({
    path: route,
    getParentRoute: () => root,
    validateSearch: route === "/settings" ? validateSettingsSearch : withAccountPanelSearch((search) => ({ c: search.c, mock: search.mock })),
    component: () =>
      route === "/settings" ? <SettingsPage /> : <SettingsPage presentation="panel" />,
  });
  const history = createMemoryHistory({
    initialEntries: [route === "/settings" ? `${route}?tab=${tab}` : `${route}?c=chat&mock=1&panel=settings&panelTab=${tab}`],
  });
  const router = createRouter({ routeTree: root.addChildren([page]), history });
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: Infinity } },
  });
  client.setQueryData(["sessions"], []);
  render(
    <QueryClientProvider client={client}>
      <TooltipProvider>
        <RouterProvider router={router} />
      </TooltipProvider>
    </QueryClientProvider>,
  );
  await act(() => router.load());
  await screen.findByRole("tab", { name: "Profile" });
  if (tab === "security") await screen.findByText(/No active onboarding bindings/);
  return { router, history };
}

describe.each(["/settings", "/assistant"] as const)(
  "SettingsPage at %s",
  (route) => {
    it.each([400, 429])("shows password-change %i errors without signing out", async (status) => {
      useAuthStore.setState({
        user: { ...useAuthStore.getState().user!, has_password: true },
        isAuthenticated: true,
      });
      const original = transport.getMockImplementation()!;
      transport.mockImplementation(async (url, options) => {
        if (url === "/api/v1/auth/password/change") {
          return new Response(JSON.stringify({
            error: status === 400 ? "validation_error" : "rate_limited",
            error_code: status === 400 ? 1008 : 1005,
            message: status === 400 ? "Validation error: Current password is incorrect" : "Rate limited",
          }), { status, headers: { "Content-Type": "application/json" } });
        }
        return original(url, options);
      });
      await open(route, "security");
      await userEvent.type(screen.getByLabelText("Current Password"), "old");
      await userEvent.type(screen.getByLabelText("New Password", { exact: true }), "abcdefé");
      await userEvent.type(screen.getByLabelText("Confirm New Password"), "abcdefé");
      await userEvent.click(screen.getByRole("button", { name: "Change Password" }));
      expect(await screen.findByRole("alert")).toHaveTextContent(status === 400 ? "Current password is incorrect" : "Rate limited");
      expect(useAuthStore.getState().isAuthenticated).toBe(true);
      expect(useAuthStore.getState().user?.id).toBe("owner");
      expect(transport).toHaveBeenCalledWith("/api/v1/auth/password/change", expect.objectContaining({
        method: "POST", body: JSON.stringify({ current_password: "old", new_password: "abcdefé" }),
      }));
    });

    it("submits same-as-current passwords and clears the fields on success", async () => {
      useAuthStore.setState({ user: { ...useAuthStore.getState().user!, has_password: true } });
      const original = transport.getMockImplementation()!;
      transport.mockImplementation(async (url, options) => url === "/api/v1/auth/password/change"
        ? new Response(JSON.stringify({ message: "Password changed successfully" }), { status: 200 })
        : original(url, options));
      await open(route, "security");
      for (const label of ["Current Password", "New Password", "Confirm New Password"]) {
        await userEvent.type(screen.getByLabelText(label, { exact: true }), "samepassword");
      }
      await userEvent.click(screen.getByRole("button", { name: "Change Password" }));
      await waitFor(() => expect(screen.getByLabelText("Current Password")).toHaveValue(""));
      expect(screen.getByLabelText("New Password", { exact: true })).toHaveValue("");
      expect(screen.getByRole("button", { name: "Change Password" })).toBeDisabled();
    });

    it.each([false, true])(
      "shows QR MFA verification as automatic when TOTP is %s",
      async (mfa_enabled) => {
        useAuthStore.setState({
          user: { ...useAuthStore.getState().user!, mfa_enabled },
        });
        await open(route, "security");
        expect(
          screen.getByText("Skip MFA for NyxID app sign-ins"),
        ).toBeInTheDocument();
        expect(screen.getByText("Always enabled")).toBeInTheDocument();
        expect(
          screen.getByRole("switch", {
            name: "Toggle two-factor authentication",
          }),
        ).toHaveAttribute("aria-checked", String(mfa_enabled));
      },
    );

    it.each([
      { social_provider: null, has_password: false },
      { social_provider: "google", has_password: false },
      { social_provider: null, has_password: true },
      { social_provider: "google", has_password: true },
      { social_provider: null, has_password: undefined },
    ])(
      "shows password changes only for a stored password: %j",
      async (credentials) => {
        useAuthStore.setState({
          user: { ...useAuthStore.getState().user!, ...credentials },
        });
        await open(route, "security");
        if (credentials.has_password) {
          expect(
            screen.getByText("Change Password", { selector: "div" }),
          ).toBeInTheDocument();
          expect(screen.getByLabelText("Current Password")).toBeEnabled();
          expect(
            screen.getByRole("button", { name: "Change Password" }),
          ).toBeInTheDocument();
        } else {
          expect(screen.queryAllByText("Change Password")).toHaveLength(0);
          expect(
            screen.queryByLabelText("Current Password"),
          ).not.toBeInTheDocument();
        }
      },
    );

    it("discloses deletion of every managed Machine and paused workspace in the actual confirmation", async () => {
      await open(route, "security");
      await userEvent.click(screen.getByRole("button", { name: "Delete My Account" }));
      const dialog = screen.getByRole("dialog", { name: "Delete Account" });
      expect(dialog).toHaveTextContent(/all (?:of your )?managed Machines/i);
      expect(dialog).toHaveTextContent(/paused \/workspace disks/i);
      expect(dialog).toHaveTextContent(/temporary (?:files|data)/i);
      expect(dialog).toHaveTextContent(/irreversible/i);
      expect(dialog).toHaveTextContent(/no snapshots/i);
      expect(dialog).toHaveTextContent(/no recovery or restore/i);
      expect(dialog).toHaveTextContent(/at most 1 hour after a job ends/i);
      expect(dialog).toHaveTextContent(/30 days after the job ends/i);
      expect(dialog).toHaveTextContent(/90 days after completion/i);
      expect(dialog).toHaveTextContent(/audit and financial records/i);
      expect(dialog).toHaveTextContent(/cleanup pending/i);
      expect(within(dialog).getByRole("button", { name: "Permanently Delete" })).toBeDisabled();
    });

    it("binds the displayed deletion policy only after the owner types the account email", async () => {
      await open(route, "security");
      const bindingsRequest = ["/api/v1/onboarding/bindings", expect.objectContaining({ method: "GET" })];
      await userEvent.click(screen.getByRole("button", { name: "Delete My Account" }));
      const dialog = screen.getByRole("dialog", { name: "Delete Account" });
      const confirm = within(dialog).getByRole("button", { name: "Permanently Delete" });
      expect(confirm).toBeDisabled();
      expect(transport.mock.calls).toEqual([bindingsRequest]);
      await userEvent.type(within(dialog).getByLabelText("Email"), "wrong@example.com");
      expect(confirm).toBeDisabled();
      await userEvent.click(confirm);
      expect(transport.mock.calls).toEqual([bindingsRequest]);
      await userEvent.clear(within(dialog).getByLabelText("Email"));
      await userEvent.type(within(dialog).getByLabelText("Email"), "reader@example.com");
      expect(confirm).toBeEnabled();
      expect(transport.mock.calls).toEqual([bindingsRequest]);
      await userEvent.click(confirm);
      await waitFor(() => expect(transport.mock.calls).toEqual([
        bindingsRequest,
        ["/api/v1/users/me", expect.objectContaining({
          method: "DELETE",
          credentials: "include",
          headers: expect.objectContaining({ "X-NyxID-Account-Deletion-Policy": "machine-data" }),
        })],
      ]));
      await waitFor(() => expect(confirm).toBeEnabled());
    });

    it("replaces tab state on the supplied route for every account tab", async () => {
      const { router, history } = await open(route);
      for (const tab of SETTINGS_TABS) {
        const trigger = screen.getByRole("tab", {
          name:
            tab === "mcp" ? "MCP" : tab.charAt(0).toUpperCase() + tab.slice(1),
        });
        await userEvent.click(trigger);
        expect(router.state.location.pathname).toBe(route);
        expect(router.state.location.search).toEqual(route === "/settings" ? { tab } : { c: "chat", mock: 1, panel: "settings", panelTab: tab });
        expect(trigger).toHaveAttribute("data-state", "active");
        expect(history.length).toBe(1);
        if (tab === "security") await screen.findByText(/No active onboarding bindings/);
        if (tab === "sessions")
          expect(
            screen.getByRole("link", { name: "Generate terminal login code" }),
          ).toHaveAttribute("href", "/login/code");
        if (tab === "privacy")
          expect(
            screen.getByRole("link", { name: "privacy policy" }),
          ).toHaveAttribute("href", "/privacy");
      }
      if (route === "/settings") expect(screen.getByRole("heading", { name: "Account Settings" })).toHaveClass("text-22", "sm:text-28");
      else expect(screen.queryByRole("heading", { name: "Account Settings" })).not.toBeInTheDocument();
    });

    it("defaults invalid tab deep links to Profile", async () => {
      await open(route, "unknown");
      expect(screen.getByRole("tab", { name: "Profile" })).toHaveAttribute(
        "data-state",
        "active",
      );
    });
  },
);
