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

const { securityUser, mockDisable, toastSuccess, toastError } = vi.hoisted(() => ({
  securityUser: { id: "owner", display_name: "Reader", avatar_url: null, email: "reader@example.com", email_verified: true, mfa_enabled: false, social_provider: null as string | null, has_password: true },
  mockDisable: vi.fn(),
  toastSuccess: vi.fn(),
  toastError: vi.fn(),
}));
vi.mock("sonner", () => ({ toast: { success: toastSuccess, error: toastError } }));
vi.mock("@/hooks/use-auth", () => ({
  useLogout: () => ({ mutateAsync: vi.fn() }),
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
  toastSuccess.mockReset();
  toastError.mockReset();
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
  client.setQueryData(["notifications", "settings"], {
    telegram_connected: false, telegram_enabled: false, push_enabled: true,
    approval_required: true, approval_timeout_secs: 60, grant_expiry_days: 30,
  });
  client.setQueryData(["notifications", "devices"], { devices: [] });
  client.setQueryData(["approvals", "service-configs", "personal"], { configs: [] });
  client.setQueryData(["user-services"], []);
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
  return { router, history, client };
}

async function saveSettings() {
  await userEvent.click(screen.getByRole("button", { name: "Save changes" }));
  const review = await screen.findByRole("dialog", { name: "Review changes" });
  await userEvent.click(within(review).getByRole("button", { name: "Save changes" }));
  await waitFor(() => expect(review).not.toBeInTheDocument());
}

describe.each(["/settings", "/assistant"] as const)(
  "SettingsPage at %s",
  (route) => {
    it("uses sibling groups for profile, security, sessions, MCP, display and privacy", async () => {
      await open(route);
      for (const label of ["Profile", "Security", "Sessions", "MCP", "Display", "Privacy"]) {
        await userEvent.click(screen.getByRole("tab", { name: label }));
        const panel = screen.getByRole("tabpanel");
        await waitFor(() => expect(panel.querySelector("[data-grouped-list]")).not.toBeNull());
        expect(panel.querySelector("[data-grouped-list] [data-grouped-list], [data-slot=card]")).toBeNull();
        if (label === "security") await screen.findByText(/No active onboarding bindings/);
      }
      expect(screen.queryByRole("region", { name: "Unsaved changes" })).not.toBeInTheDocument();
    });

    it("identifies editable text fields and keeps long email addresses available", async () => {
      await open(route);
      const name = screen.getByRole("textbox", { name: "Name" });
      expect(name).toBeEnabled();
      expect(name).toHaveAttribute("placeholder", "Your name");
      const email = screen.getByRole("textbox", { name: "Email" });
      expect(email).toBeDisabled();
      expect(email).toHaveAttribute("title", securityUser.email);
      expect(email).toHaveClass("truncate");
      act(() => useAuthStore.setState({ user: { ...useAuthStore.getState().user!, has_password: true } }));
      await userEvent.click(screen.getByRole("tab", { name: "Security" }));
      for (const [label, placeholder] of [["Current Password", "Enter current password"], ["New Password", "Enter new password"], ["Confirm New Password", "Re-enter new password"]] as const) {
        const input = screen.getByLabelText(label, { exact: true });
        expect(input).toHaveAttribute("placeholder", placeholder);
        expect(input).toHaveAttribute("type", "password");
        await userEvent.click(screen.getByRole("button", { name: `Show ${label.toLowerCase()}` }));
        expect(input).toHaveAttribute("type", "text");
        await userEvent.click(screen.getByRole("button", { name: `Hide ${label.toLowerCase()}` }));
        expect(input).toHaveAttribute("type", "password");
      }
    });

    it("keeps profile drafts across tabs, reviews only the name, and preserves the update API", async () => {
      const config = { ...useAuthStore.getState().user!.profile_config!, analytics: { prompt_seen: true } };
      useAuthStore.setState({ user: { ...useAuthStore.getState().user!, profile_config: config } });
      const original = transport.getMockImplementation()!;
      transport.mockImplementation(async (url, options) => url === "/api/v1/users/me" && options?.method === "PUT"
        ? new Response(JSON.stringify({ ...securityUser, id: "owner", display_name: "Updated Reader" })) : original(url, options));
      const { client } = await open(route);
      const name = screen.getByRole("textbox", { name: "Name" });
      expect(name).toHaveValue("Reader");
      expect(screen.queryByRole("region", { name: "Unsaved changes" })).not.toBeInTheDocument();
      await userEvent.clear(name);
      await userEvent.type(name, "Updated Reader");
      expect(screen.getAllByRole("region", { name: "Unsaved changes" })).toHaveLength(1);
      await userEvent.click(screen.getByRole("tab", { name: "MCP" }));
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
      expect(screen.getByRole("region", { name: "Unsaved changes" })).toBeVisible();
      await userEvent.click(screen.getByRole("tab", { name: "Profile" }));
      expect(name).toHaveValue("Updated Reader");
      await userEvent.click(screen.getByRole("button", { name: "Save changes" }));
      const review = screen.getByRole("dialog", { name: "Review changes" });
      expect(review).toHaveTextContent("Name: Reader → Updated Reader");
      expect(review).not.toHaveTextContent(securityUser.email);
      await userEvent.click(within(review).getByRole("button", { name: "Save changes" }));
      await waitFor(() => expect(screen.queryByRole("region", { name: "Unsaved changes" })).not.toBeInTheDocument());
      expect(transport).toHaveBeenCalledWith("/api/v1/users/me", expect.objectContaining({ method: "PUT", body: JSON.stringify({ display_name: "Updated Reader" }) }));
      expect(useAuthStore.getState().user?.display_name).toBe("Updated Reader");
      expect(useAuthStore.getState().user?.profile_config).toEqual(config);
      expect(client.getQueryData<User>(["user", "me"])?.profile_config).toEqual(config);
    });

    it("retains profile edits made while a save is pending against the saved baseline", async () => {
      let finish!: (response: Response) => void;
      const original = transport.getMockImplementation()!;
      transport.mockImplementation(async (url, options) => url === "/api/v1/users/me" && options?.method === "PUT"
        ? new Promise<Response>(resolve => { finish = resolve; }) : original(url, options));
      await open(route);
      const name = screen.getByRole("textbox", { name: "Name" });
      await userEvent.clear(name);
      await userEvent.type(name, "Saved name");
      await userEvent.click(screen.getByRole("button", { name: "Save changes" }));
      const review = screen.getByRole("dialog", { name: "Review changes" });
      await userEvent.click(within(review).getByRole("button", { name: "Save changes" }));
      await waitFor(() => expect(finish).toBeTypeOf("function"));
      // The review traps focus, but an already queued native input can still commit.
      await act(async () => {
        const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
        setter.call(name, "Newer draft");
        name.dispatchEvent(new Event("input", { bubbles: true }));
      });
      await act(async () => finish(new Response(JSON.stringify({ ...securityUser, id: "owner", display_name: "Saved name" }))));
      await waitFor(() => expect(review).not.toBeInTheDocument());
      expect(name).toHaveValue("Newer draft");
      expect(screen.getByRole("region", { name: "Unsaved changes" })).toBeVisible();
      await userEvent.click(screen.getByRole("button", { name: "Discard" }));
      expect(name).toHaveValue("Saved name");
    });

    it("does not apply a pending profile update to a successor account", async () => {
      let finish!: (response: Response) => void;
      const original = transport.getMockImplementation()!;
      transport.mockImplementation(async (url, options) => url === "/api/v1/users/me" && options?.method === "PUT"
        ? new Promise<Response>(resolve => { finish = resolve; }) : original(url, options));
      const { client } = await open(route);
      await userEvent.type(screen.getByRole("textbox", { name: "Name" }), " draft");
      await userEvent.click(screen.getByRole("button", { name: "Save changes" }));
      const review = screen.getByRole("dialog", { name: "Review changes" });
      await userEvent.click(within(review).getByRole("button", { name: "Save changes" }));
      await waitFor(() => expect(finish).toBeTypeOf("function"));
      const successor = { ...useAuthStore.getState().user!, id: "other", display_name: "Other person" };
      act(() => { useAuthStore.getState().setUser(successor); });
      await act(async () => { finish(new Response(JSON.stringify({ ...securityUser, display_name: "Saved old name" }))); });
      await waitFor(() => expect(review).not.toBeInTheDocument());
      expect(useAuthStore.getState().user).toEqual(successor);
      expect(client.getQueryData(["user", "me"])).toBeUndefined();
    });

    it("retains failed profile edits and discards without calling the API again", async () => {
      const original = transport.getMockImplementation()!;
      transport.mockImplementation(async (url, options) => url === "/api/v1/users/me" && options?.method === "PUT"
        ? new Response(JSON.stringify({ error: "validation_error", error_code: 1000, message: "Name rejected" }), { status: 400 }) : original(url, options));
      await open(route);
      await userEvent.type(screen.getByRole("textbox", { name: "Name" }), " draft");
      await saveSettings();
      expect(await screen.findByRole("alert")).toHaveTextContent("Name rejected");
      expect(screen.getByRole("textbox", { name: "Name" })).toHaveValue("Reader draft");
      await userEvent.click(screen.getByRole("button", { name: "Discard" }));
      expect(screen.getByRole("textbox", { name: "Name" })).toHaveValue("Reader");
      expect(screen.queryByRole("region", { name: "Unsaved changes" })).not.toBeInTheDocument();
      expect(transport.mock.calls.filter(([, options]) => options?.method === "PUT")).toHaveLength(1);
    });

    it("keeps password validation and hides passwords from the change review", async () => {
      useAuthStore.setState({ user: { ...useAuthStore.getState().user!, has_password: true } });
      await open(route, "security");
      await userEvent.type(screen.getByLabelText("New Password", { exact: true }), "short");
      await userEvent.click(screen.getByRole("button", { name: "Save changes" }));
      const review = screen.getByRole("dialog", { name: "Review changes" });
      expect(review).toHaveTextContent("Password: Current password → Changed");
      expect(review).not.toHaveTextContent("short");
      await userEvent.click(within(review).getByRole("button", { name: "Save changes" }));
      expect(await screen.findByText("Password must be at least 8 UTF-8 bytes")).toBeVisible();
      expect(screen.getByLabelText("New Password", { exact: true })).toHaveAttribute("aria-invalid", "true");
      const errorId = screen.getByText("Password must be at least 8 UTF-8 bytes").id;
      expect(screen.getByLabelText("New Password", { exact: true }).getAttribute("aria-describedby")).toContain(errorId);
      expect(transport.mock.calls.some(([url]) => url === "/api/v1/auth/password/change")).toBe(false);
    });

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
      await saveSettings();
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
      await saveSettings();
      await waitFor(() => expect(screen.getByLabelText("Current Password")).toHaveValue(""));
      expect(screen.getByLabelText("New Password", { exact: true })).toHaveValue("");
      expect(screen.queryByRole("region", { name: "Unsaved changes" })).not.toBeInTheDocument();
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
            screen.getByRole("heading", { name: "Change password" }),
          ).toBeInTheDocument();
          expect(screen.getByLabelText("Current Password")).toBeEnabled();
          expect(screen.queryByRole("region", { name: "Unsaved changes" })).not.toBeInTheDocument();
        } else {
          expect(screen.queryByRole("heading", { name: "Change password" })).not.toBeInTheDocument();
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

    it.each([null, "gzip"])("downloads the server-named JSON export and revokes its object URL (encoding %s)", async (encoding) => {
      const original = transport.getMockImplementation()!;
      transport.mockImplementation(async (url, options) => {
        if (url === "/api/v1/users/me/export" && options?.method === "POST") {
          return new Response('{"format":"nyxid.account-export"}', {
            headers: { "Content-Type": "application/json", "Content-Disposition": 'attachment; filename="account.json"', ...(encoding ? { "Content-Encoding": encoding, "Content-Length": "1024" } : {}) },
          });
        }
        return original(url, options);
      });
      const createObjectUrl = vi.fn(() => "blob:account-export");
      const revokeObjectUrl = vi.fn();
      const oldCreateObjectUrl = URL.createObjectURL;
      const oldRevokeObjectUrl = URL.revokeObjectURL;
      Object.defineProperty(URL, "createObjectURL", { configurable: true, value: createObjectUrl });
      Object.defineProperty(URL, "revokeObjectURL", { configurable: true, value: revokeObjectUrl });
      const click = vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => undefined);
      try {
        await open(route, "privacy");
        await userEvent.click(screen.getByRole("button", { name: "Download" }));
        await waitFor(() => expect(toastSuccess).toHaveBeenCalledWith("Your data export is ready."));
        expect(transport).toHaveBeenCalledWith("/api/v1/users/me/export", expect.objectContaining({ method: "POST" }));
        expect(createObjectUrl).toHaveBeenCalledOnce();
        expect(click).toHaveBeenCalledOnce();
        expect(click.mock.instances[0]).toHaveProperty("download", "account.json");
        await new Promise((resolve) => window.setTimeout(resolve, 1000));
        expect(revokeObjectUrl).toHaveBeenCalledWith("blob:account-export");
      } finally {
        if (oldCreateObjectUrl) Object.defineProperty(URL, "createObjectURL", { configurable: true, value: oldCreateObjectUrl }); else Reflect.deleteProperty(URL, "createObjectURL");
        if (oldRevokeObjectUrl) Object.defineProperty(URL, "revokeObjectURL", { configurable: true, value: oldRevokeObjectUrl }); else Reflect.deleteProperty(URL, "revokeObjectURL");
        click.mockRestore();
      }
    });

    it.each(["fetch abort", "body abort", "short body", "identity mismatch", "encoded invalid JSON", "invalid JSON without length"])("does not download an incomplete export: %s", async (failure) => {
      const original = transport.getMockImplementation()!;
      transport.mockImplementation(async (url, options) => {
        if (url !== "/api/v1/users/me/export" || options?.method !== "POST") return original(url, options);
        if (failure === "fetch abort") throw new TypeError("connection aborted");
        if (failure === "body abort") return new Response(new ReadableStream({
          start(controller) { controller.error(new TypeError("connection aborted")); },
        }));
        return new Response(["short body", "identity mismatch"].includes(failure) ? '{"format":"nyxid.account-export"}' : '{"format":', {
          headers: failure === "short body" ? { "Content-Length": "1024" } : failure === "identity mismatch" ? { "Content-Encoding": "identity", "Content-Length": "1024" } : failure === "encoded invalid JSON" ? { "Content-Encoding": "gzip", "Content-Length": "1024" } : {},
        });
      });
      const click = vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => undefined);
      try {
        await open(route, "privacy");
        await userEvent.click(screen.getByRole("button", { name: "Download" }));
        await waitFor(() => expect(toastError).toHaveBeenCalledWith("We couldn't create your data export. Please try again."));
        expect(click).not.toHaveBeenCalled();
        expect(toastSuccess).not.toHaveBeenCalled();
      } finally { click.mockRestore(); }
    });

    it.each([[1, "1 second"], [42, "42 seconds"], [60, "1 minute"], [130, "3 minutes"]])("shows Retry-After %i with the right time unit", async (seconds, duration) => {
      const original = transport.getMockImplementation()!;
      transport.mockImplementation(async (url, options) => url === "/api/v1/users/me/export" && options?.method === "POST"
        ? new Response(JSON.stringify({ error: "rate_limited", error_code: 1005, message: "Rate limited" }), { status: 429, headers: { "Content-Type": "application/json", "Retry-After": String(seconds) } })
        : original(url, options));
      await open(route, "privacy");
      await userEvent.click(screen.getByRole("button", { name: "Download" }));
      await waitFor(() => expect(toastError).toHaveBeenCalledWith(`Your data export is temporarily rate limited. Try again in ${duration}.`));
    });

    it.each([404, 405])("explains that export is unavailable on an older backend (%i)", async (status) => {
      const original = transport.getMockImplementation()!;
      transport.mockImplementation(async (url, options) => url === "/api/v1/users/me/export" && options?.method === "POST"
        ? new Response(JSON.stringify({ error: "not_found", error_code: 1404, message: "Not available" }), { status, headers: { "Content-Type": "application/json" } })
        : original(url, options));
      await open(route, "privacy");
      await userEvent.click(screen.getByRole("button", { name: "Download" }));
      await waitFor(() => expect(toastError).toHaveBeenCalledWith("Data export is not available on this server yet."));
    });

    it("shows a generic message for other export failures", async () => {
      const original = transport.getMockImplementation()!;
      transport.mockImplementation(async (url, options) => url === "/api/v1/users/me/export" && options?.method === "POST"
        ? new Response("upstream failure", { status: 500, headers: { "Content-Type": "text/plain" } })
        : original(url, options));
      await open(route, "privacy");
      await userEvent.click(screen.getByRole("button", { name: "Download" }));
      await waitFor(() => expect(toastError).toHaveBeenCalledWith("We couldn't create your data export. Please try again."));
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
        if (tab === "notifications")
          expect(await screen.findByRole("switch", { name: "Mentions in groups" })).toBeVisible();
        if (tab === "privacy")
          expect(
            screen.getByRole("link", { name: "privacy policy" }),
          ).toHaveAttribute("href", "/privacy");
      }
      if (route === "/settings") expect(screen.getByRole("heading", { name: "Account Settings" })).toHaveClass("type-compact-page-title", "sm:type-page-title");
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
    useAuthStore.setState({ user: { ...securityUser } as User });
    const user = userEvent.setup(); await open("/settings", "security");
    await user.click(await screen.findByRole("switch", { name: "Toggle two-factor authentication" }));
    expect(screen.getByTestId("mfa-setup")).toBeInTheDocument();
  });
  it.each([{ provider: "google", hasPassword: false }, { provider: null, hasPassword: true }, { provider: "github", hasPassword: true }])("requires code and the actual password availability: $provider / $hasPassword", async ({ provider, hasPassword }) => {
    Object.assign(securityUser, { mfa_enabled: true, social_provider: provider, has_password: hasPassword });
    useAuthStore.setState({ user: { ...securityUser } as User });
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

it("privacy hides diagnostics when not offered and orders offered rows account browser diagnostics", async () => {
  const {client}=await open("/settings","privacy");
  expect(screen.getByText("Optional usage events are linked to your account. Account permission applies across devices. Each device must also opt in locally; Do Not Track or Global Privacy Control always disables collection. Security audit and billing records have separate retention rules.")).toBeInTheDocument();
  await waitFor(()=>expect(transport).toHaveBeenCalledWith("/api/v1/users/me/preferences/analytics",expect.anything()));
  expect(screen.queryByText("Crash diagnostics")).not.toBeInTheDocument();
  expect(screen.queryByText("Review updated analytics")).not.toBeInTheDocument();
  await act(async()=>{client.setQueryData(["analytics-consent","owner"],{account_id:"owner",enabled:true,current_policy_version:"2",diagnostics_offered:true,diagnostics_available:false,revision:"live"});});
  const diagnostics=await screen.findByText("Crash diagnostics");
  const browser=screen.getByText("Allow on this browser");
  expect(browser.compareDocumentPosition(diagnostics) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(screen.getByRole("button", { name: "Review updated analytics" })).toBeInTheDocument();
  useConsentStore.setState({ accountId: "owner", enabled: true, asked: true, policy: 2 });
  await act(async()=>{client.setQueryData(["analytics-consent","owner"],{account_id:"owner",enabled:true,current_policy_version:"2",diagnostics_offered:true,diagnostics_available:true,revision:"live"});});
  expect(screen.queryByRole("button", { name: "Review updated analytics" })).not.toBeInTheDocument();
});

it.each(["2",null])("account re-enable retains an acknowledged policy from account=%s or device",async(accountPolicy)=>{
  useConsentStore.getState().setConsent(false,2);
  transport.mockImplementation(async (_url,options)=>new Response(JSON.stringify({account_id:"owner",enabled:options?.method==="PUT",policy_version:accountPolicy,current_policy_version:"2",diagnostics_offered:true,diagnostics_status:"consent_required",diagnostics_available:false,revision:"r",changed_at:null}),{headers:{"Content-Type":"application/json"}}));
  await open("/settings","privacy");
  const toggle=await screen.findByRole("switch",{name:"Allow account usage analytics"});
  await waitFor(()=>expect(toggle).not.toBeDisabled()); await userEvent.click(toggle);
  await waitFor(()=>expect(transport.mock.calls.some(([,opts])=>opts?.method==="PUT")).toBe(true));
  const request=transport.mock.calls.find(([,opts])=>opts?.method==="PUT")!;
  expect(JSON.parse(String(request[1]?.body))).toMatchObject({enabled:true,policy_version:"2"});
});
