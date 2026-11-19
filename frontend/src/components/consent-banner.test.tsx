import "@/i18n";
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider, onlineManager } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ConsentBanner } from "./consent-banner";
import { useAuthStore } from "@/stores/auth-store";
import { useConsentStore } from "@/stores/consent-store";
import type { User } from "@/types/api";
import type { AnalyticsConsent } from "@/lib/analytics-consent";

const mocks = vi.hoisted(() => ({ api: vi.fn(), error: vi.fn(), telemetry: true, crashOffered: false, permission: {} as Partial<AnalyticsConsent> }));
vi.mock("@/lib/api-client", async (original) => ({
  ...await original<typeof import("@/lib/api-client")>(),
  apiClient: mocks.api,
}));
vi.mock("@/lib/telemetry", () => ({ disableTelemetry: vi.fn(), identify: vi.fn(), reset: vi.fn() }));
vi.mock("sonner", () => ({ toast: { error: mocks.error } }));
vi.mock("@/hooks/use-public-config", () => ({
  usePublicConfig: () => ({ data: { telemetry_share_analytics: mocks.telemetry, crash_diagnostics_offered: mocks.crashOffered || undefined } }),
}));

const initialAuth = useAuthStore.getState();
const initialConsent = useConsentStore.getState();
let client: QueryClient;
function show() {
  return render(<ConsentBanner />, { wrapper: ({ children }) => <QueryClientProvider client={client}>{children}</QueryClientProvider> });
}
function reads() {
  return mocks.api.mock.calls.filter(([, options]) => options.method !== "PUT");
}
const owner = {
  id: "owner",
  profile_config: { analytics: { prompt_seen: false }, onboarding: { ai_services_completed_at: null } },
} as User;

beforeEach(() => {
  localStorage.clear();
  mocks.telemetry = true;
  mocks.crashOffered = false;
  mocks.permission = { account_id: owner.id, enabled: false, prompt_seen: false, revision: "", current_policy_version: "1", diagnostics_offered: false };
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  mocks.api.mockReset().mockImplementation(async (_path, options) => {
    options.authorityGuard();
    if (options.method !== "PUT") return { ...mocks.permission };
    return { ...options.body, prompt_seen: true, revision: "saved", changed_at: "2026-11-18T00:00:00Z" };
  });
  mocks.error.mockReset();
  useAuthStore.setState({ user: owner, isAuthenticated: true });
  useConsentStore.getState().bindAccount(owner.id);
});

afterEach(() => {
  cleanup();
  client.clear();
  useAuthStore.setState(initialAuth);
  useConsentStore.setState(initialConsent);
});

it.each([false, true])("remembers the account decision after browser storage is cleared (allow=%s)", async (enabled) => {
  const user = userEvent.setup();
  const view = show();
  expect(screen.getByRole("dialog", { name: "Usage analytics" })).toHaveAttribute("aria-modal", "true");
  await user.click(screen.getByRole("button", { name: enabled ? "Accept telemetry" : "Decline telemetry" }));
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  expect(mocks.api).toHaveBeenCalledWith("/users/me/preferences/analytics", expect.objectContaining({
    method: "PUT", body: { account_id: owner.id, enabled },
  }));
  expect(useAuthStore.getState().user?.profile_config?.analytics?.prompt_seen).toBe(true);
  expect(useConsentStore.getState().enabled).toBe(enabled);

  // A fresh browser knows only the restored account profile, not this device's choice.
  view.unmount();
  localStorage.clear();
  useConsentStore.getState().bindAccount(owner.id);
  show();
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(useConsentStore.getState().enabled).toBe(false);
});

it.each([false, true])("honors an existing browser choice even if the profile acknowledgement is false (allow=%s)", (enabled) => {
  useConsentStore.getState().setConsent(enabled);
  show();
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(mocks.api).not.toHaveBeenCalled();
  expect(useConsentStore.getState().enabled).toBe(enabled);
});

it("requires an explicit selection and ignores Escape and clicks outside the footer", async () => {
  const user = userEvent.setup();
  show();
  const dialog = screen.getByRole("dialog");
  await user.keyboard("{Escape}");
  // Radix renders the full-page backdrop immediately before the footer.
  await user.click(dialog.previousElementSibling!);
  expect(dialog).toBeInTheDocument();
  expect(mocks.api).not.toHaveBeenCalled();
  expect(useConsentStore.getState().asked).toBe(false);
});

it("keeps the footer open and prevents duplicate choices until the save is acknowledged", async () => {
  let release!: (value: object) => void;
  mocks.api.mockImplementationOnce(() => new Promise((resolve) => { release = resolve; }));
  show();
  await userEvent.click(screen.getByRole("button", { name: "Accept telemetry" }));
  expect(screen.getByRole("dialog")).toHaveAttribute("aria-busy", "true");
  expect(screen.getByRole("button", { name: "Accept telemetry" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Decline telemetry" })).toBeDisabled();
  expect(useConsentStore.getState()).toMatchObject({ enabled: false, asked: false });
  const dialog = screen.getByRole("dialog");
  expect(dialog).toHaveFocus();
  await userEvent.tab();
  expect(dialog).toHaveFocus();
  await userEvent.tab({ shift: true });
  expect(dialog).toHaveFocus();
  await userEvent.click(screen.getByRole("button", { name: "Decline telemetry" }));
  expect(mocks.api).toHaveBeenCalledOnce();
  await act(async () => { release({ account_id: owner.id, enabled: true }); });
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(useConsentStore.getState()).toMatchObject({ enabled: true, asked: true });
});

it.each([
  { enabled: false, acknowledged: false },
  { enabled: true, acknowledged: false },
  { enabled: false, acknowledged: true },
  { enabled: true, acknowledged: true },
])("keeps a pending choice through profile refresh (allow=$enabled, server answered=$acknowledged)", async ({ enabled, acknowledged }) => {
  let release!: (value: object) => void;
  mocks.api.mockImplementationOnce(() => new Promise((resolve) => { release = resolve; }));
  useAuthStore.getState().setUser(owner);
  show();
  await userEvent.click(screen.getByRole("button", { name: enabled ? "Accept telemetry" : "Decline telemetry" }));
  const revision = useConsentStore.getState().revision;
  act(() => { useAuthStore.getState().setUser({
    ...owner,
    display_name: "Refreshed profile",
    profile_config: { ...owner.profile_config!, analytics: { prompt_seen: acknowledged } },
  }); });
  expect(useConsentStore.getState().revision).toBe(revision);
  expect(screen.getByRole("dialog")).toHaveAttribute("aria-busy", "true");
  expect(screen.getByRole("dialog")).toHaveFocus();
  expect(screen.getByRole("button", { name: "Accept telemetry" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Decline telemetry" })).toBeDisabled();
  await act(async () => { release({ account_id: owner.id, enabled }); });
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(mocks.error).not.toHaveBeenCalled();
  expect(useConsentStore.getState()).toMatchObject({ enabled, asked: true });
  expect(useAuthStore.getState().user?.profile_config?.analytics?.prompt_seen).toBe(true);
});

it.each([true, false])("allows retry after a failed save and keeps this browser off (profile present=%s)", async (profilePresent) => {
  if (!profilePresent) useAuthStore.setState({ user: { ...owner, profile_config: undefined } });
  mocks.api.mockRejectedValueOnce(new Error("offline"));
  show();
  await userEvent.click(screen.getByRole("button", { name: "Accept telemetry" }));
  await waitFor(() => expect(mocks.error).toHaveBeenCalledOnce());
  expect(screen.getByRole("dialog")).toBeInTheDocument();
  expect(useAuthStore.getState().user?.profile_config?.analytics?.prompt_seen).toBe(profilePresent ? false : undefined);
  expect(useConsentStore.getState()).toMatchObject({ enabled: false, asked: false });
  await userEvent.click(screen.getByRole("button", { name: "Decline telemetry" }));
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
});

it("does not apply the previous account's acknowledgement to a new person", () => {
  useAuthStore.setState({ user: { ...owner, profile_config: { ...owner.profile_config!, analytics: { prompt_seen: true } } } });
  show();
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  act(() => {
    useAuthStore.setState({ user: { ...owner, id: "other" } });
    useConsentStore.getState().bindAccount("other");
  });
  expect(screen.getByRole("dialog")).toBeInTheDocument();
});

it("does not use another account's browser choice while identity binding changes", () => {
  useConsentStore.getState().bindAccount("other");
  useConsentStore.getState().setConsent(true);
  show();
  expect(screen.getByRole("dialog")).toBeInTheDocument();
  expect(mocks.api).not.toHaveBeenCalled();
});

it("does not prompt signed-out people or deployments without analytics", () => {
  mocks.telemetry = false;
  const view = show();
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  mocks.telemetry = true;
  act(() => { useAuthStore.setState({ user: null }); });
  view.rerender(<ConsentBanner />);
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});

it("retains the existing disclosure and makes no preference GET when crash diagnostics are not offered", async () => {
  show();
  expect(screen.getByRole("dialog")).toHaveTextContent("Do Not Track or Global Privacy Control always disables collection");
  expect(screen.queryByText(/crash diagnostics/i)).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Decline telemetry" }));
  expect(reads()).toHaveLength(0);
});

it.each([false, true])("first-time offered diagnostics includes its disclosure and saves policy 2 (allow=%s)", async (enabled) => {
  mocks.crashOffered = true;
  mocks.permission = { ...mocks.permission, current_policy_version: "2", diagnostics_offered: true };
  show();
  const dialog = await screen.findByRole("dialog");
  for (const part of ["linked to your account", "Declining withdraws account permission across devices", "Each other device still needs its own opt-in", "Do Not Track or Global Privacy Control", "sanitized code locations", "up to 30 days"])
    expect(dialog).toHaveTextContent(part);
  expect(dialog).not.toHaveTextContent("Analytics updated");
  await userEvent.click(screen.getByRole("button", { name: enabled ? "Accept telemetry" : "Decline telemetry" }));
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  expect(mocks.api).toHaveBeenCalledWith("/users/me/preferences/analytics", expect.objectContaining({ method: "PUT", body: { account_id: owner.id, enabled, policy_version: "2" } }));
  expect(useConsentStore.getState()).toMatchObject({ enabled, asked: true, policy: 2 });
});

it.each([false, true])("does not redisplay the footer for existing choices when diagnostics are newly offered (allow=%s)", async (enabled) => {
  mocks.crashOffered = true;
  useConsentStore.getState().setConsent(enabled);
  show();
  await act(async () => { window.dispatchEvent(new Event("focus")); });
  expect(mocks.api).not.toHaveBeenCalled();
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});

it("an existing account acknowledgement suppresses diagnostics discovery on a fresh browser", async () => {
  mocks.crashOffered = true;
  useAuthStore.setState({ user: { ...owner, profile_config: { ...owner.profile_config!, analytics: { prompt_seen: true } } } });
  show();
  await act(async () => { await Promise.resolve(); });
  expect(mocks.api).not.toHaveBeenCalled();
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(useConsentStore.getState().enabled).toBe(false);
});

it("waits for the person's diagnostics offer before showing a crash-only first prompt", async () => {
  mocks.telemetry = false;
  mocks.crashOffered = true;
  let release!: (value: object) => void;
  mocks.api.mockImplementationOnce(() => new Promise(resolve => { release = resolve; }));
  show();
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  await act(async () => { release({ ...mocks.permission, current_policy_version: "2", diagnostics_offered: true }); });
  expect(await screen.findByRole("dialog")).toHaveTextContent("Optional crash diagnostics");
});

it("reuses discovery across remounts and time, focus and reconnect events", async () => {
  mocks.telemetry = false;
  mocks.crashOffered = true;
  const clock = vi.spyOn(Date, "now");
  try {
    const now = Date.now();
    const first = show();
    await waitFor(() => expect(reads()).toHaveLength(1));
    first.unmount();
    clock.mockReturnValue(now + 24 * 60 * 60 * 1000);
    show();
    await act(async () => {
      onlineManager.setOnline(false); onlineManager.setOnline(true);
      window.dispatchEvent(new Event("focus")); await Promise.resolve();
    });
    expect(reads()).toHaveLength(1);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  } finally { clock.mockRestore(); }
});

it("failed discovery does not retry on remount, focus or reconnect", async () => {
  mocks.crashOffered = true;
  mocks.api.mockRejectedValue(new Error("offline"));
  const first = show();
  await waitFor(() => expect(client.getQueryState(["analytics-consent", owner.id])?.status).toBe("error"));
  first.unmount(); show();
  await act(async () => {
    onlineManager.setOnline(false); onlineManager.setOnline(true);
    window.dispatchEvent(new Event("focus")); await Promise.resolve();
  });
  expect(reads()).toHaveLength(1);
  expect(await screen.findByRole("dialog")).toBeInTheDocument();
});
