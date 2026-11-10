import { confirmChanges } from "@/test-helpers/confirm-changes";
import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it } from "vitest";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { useAuthStore } from "@/stores/auth-store";
import { NotificationPreferences } from "./notification-preferences";

const json = (value: unknown, status = 200) =>
  new Response(JSON.stringify(value), { status, headers: { "content-type": "application/json" } });

const SETTINGS = {
  skip_destructive_confirmation: false,
  access_mode: "nyxmode",
  max_live_subagents: 8,
  max_concurrent_subagent_turns: 3,
  max_live_subagents_limit: 32,
  max_concurrent_subagent_turns_limit: 8,
};

let settings: Record<string, unknown>;
const puts: unknown[] = [];
let saveStatus = 200;

function signIn(features: string[]) {
  useAuthStore.getState().setUser({
    id: "owner", email: "owner@example.com", display_name: "Owner", avatar_url: null,
    email_verified: true, mfa_enabled: false, is_admin: false, is_active: true,
    created_at: "2026-11-06T00:00:00Z", capabilities: { enabled_features: features },
  });
}

beforeEach(() => {
  puts.length = 0;
  saveStatus = 200;
  settings = { ...SETTINGS, notifications: { push: { chat_update: true } } };
  globalThis.__nyxidAssistantHttpMock = ({ endpoint, init }) => {
    if (endpoint === "/assistant/nyxagent/settings" && (init.method ?? "GET") === "GET") return json(settings);
    if (endpoint === "/assistant/nyxagent/settings" && init.method === "PUT") {
      const body = JSON.parse(init.body as string) as { notifications: { push: Record<string, boolean> } };
      puts.push(body);
      if (saveStatus !== 200) return json({ error: "boom", message: "Settings are read-only right now", error_code: 5000 }, saveStatus);
      settings = { ...settings, notifications: body.notifications };
      return json(settings);
    }
    throw new Error(`Unexpected request: ${init.method ?? "GET"} ${endpoint}`);
  };
});

afterEach(() => {
  cleanup();
  globalThis.__nyxidAssistantHttpMock = undefined;
  useAuthStore.setState({ user: null });
});

function renderPreferences() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  render(<QueryClientProvider client={client}><NotificationPreferences /></QueryClientProvider>);
}

it("is absent without the Notifications feature", async () => {
  signIn([FEATURE_FLAG.NYXAGENT_ENGINE]);
  renderPreferences();
  await new Promise((resolve) => setTimeout(resolve, 20));
  expect(screen.queryByRole("form", { name: "Notification preferences" })).not.toBeInTheDocument();
});

it("shows one push switch per kind, off by default, and saves the whole set after review", async () => {
  signIn([FEATURE_FLAG.NYXAGENT_ENGINE, FEATURE_FLAG.ASSISTANT_NOTIFICATIONS]);
  const user = userEvent.setup();
  renderPreferences();
  const form = await screen.findByRole("form", { name: "Notification preferences" });
  const switches = within(form).getAllByRole("switch");
  expect(switches.map((control) => control.getAttribute("aria-label"))).toEqual([
    "Push for automation results",
    "Push for automation problems",
    "Push for decisions waiting for you",
    "Push for agent updates",
    "Push for chat updates",
    "Push for group mentions",
  ]);
  expect(within(form).getByRole("switch", { name: "Push for chat updates" })).toHaveAttribute("aria-checked", "true");
  expect(within(form).getByRole("switch", { name: "Push for agent updates" })).toHaveAttribute("aria-checked", "false");
  expect(screen.queryByRole("button", { name: "Save notifications" })).not.toBeInTheDocument();

  await user.click(within(form).getByRole("switch", { name: "Push for decisions waiting for you" }));
  await user.click(within(form).getByRole("switch", { name: "Push for chat updates" }));
  await user.click(await screen.findByRole("button", { name: "Save notifications" }));
  const review = await screen.findByLabelText("Review changes", { selector: '[role="dialog"]' });
  expect(review).toHaveTextContent("Push: Decisions waiting for you");
  expect(review).toHaveTextContent("Push: Chat updates");
  await confirmChanges();
  await waitFor(() => expect(puts).toHaveLength(1));
  expect(puts[0]).toEqual({
    notifications: {
      push: {
        automation_result: false, automation_problem: false, needs_decision: true,
        agent_update: false, chat_update: false, group_mention: false,
      },
    },
  });
  await waitFor(() => expect(screen.queryByRole("button", { name: "Save notifications" })).not.toBeInTheDocument());
  expect(within(form).getByRole("switch", { name: "Push for decisions waiting for you" })).toHaveAttribute("aria-checked", "true");
});

it("keeps the draft and shows the error when saving fails", async () => {
  signIn([FEATURE_FLAG.NYXAGENT_ENGINE, FEATURE_FLAG.ASSISTANT_NOTIFICATIONS]);
  saveStatus = 409;
  const user = userEvent.setup();
  renderPreferences();
  const form = await screen.findByRole("form", { name: "Notification preferences" });
  await user.click(within(form).getByRole("switch", { name: "Push for group mentions" }));
  await user.click(await screen.findByRole("button", { name: "Save notifications" }));
  await confirmChanges(false);
  expect(await screen.findByText("Settings are read-only right now")).toBeInTheDocument();
  expect(within(form).getByRole("switch", { name: "Push for group mentions" })).toHaveAttribute("aria-checked", "true");
  expect(screen.getByRole("button", { name: "Save notifications" })).toBeInTheDocument();
});
