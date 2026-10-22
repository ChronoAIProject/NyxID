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
  useAuthStore.setState({ user: { id: "owner", email: "reader@example.com" } as User });
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
