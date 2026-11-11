import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { BotAudienceSettings } from "./bot-audience-settings";
import {
  assistantAgentSchema,
  nyxAgentChannelAgentSchema,
} from "@/schemas/assistant-nyxagent";
import {
  orgChannelSettingsSchema,
  type OrgChannelSettings,
} from "@/schemas/org-specialist-channel";

vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (select: (s: { user: { id: string } }) => unknown) =>
    select({ user: { id: "admin" } }),
}));
const at = "2026-11-06T00:00:00Z";
const agent = assistantAgentSchema.parse({
  id: "agent",
  name: "Researcher",
  kind: "specialist",
  status: "idle",
  created_at: at,
  last_active_at: at,
});
function channel(status = "active", org = true) {
  return nyxAgentChannelAgentSchema.parse({
    id: "channel",
    channel_bot_id: "bot",
    platform: "lark",
    bot_label: "Helper",
    transport: "direct",
    status,
    agent_id: "agent",
    private_chats: "owner",
    owner_linked: false,
    created_at: at,
    ...(org
      ? {
          org_specialist: {
            org_id: "org",
            guest_enabled: false,
            daily_credits: "4",
            delegated_service_ids: [],
          },
        }
      : {}),
  });
}
let client: QueryClient;
let settings: OrgChannelSettings;
let failRead: boolean;
let conflict: boolean;
let writes: {
  endpoint: string;
  method: string;
  body: Record<string, unknown>;
}[];
beforeEach(() => {
  client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  settings = orgChannelSettingsSchema.parse({
    channel_id: "channel",
    agent_id: "agent",
    grants_ready: true,
    guest_policy_mode: "grants",
    effective_guest_services: [
      {
        id: "service",
        label: "Calendar",
        access: "use",
        limited_operations: false,
      },
    ],
    org_id: "org",
    can_manage: true,
    guest_revision: "revision-1",
    channel_status: "active",
    guest_enabled: false,
    delegated_service_ids: [],
    daily_credits: null,
    default_daily_credits: "3",
    spent_today_credits: "0.25",
    reserved_today_credits: "0.1",
    readiness_reason: null,
    available_services: [{ id: "service", label: "Public lookup" }],
  });
  failRead = false;
  conflict = false;
  writes = [];
  globalThis.__nyxidAssistantHttpMock = ({ endpoint, init }) => {
    const method = init.method ?? "GET";
    if (method === "PUT" || method === "PATCH") {
      const body = JSON.parse(String(init.body));
      writes.push({ endpoint, method, body });
      if (method === "PATCH")
        return new Response(
          JSON.stringify({ ...body, channel_agent_id: "channel" }),
        );
      if (conflict)
        return new Response(
          JSON.stringify({
            error: "conflict",
            message: "Guest settings changed; reload them before saving",
          }),
          { status: 409 },
        );
      settings = {
        ...settings,
        guest_enabled: body.enabled,
        guest_revision: "revision-2",
      };
    }
    if (endpoint.includes("guest-readiness"))
      return new Response(
        JSON.stringify({
          status: "setting_up",
          configuration: {
            instructions_present: false,
            persona_present: false,
            memory_notes: 0,
            skills: 0,
            service_grants: 0,
            account_read: false,
          },
        }),
      );
    if (failRead)
      return new Response(
        JSON.stringify({ message: "Settings temporarily unavailable" }),
        { status: 503 },
      );
    return new Response(JSON.stringify(settings));
  };
});
afterEach(() => {
  cleanup();
  client.clear();
  globalThis.__nyxidAssistantHttpMock = undefined;
});
function mount(status = "active", org = true, onReconnect = vi.fn()) {
  render(
    <QueryClientProvider client={client}>
      <BotAudienceSettings
        channel={channel(status, org)}
        agent={agent}
        canManage
        onReconnect={onReconnect}
      />
    </QueryClientProvider>,
  );
  return onReconnect;
}

it("enables with a switch-only revision-bound request and shows Grants before enabling", async () => {
  mount();
  expect(await screen.findByText(/Enabling guests applies/)).toBeVisible();
  expect(screen.getByText(/Guests can use/)).toHaveTextContent("Calendar");
  expect(
    screen.getByText(/The organization pays for their messages/),
  ).toBeVisible();
  expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
  expect(screen.queryByRole("checkbox")).not.toBeInTheDocument();
  expect(screen.queryByText("Daily guest budget")).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("switch", { name: "Allow guests" }));
  await waitFor(() =>
    expect(writes[0]?.body).toEqual({
      enabled: true,
      expected_revision: "revision-1",
    }),
  );
  expect(
    screen.getByRole("link", { name: /Agent details → Grants/ }),
  ).toHaveAttribute("href", "/assistant?agent=agent&details=grants");
});
it("shows one legacy notice without a budget or service editor", async () => {
  settings.guest_policy_mode = "legacy";
  mount();
  expect(
    await screen.findByText(
      "Existing guest limits are kept. Review them in Grants.",
    ),
  ).toBeVisible();
  expect(screen.queryByText(/Enabling guests applies/)).not.toBeInTheDocument();
  expect(screen.queryByRole("checkbox")).not.toBeInTheDocument();
  expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
});
it("never reveals service IDs or budget controls while loading fails", async () => {
  failRead = true;
  mount();
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "NyxBot is temporarily unavailable. Try again shortly.",
  );
  expect(screen.queryByText("Delegated services")).not.toBeInTheDocument();
  expect(screen.queryByText("Daily guest budget")).not.toBeInTheDocument();
  expect(screen.getByRole("switch", { name: "Allow guests" })).toBeDisabled();
});
it.each(["failed", "pending"])(
  "keeps %s org settings readable but disabled",
  async (status) => {
    mount(status);
    await screen.findByText(/Enabling guests applies/);
    expect(screen.getByRole("switch", { name: "Allow guests" })).toBeDisabled();
  },
);
it("keeps member settings read-only", async () => {
  settings.can_manage = false;
  mount();
  await screen.findByText(/Enabling guests applies/);
  expect(screen.getByRole("switch", { name: "Allow guests" })).toBeDisabled();
});
it("explains missing wallet and permits disabling during rollout loss", async () => {
  settings.readiness_reason = "org_wallet_not_provisioned";
  settings.grants_ready = false;
  settings.guest_enabled = true;
  mount();
  expect(
    await screen.findByText(/Provision the organization billing wallet/),
  ).toBeVisible();
  expect(screen.getByText(/legacy daily cap still apply/)).toBeVisible();
  expect(
    screen.getByRole("switch", { name: "Allow guests" }),
  ).not.toBeDisabled();
  await userEvent.click(screen.getByRole("switch", { name: "Allow guests" }));
  await waitFor(() => expect(writes[0]?.body.enabled).toBe(false));
});
it("refuses enabling with an unavailable wallet", async () => {
  settings.readiness_reason = "org_wallet_suspended";
  mount();
  await screen.findByText("The organization wallet is suspended.");
  expect(screen.getByRole("switch", { name: "Allow guests" })).toBeDisabled();
});
it("keeps the switch state on a stale-revision conflict", async () => {
  conflict = true;
  mount();
  await screen.findByText(/Enabling guests applies/);
  await userEvent.click(screen.getByRole("switch", { name: "Allow guests" }));
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Guest settings changed",
  );
  expect(
    screen.getByRole("switch", { name: "Allow guests" }),
  ).not.toBeChecked();
});
it("explains personal billing, links specialist grants and saves private audience immediately", async () => {
  mount("active", false);
  expect(screen.getByText(/Guests use your billing/)).toBeVisible();
  expect(
    screen.queryByText(/paid by the organization/),
  ).not.toBeInTheDocument();
  expect(
    screen.getByRole("link", { name: /Agent details → Grants/ }),
  ).toHaveAttribute("href", "/assistant?agent=agent&details=grants");
  await userEvent.click(
    screen.getByRole("combobox", {
      name: "Who can talk to Helper in private chats",
    }),
  );
  await userEvent.click(screen.getByRole("option", { name: "Anyone" }));
  await waitFor(() =>
    expect(writes[0]).toEqual({
      endpoint: "/assistant/nyxagent/channels/channel",
      method: "PATCH",
      body: { private_chats: "everyone" },
    }),
  );
});
it.each(["failed", "pending"])(
  "keeps a %s personal audience visible and read-only",
  (status) => {
    mount(status, false);
    expect(
      screen.getByRole("combobox", {
        name: "Who can talk to Helper in private chats",
      }),
    ).toBeDisabled();
    expect(screen.getByText(/Guests use your billing/)).toBeVisible();
    expect(writes).toEqual([]);
  },
);

it("shows model setup and missing onboarding facts beside the immediate audience choice", async () => {
  mount("active", false);
  expect(await screen.findByText("Setting up for visitors…")).toBeTruthy();
  expect(
    screen.getByText(/Instructions: missing; persona: missing/),
  ).toBeTruthy();
  expect(
    screen.getByRole("combobox", {
      name: "Who can talk to Helper in private chats",
    }),
  ).not.toBeDisabled();
  expect(screen.getByText(/Opening access is your choice/)).toBeTruthy();
});
