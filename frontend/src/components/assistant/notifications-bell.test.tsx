import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { useAuthStore } from "@/stores/auth-store";
import { notificationBadgeText, notificationBellLabel } from "@/lib/assistant/notification-presentation";
import { NotificationsBell } from "./notifications-bell";

const panel = vi.hoisted(() => ({ open: vi.fn(), current: {} as { panel?: string } }));
vi.mock("@/hooks/use-account-panel", () => ({
  useAccountPanel: () => ({ current: panel.current, open: panel.open }),
}));

let summary: Record<string, unknown>;
let summaryReads = 0;
const json = (value: unknown) => new Response(JSON.stringify(value), { status: 200, headers: { "content-type": "application/json" } });

function signIn(features: string[]) {
  useAuthStore.getState().setUser({
    id: "owner", email: "owner@example.com", display_name: "Owner", avatar_url: null,
    email_verified: true, mfa_enabled: false, is_admin: false, is_active: true,
    created_at: "2026-11-06T00:00:00Z", capabilities: { enabled_features: features },
  });
}

function renderBell() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(<QueryClientProvider client={client}><NotificationsBell /></QueryClientProvider>);
}

beforeEach(() => {
  summaryReads = 0;
  summary = { unread_count: 0, needs_decision_count: 0, by_source: [] };
  panel.open.mockReset();
  panel.current = {};
  globalThis.__nyxidAssistantHttpMock = ({ endpoint }) => {
    if (endpoint === "/assistant/nyxagent/notifications/summary") { summaryReads += 1; return json(summary); }
    throw new Error(`Unexpected request: ${endpoint}`);
  };
});

afterEach(() => {
  cleanup();
  globalThis.__nyxidAssistantHttpMock = undefined;
  useAuthStore.setState({ user: null });
});

it("is absent, and reads nothing, without the Notifications feature", async () => {
  signIn([FEATURE_FLAG.NYXAGENT_ENGINE]);
  renderBell();
  expect(screen.queryByRole("button", { name: /Notifications/ })).not.toBeInTheDocument();
  await new Promise((resolve) => setTimeout(resolve, 20));
  expect(summaryReads).toBe(0);
});

it("shows the unread count, capped at 9+, and names what needs you", async () => {
  signIn([FEATURE_FLAG.NYXAGENT_ENGINE, FEATURE_FLAG.ASSISTANT_NOTIFICATIONS]);
  summary = { unread_count: 12, needs_decision_count: 2, by_source: [] };
  renderBell();
  const bell = await screen.findByRole("button", { name: "Notifications, 12 unread, 2 need you" });
  expect(bell).toHaveTextContent("9+");
  expect(bell.querySelector("[data-needs-decision]")).not.toBeNull();
  expect(bell).toHaveAttribute("aria-haspopup", "dialog");
  expect(bell).toHaveAttribute("aria-expanded", "false");
});

it("uses the neutral badge when nothing needs a decision and none at zero", async () => {
  signIn([FEATURE_FLAG.NYXAGENT_ENGINE, FEATURE_FLAG.ASSISTANT_NOTIFICATIONS]);
  summary = { unread_count: 3, needs_decision_count: 0, by_source: [] };
  const view = renderBell();
  const bell = await screen.findByRole("button", { name: "Notifications, 3 unread" });
  expect(bell).toHaveTextContent("3");
  expect(bell.querySelector("[data-needs-decision]")).toBeNull();
  view.unmount();
  summary = { unread_count: 0, needs_decision_count: 0, by_source: [] };
  renderBell();
  const quiet = await screen.findByRole("button", { name: "Notifications" });
  await waitFor(() => expect(summaryReads).toBeGreaterThanOrEqual(2));
  expect(quiet).toHaveTextContent("");
});

it("opens the notifications panel from the bell and marks it expanded while open", async () => {
  signIn([FEATURE_FLAG.NYXAGENT_ENGINE, FEATURE_FLAG.ASSISTANT_NOTIFICATIONS]);
  const user = userEvent.setup();
  renderBell();
  const bell = await screen.findByRole("button", { name: "Notifications" });
  await user.click(bell);
  expect(panel.open).toHaveBeenCalledWith("notifications", {}, bell);
  cleanup();
  panel.current = { panel: "notifications" };
  renderBell();
  expect(await screen.findByRole("button", { name: "Notifications" })).toHaveAttribute("aria-expanded", "true");
});

it("formats the badge and label", () => {
  expect(notificationBadgeText(9)).toBe("9");
  expect(notificationBadgeText(10)).toBe("9+");
  expect(notificationBellLabel(0, 0)).toBe("Notifications");
  expect(notificationBellLabel(1, 1)).toBe("Notifications, 1 unread, 1 needs you");
});
