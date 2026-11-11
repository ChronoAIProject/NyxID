import {
  act,
  cleanup,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import {
  GroupedDraftBoundary,
  GroupedDraftDialog,
} from "@/components/ui/grouped-drafts";
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
const key = ["org-specialist-channel", "admin", "channel"];
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
        daily_credits: body.daily_credits,
        delegated_service_ids: body.service_ids,
        guest_revision: "revision-2",
      };
    }
    if (endpoint.includes("guest-readiness")) return new Response(JSON.stringify({
      status: "setting_up", configuration: { instructions_present: false, persona_present: false, memory_notes: 0, skills: 0, service_grants: 0, account_read: false },
    }));
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
async function budget() {
  return screen.findByRole("textbox", {
    name: "Daily guest budget (credits, UTC)",
  });
}
async function reviewSave() {
  await userEvent.click(screen.getByRole("button", { name: "Save changes" }));
  const dialog = await screen.findByRole("dialog", { name: "Review changes" });
  await userEvent.click(
    within(dialog).getByRole("button", { name: "Save changes" }),
  );
}

it("enables guests in one switch with the server default budget and organization billing copy", async () => {
  mount();
  expect(await budget()).toHaveValue("3");
  expect(
    screen.getByText(
      /Verified organization members use their own permissions and billing/,
    ),
  ).toBeVisible();
  expect(screen.getByText("0.25 credits (0.1 reserved)")).toBeVisible();
  expect(
    screen.queryByRole("button", { name: "Verify my chat account" }),
  ).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("switch", { name: "Allow guests" }));
  await waitFor(() =>
    expect(writes).toEqual([
      {
        endpoint: "/assistant/nyxagent/channels/channel/org-specialist",
        method: "PUT",
        body: {
          enabled: true,
          daily_credits: "3",
          service_ids: [],
          expected_revision: "revision-1",
        },
      },
    ]),
  );
  expect(screen.getByRole("switch", { name: "Allow guests" })).toBeChecked();
  expect(
    screen.queryByRole("region", { name: "Unsaved changes" }),
  ).not.toBeInTheDocument();
});
it.each(["active", "pending", "failed"])(
  "shows saved settings for a %s binding with recovery for inactive rows",
  async (status) => {
    settings.daily_credits = "7";
    settings.guest_enabled = true;
    settings.channel_status = status;
    const reconnect = mount(status);
    expect(await budget()).toHaveValue("7");
    expect(
      screen.getByRole("region", { name: "Who can talk to this bot · Helper" }),
    ).toBeVisible();
    if (status === "active")
      expect(
        screen.getByRole("switch", { name: "Allow guests" }),
      ).toBeEnabled();
    else {
      expect(await budget()).toBeDisabled();
      expect(
        screen.getByRole("switch", { name: "Allow guests" }),
      ).toBeDisabled();
      expect(
        screen.getByRole("checkbox", { name: "Public lookup" }),
      ).toBeDisabled();
      await userEvent.click(
        screen.getByRole("button", {
          name: status === "pending" ? "Finish connecting" : "Reconnect",
        }),
      );
      expect(reconnect).toHaveBeenCalledOnce();
    }
    expect(writes).toEqual([]);
  },
);
it("keeps listing settings visible when the settings request is unavailable", async () => {
  failRead = true;
  mount("failed");
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "temporarily unavailable",
  );
  expect(screen.getByText("4 credits")).toBeVisible();
  expect(screen.getByText("Model-only chat")).toBeVisible();
  expect(screen.getByRole("switch", { name: "Allow guests" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Reconnect" })).toBeEnabled();
});
it("preserves dirty drafts through refresh and reviews the save with its original revision", async () => {
  mount();
  const input = await budget();
  await userEvent.clear(input);
  await userEvent.type(input, "2.5");
  await userEvent.click(
    screen.getByRole("checkbox", { name: "Public lookup" }),
  );
  expect(screen.getByRole("region", { name: "Unsaved changes" })).toBeVisible();
  expect(screen.getByRole("switch", { name: "Allow guests" })).toBeDisabled();
  settings = {
    ...settings,
    spent_today_credits: "0.4",
    daily_credits: "8",
    guest_revision: "remote-edit",
  };
  await act(async () => {
    await client.invalidateQueries({ queryKey: key });
  });
  expect(input).toHaveValue("2.5");
  expect(screen.getByRole("checkbox", { name: "Public lookup" })).toBeChecked();
  expect(await screen.findByText("0.4 credits (0.1 reserved)")).toBeVisible();
  expect(writes).toEqual([]);
  await userEvent.click(screen.getByRole("button", { name: "Save changes" }));
  const dialog = await screen.findByRole("dialog", { name: "Review changes" });
  expect(dialog).toHaveTextContent("Daily guest budget");
  expect(dialog).toHaveTextContent("Public lookup");
  expect(writes).toEqual([]);
  await userEvent.click(
    within(dialog).getByRole("button", { name: "Save changes" }),
  );
  await waitFor(() =>
    expect(writes[0]?.body).toEqual({
      enabled: false,
      daily_credits: "2.5",
      service_ids: ["service"],
      expected_revision: "revision-1",
    }),
  );
});
it("retains conflicting edits until explicitly discarded to the latest settings", async () => {
  mount();
  const input = await budget();
  await userEvent.clear(input);
  await userEvent.type(input, "2");
  conflict = true;
  settings = { ...settings, daily_credits: "9", guest_revision: "remote-edit" };
  await reviewSave();
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Guest settings changed",
  );
  expect(input).toHaveValue("2");
  expect(screen.getByRole("region", { name: "Unsaved changes" })).toBeVisible();
  await userEvent.click(screen.getByRole("button", { name: "Discard" }));
  expect(input).toHaveValue("9");
  expect(
    screen.queryByRole("region", { name: "Unsaved changes" }),
  ).not.toBeInTheDocument();
});
it("keeps member controls read-only", async () => {
  settings.can_manage = false;
  mount();
  expect(await budget()).toBeDisabled();
  expect(
    screen.getByText("Only organization admins can change guest access."),
  ).toBeVisible();
  expect(screen.getByRole("switch", { name: "Allow guests" })).toBeDisabled();
  expect(writes).toEqual([]);
});
it("explains a missing wallet and prevents enabling guests", async () => {
  settings.readiness_reason = "org_wallet_not_provisioned";
  mount();
  await budget();
  expect(
    screen.getByText(/Provision the organization billing wallet/),
  ).toBeVisible();
  expect(
    screen.getByRole("link", { name: "Open organization Billing" }),
  ).toHaveAttribute("href", "/billing?ownerId=org");
  expect(screen.getByRole("switch", { name: "Allow guests" })).toBeDisabled();
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

it("keeps the saved switch state when immediate enabling fails", async () => {
  conflict = true;
  mount();
  await budget();
  await userEvent.click(screen.getByRole("switch", { name: "Allow guests" }));
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Guest settings changed",
  );
  expect(
    screen.getByRole("switch", { name: "Allow guests" }),
  ).not.toBeChecked();
  expect(writes).toHaveLength(1);
});

it("asks before reconnecting a failed binding with a retained dirty draft", async () => {
  const reconnect = vi.fn();
  const content = (status: string) => (
    <QueryClientProvider client={client}>
      <GroupedDraftBoundary>
        <BotAudienceSettings
          channel={channel(status)}
          agent={agent}
          canManage
          onReconnect={reconnect}
        />
        <GroupedDraftDialog />
      </GroupedDraftBoundary>
    </QueryClientProvider>
  );
  const view = render(content("active"));
  const input = await budget();
  await userEvent.clear(input);
  await userEvent.type(input, "5");
  view.rerender(content("failed"));
  await userEvent.click(screen.getByRole("button", { name: "Reconnect" }));
  let dialog = await screen.findByRole("dialog", {
    name: "Discard unsaved changes?",
  });
  await userEvent.click(
    within(dialog).getByRole("button", { name: "Keep editing" }),
  );
  expect(reconnect).not.toHaveBeenCalled();
  expect(input).toHaveValue("5");
  await userEvent.click(screen.getByRole("button", { name: "Reconnect" }));
  dialog = await screen.findByRole("dialog", {
    name: "Discard unsaved changes?",
  });
  await userEvent.click(
    within(dialog).getByRole("button", { name: "Discard" }),
  );
  await waitFor(() => expect(reconnect).toHaveBeenCalledOnce());
  expect(writes).toEqual([]);
});

it("shows model setup and missing onboarding facts beside the immediate audience choice", async () => {
  mount("active", false);
  expect(await screen.findByText("Setting up for visitors…")).toBeTruthy();
  expect(screen.getByText(/Instructions: missing; persona: missing/)).toBeTruthy();
  expect(screen.getByRole("combobox", { name: "Who can talk to Helper in private chats" })).not.toBeDisabled();
  expect(screen.getByText(/Opening access is your choice/)).toBeTruthy();
});
