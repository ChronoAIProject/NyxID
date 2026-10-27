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
import { useConsentStore } from "@/stores/consent-store";
import type { User } from "@/types/api";
import { SettingsPage } from "./settings";

const { securityUser, mockDisable } = vi.hoisted(() => ({
  securityUser: { display_name: "Reader", email: "reader@example.com", email_verified: true, mfa_enabled: false, social_provider: null as string | null, has_password: true },
  mockDisable: vi.fn(),
}));
vi.mock("@/hooks/use-auth", () => ({
  useUser: () => ({ data: securityUser }),
  useMfaDisable: () => ({ mutateAsync: mockDisable }),
  useRevokeSession: () => ({ mutateAsync: vi.fn() }),
}));
vi.mock("@/hooks/use-public-config", () => ({
  usePublicConfig: () => ({ data: { mcp_url: "https://nyxid.test/mcp", email_auth_enabled: false } }),
}));
vi.mock("@/components/auth/mfa-setup-dialog", () => ({
  MfaSetupDialog: ({ open }: { open: boolean }) => open ? <div data-testid="mfa-setup" /> : null,
}));

const initialAuth = useAuthStore.getState();
const transport = vi.fn<typeof fetch>();
const unexpectedRequests: string[] = [];
beforeEach(() => {
  Object.assign(securityUser, { mfa_enabled: false, social_provider: null, has_password: true });
  mockDisable.mockReset().mockResolvedValue(undefined);
  useAuthStore.setState({ user: { id: "owner", email: "reader@example.com", mfa_enabled: true, profile_config: { security: { skip_mfa_for_approved_sign_ins: false }, onboarding: { ai_services_completed_at: null } } } as User });
  unexpectedRequests.length = 0;
  transport.mockReset().mockImplementation(async (url, options) => {
    const headers = { "Content-Type": "application/json" };
    if (url === "/api/v1/users/me/preferences/analytics" && options?.method === "GET") {
      return new Response(JSON.stringify({ account_id: "owner", enabled: false, changed_at: null, revision: "" }), { headers });
    }
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

    it("saves the QR MFA preference when TOTP is enabled", async () => {
      const original = transport.getMockImplementation()!;
      transport.mockImplementation(async (url, options) => url === "/api/v1/users/me/security"
        ? new Response(String(options?.body), { headers: { "Content-Type": "application/json" } })
        : original(url, options));
      await open(route, "security");
      const toggle = screen.getByRole("switch", { name: "Skip MFA for NyxID app sign-ins" });
      expect(toggle).toHaveAttribute("aria-checked", "false");
      await userEvent.click(toggle);
      expect(screen.getByRole("dialog")).toHaveTextContent("Anyone with access to your signed-in NyxID app");
      await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
      expect(transport.mock.calls.some(([url]) => url === "/api/v1/users/me/security")).toBe(false);
      await userEvent.click(toggle);
      await userEvent.click(screen.getByRole("button", { name: "Enable for app sign-ins" }));
      await waitFor(() => expect(toggle).toHaveAttribute("aria-checked", "true"));
      expect(transport).toHaveBeenCalledWith("/api/v1/users/me/security", expect.objectContaining({
        method: "PUT", body: JSON.stringify({ skip_mfa_for_approved_sign_ins: true }),
      }));
      await userEvent.click(toggle);
      await waitFor(() => expect(toggle).toHaveAttribute("aria-checked", "false"));
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    });

    it("shows the resolved default on and saves an explicit opt-out", async () => {
      useAuthStore.setState({ user: { ...useAuthStore.getState().user!, profile_config: { security: { skip_mfa_for_approved_sign_ins: true } } } as User });
      const original = transport.getMockImplementation()!;
      transport.mockImplementation(async (url, options) => url === "/api/v1/users/me/security"
        ? new Response(String(options?.body), { headers: { "Content-Type": "application/json" } })
        : original(url, options));
      await open(route, "security");
      const toggle = screen.getByRole("switch", { name: "Skip MFA for NyxID app sign-ins" });
      expect(toggle).toHaveAttribute("aria-checked", "true");
      expect(screen.getByText(/Enabled by default/)).toBeInTheDocument();
      await userEvent.click(toggle);
      await waitFor(() => expect(toggle).toHaveAttribute("aria-checked", "false"));
      expect(transport).toHaveBeenCalledWith("/api/v1/users/me/security", expect.objectContaining({
        method: "PUT", body: JSON.stringify({ skip_mfa_for_approved_sign_ins: false }),
      }));
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    });

    it("saves either choice without two-factor authentication", async () => {
      useAuthStore.setState({ user: { ...useAuthStore.getState().user!, mfa_enabled: false } });
      const original = transport.getMockImplementation()!;
      transport.mockImplementation(async (url, options) => url === "/api/v1/users/me/security"
        ? new Response(String(options?.body), { headers: { "Content-Type": "application/json" } })
        : original(url, options));
      await open(route, "security");
      const toggle = screen.getByRole("switch", { name: "Skip MFA for NyxID app sign-ins" });
      expect(toggle).toBeEnabled();
      expect(screen.getByText(/Takes effect when two-factor authentication is on/)).toBeInTheDocument();
      await userEvent.click(toggle);
      await userEvent.click(screen.getByRole("button", { name: "Enable for app sign-ins" }));
      await waitFor(() => expect(toggle).toHaveAttribute("aria-checked", "true"));
      await userEvent.click(toggle);
      await waitFor(() => expect(toggle).toHaveAttribute("aria-checked", "false"));
      expect(screen.queryByRole("region", { name: "Verify this session" })).not.toBeInTheDocument();
    });

    it("keeps the saved value and shows a failed save", async () => {
      const original = transport.getMockImplementation()!;
      transport.mockImplementation(async (url, options) => url === "/api/v1/users/me/security"
        ? new Response(JSON.stringify({ error: "unavailable", error_code: 1000, message: "Try again later" }), { status: 503 })
        : original(url, options));
      await open(route, "security");
      await userEvent.click(screen.getByRole("switch", { name: "Skip MFA for NyxID app sign-ins" }));
      await userEvent.click(screen.getByRole("button", { name: "Enable for app sign-ins" }));
      expect(await screen.findByRole("alert")).toHaveTextContent("Try again later");
      expect(screen.getByRole("switch", { name: "Skip MFA for NyxID app sign-ins" })).toHaveAttribute("aria-checked", "false");
    });

    it("verifies the session once and retries enabling", async () => {
      let attempts = 0;
      const original = transport.getMockImplementation()!;
      transport.mockImplementation(async (url, options) => {
        if (url === "/api/v1/users/me/security") {
          attempts += 1;
          if (attempts === 1) return new Response(JSON.stringify({ error: "mfa_step_up_required", error_code: 2003, message: "Verify this session to enable" }), { status: 403 });
          return new Response(String(options?.body), { headers: { "Content-Type": "application/json" } });
        }
        if (url === "/api/v1/auth/mfa/step-up") return new Response("{}");
        return original(url, options);
      });
      await open(route, "security");
      await userEvent.click(screen.getByRole("switch", { name: "Skip MFA for NyxID app sign-ins" }));
      await userEvent.click(screen.getByRole("button", { name: "Enable for app sign-ins" }));
      const verify = await screen.findByRole("region", { name: "Verify this session" });
      await userEvent.type(within(verify).getByRole("textbox"), "123456");
      await userEvent.click(within(verify).getByRole("button", { name: "Verify & continue" }));
      await waitFor(() => expect(screen.getByRole("switch", { name: "Skip MFA for NyxID app sign-ins" })).toHaveAttribute("aria-checked", "true"));
      expect(attempts).toBe(2);
    });

    it("disables the unsupported setting for older servers", async () => {
      useAuthStore.setState({ user: { ...useAuthStore.getState().user!, profile_config: undefined } });
      await open(route, "security");
      expect(screen.getByRole("switch", { name: "Skip MFA for NyxID app sign-ins" })).toBeDisabled();
      expect(screen.queryByText("Always enabled")).not.toBeInTheDocument();
    });

    it("does not apply a pending save to a replacement account", async () => {
      let finish!: (response: Response) => void;
      const original = transport.getMockImplementation()!;
      transport.mockImplementation(async (url, options) => url === "/api/v1/users/me/security"
        ? new Promise<Response>((resolve) => { finish = resolve; })
        : original(url, options));
      await open(route, "security");
      await userEvent.click(screen.getByRole("switch", { name: "Skip MFA for NyxID app sign-ins" }));
      await userEvent.click(screen.getByRole("button", { name: "Enable for app sign-ins" }));
      await waitFor(() => expect(finish).toBeDefined());
      expect(screen.getByRole("switch", { name: "Skip MFA for NyxID app sign-ins" })).toBeDisabled();
      act(() => useAuthStore.setState({ user: { ...useAuthStore.getState().user!, id: "replacement" } }));
      await act(async () => finish(new Response(JSON.stringify({ skip_mfa_for_approved_sign_ins: true }))));
      expect(useAuthStore.getState().user?.id).toBe("replacement");
      expect(useAuthStore.getState().user?.profile_config?.security?.skip_mfa_for_approved_sign_ins).toBe(false);
    });

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

    it("allows local withdrawal while account permission is unavailable", async () => {
      useConsentStore.setState({ accountId: "owner", enabled: true, asked: true });
      transport.mockResolvedValueOnce(new Response("{}", { status: 503, headers: { "Content-Type": "application/json" } }));
      await open(route, "privacy");
      await screen.findByText("Account permission unavailable");
      const local = screen.getByRole("switch", { name: "Allow usage analytics on this browser" });
      expect(local).toBeEnabled();
      await userEvent.click(local);
      expect(useConsentStore.getState().enabled).toBe(false);
      expect(local).toHaveAttribute("aria-checked", "false");
      expect(transport).toHaveBeenCalledTimes(1);
    });

    it("distinguishes loading and failure and retries account permission", async () => {
      let resolve!: (response: Response) => void;
      transport.mockImplementationOnce(() => new Promise<Response>((done) => { resolve = done; }));
      await open(route, "privacy");
      expect(screen.getByRole("status")).toHaveTextContent("Loading account permission");
      expect(screen.getByRole("switch", { name: "Allow account usage analytics" })).toBeDisabled();
      await waitFor(() => expect(resolve).toBeTypeOf("function"));
      await act(async () => resolve(new Response("{}", { status: 503, headers: { "Content-Type": "application/json" } })));
      await screen.findByText("Account permission unavailable");
      expect(screen.getByRole("switch", { name: "Allow account usage analytics" })).toBeDisabled();
      await userEvent.click(screen.getByRole("button", { name: "Retry" }));
      await screen.findByText("Account permission disabled");
      expect(screen.getByRole("switch", { name: "Allow account usage analytics" })).toBeEnabled();
      expect(transport).toHaveBeenCalledTimes(2);
    });

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
          headers: expect.objectContaining({ "X-NyxID-Account-Deletion-Policy": "machine-data-provider-revocation-v2" }),
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


describe("MFA security settings", () => {
  it("offers social-only enrollment while email authentication is disabled", async () => {
    securityUser.social_provider = "google"; securityUser.has_password = false;
    useAuthStore.setState({ user: { id: "owner", ...securityUser } as User });
    const user = userEvent.setup(); await open("/settings", "security");
    await user.click(await screen.findByRole("switch", { name: "Toggle two-factor authentication" }));
    expect(screen.getByTestId("mfa-setup")).toBeInTheDocument();
  });
  it.each([{ provider: "google", hasPassword: false }, { provider: null, hasPassword: true }, { provider: "github", hasPassword: true }])("requires code and the actual password availability: $provider / $hasPassword", async ({ provider, hasPassword }) => {
    Object.assign(securityUser, { mfa_enabled: true, social_provider: provider, has_password: hasPassword });
    useAuthStore.setState({ user: { id: "owner", ...securityUser } as User });
    const user = userEvent.setup(); await open("/settings", "security");
    await user.click(await screen.findByRole("switch", { name: "Toggle two-factor authentication" }));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).queryByLabelText("Password") !== null).toBe(hasPassword);
    await user.type(within(dialog).getByLabelText("Authenticator or recovery code"), "Ab_cd-12");
    if (hasPassword) await user.type(within(dialog).getByLabelText("Password"), "current-password");
    await user.click(within(dialog).getByRole("button", { name: "Disable MFA" }));
    await waitFor(() => expect(mockDisable).toHaveBeenCalledWith({ code: "Ab_cd-12", ...(hasPassword ? { password: "current-password" } : {}) }));
  });
});
