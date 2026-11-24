import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { api } from "@/lib/api-client";
import { MetaCustomerSetup } from "./meta-customer-setup";

vi.mock("@/hooks/use-nyxbot-agents", () => ({
  useNyxBotAgents: () => ({
    isPending: false,
    data: {
      agents: [
        {
          id: "personal",
          name: "Personal support",
          kind: "specialist",
          status: "active",
          owner_kind: "person",
        },
        {
          id: "org",
          name: "Organization support",
          kind: "specialist",
          status: "active",
          owner_kind: "org",
          owner_id: "org",
          org_role: "admin",
        },
      ],
    },
  }),
}));
vi.mock("@/components/assistant/org-specialist-channel-settings", () => ({
  OrgSpecialistChannelSettings: ({ id }: { id: string }) => (
    <div>Member verification for {id}</div>
  ),
}));
let client: QueryClient;
let actions: string[];
let ready: boolean;
let enabled: boolean;
let failEnable: boolean;
beforeEach(() => {
  client = new QueryClient({ defaultOptions: { mutations: { retry: false } } });
  actions = [];
  ready = false;
  enabled = false;
  failEnable = false;
  vi.spyOn(api, "post").mockImplementation(async (endpoint, body) => {
    if (endpoint !== "/channel-bots/bot/customer-setup")
      throw new Error(endpoint);
    const input = body as { action: string; agent_id: string };
    actions.push(input.action);
    if (input.action === "prepare_model") ready = true;
    if (input.action === "enable_guests") {
      if (failEnable) throw new Error("Binding changed; review readiness");
      enabled = true;
    }
    return {
      channel_id: "channel",
      agent_id: input.agent_id,
      guests_enabled: enabled,
      readiness:
        input.agent_id === "org"
          ? { grants_ready: ready, readiness_reason: null }
          : { guest_readiness: { status: ready ? "ready" : "needs_setup" } },
      owner_link_code:
        ["assign", "owner_link"].includes(input.action) &&
        input.agent_id === "personal"
          ? "nyxlink_once"
          : null,
      owner_link_expires_at: "2026-12-01T00:00:00Z",
    };
  });
});
afterEach(() => {
  cleanup();
  client.clear();
  vi.restoreAllMocks();
});
function view(orgId: string | null = null) {
  const form = (botId: string) => (
    <QueryClientProvider client={client}>
      <MetaCustomerSetup botId={botId} orgId={orgId} />
    </QueryClientProvider>
  );
  const { rerender } = render(form("bot"));
  return (botId: string) => rerender(form(botId));
}
async function assign(id: string) {
  await userEvent.selectOptions(
    screen.getByLabelText("Support specialist"),
    id,
  );
  await userEvent.click(
    screen.getByRole("button", {
      name: "Assign specialist and check readiness",
    }),
  );
}
it("prepares before enabling and conveys the single-use owner code", async () => {
  view();
  await assign("personal");
  expect(await screen.findByText("nyxlink_once")).toBeVisible();
  expect(
    screen.getByRole("button", { name: "Enable customer conversations" }),
  ).toBeDisabled();
  await userEvent.click(
    screen.getByRole("button", { name: "Prepare guest model" }),
  );
  await waitFor(() => expect(screen.getByRole("checkbox")).toBeEnabled());
  await userEvent.click(screen.getByRole("checkbox"));
  await userEvent.click(
    screen.getByRole("button", { name: "Enable customer conversations" }),
  );
  expect(
    await screen.findByText(/Customer conversations are enabled/),
  ).toBeVisible();
  await userEvent.click(
    screen.getByRole("button", { name: "Check readiness" }),
  );
  await waitFor(() =>
    expect(actions).toEqual([
      "assign",
      "prepare_model",
      "enable_guests",
      "check",
    ]),
  );
  expect(screen.getByText("nyxlink_once")).toBeVisible();
  await userEvent.click(
    screen.getByRole("button", {
      name: "Create a fresh owner verification code",
    }),
  );
  await waitFor(() => expect(actions.at(-1)).toBe("owner_link"));
});
it("preserves committed enabled state after failed enable", async () => {
  ready = true;
  enabled = true;
  view();
  await assign("personal");
  expect(
    await screen.findByText(/Customer conversations are enabled/),
  ).toBeVisible();
  failEnable = true;
  await userEvent.click(screen.getByRole("checkbox"));
  await userEvent.click(
    screen.getByRole("button", { name: "Enable customer conversations" }),
  );
  expect(
    await screen.findByText("Binding changed; review readiness"),
  ).toBeVisible();
  expect(screen.getByText(/Customer conversations are enabled/)).toBeVisible();
});
it("restricts org scope and exposes member verification", async () => {
  ready = true;
  view("org");
  expect(
    screen.queryByRole("option", { name: "Personal support" }),
  ).not.toBeInTheDocument();
  await assign("org");
  expect(
    await screen.findByText("Member verification for channel"),
  ).toBeVisible();
  expect(
    screen.queryByRole("region", { name: "Owner verification" }),
  ).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("checkbox"));
  await userEvent.click(
    screen.getByRole("button", { name: "Enable customer conversations" }),
  );
  expect(
    await screen.findByText(/Customer conversations are enabled/),
  ).toBeVisible();
  expect(actions).toEqual(["assign", "enable_guests"]);
});

it("clears the previous specialist's readiness and management controls when the selection changes", async () => {
  ready = true;
  enabled = true;
  view("org");
  await assign("org");
  expect(await screen.findByText(/Customer conversations are enabled/)).toBeVisible();
  await userEvent.selectOptions(screen.getByLabelText("Support specialist"), "");
  expect(screen.queryByText(/Customer conversations are enabled/)).toBeNull();
  expect(screen.queryByText(/Member verification for channel/)).toBeNull();
  expect(screen.getByRole("checkbox")).toBeDisabled();
});

it("requires a new assignment and consent when navigating to another inbox", async () => {
  ready = true;
  const navigate = view();
  await assign("personal");
  await userEvent.click(screen.getByRole("checkbox"));
  expect(screen.getByText("nyxlink_once")).toBeVisible();
  navigate("another-bot");
  expect(screen.queryByText("nyxlink_once")).toBeNull();
  expect(screen.getByLabelText("Support specialist")).toHaveValue("");
  expect(screen.getByRole("checkbox")).not.toBeChecked();
  expect(screen.getByRole("button", { name: "Enable customer conversations" })).toBeDisabled();
});
