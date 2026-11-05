import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { OrgSpecialistChannelSettings } from "./org-specialist-channel-settings";
import { orgChannelGuestFormSchema } from "@/schemas/org-specialist-channel";

vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (select: (s: { user: { id: string } }) => unknown) =>
    select({ user: { id: "member" } }),
}));
let client: QueryClient;
let admin: boolean;
let reason: string | null;
let challengeState: string;
let expiresAt: string;
let memberVerified: boolean;
let settingsUnavailable: boolean;
let writes: { endpoint: string; method: string; body: unknown }[];
beforeEach(() => {
  client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  admin = true;
  reason = null;
  challengeState = "observed";
  expiresAt = new Date(Date.now() + 600_000).toISOString();
  memberVerified = false;
  settingsUnavailable = false;
  writes = [];
  globalThis.__nyxidAssistantHttpMock = ({ endpoint, init }) => {
    if (settingsUnavailable && endpoint.endsWith("/org-specialist")) {
      return new Response(JSON.stringify({ message: "Organization access is unavailable" }), { status: 403 });
    }
    if (init.method && init.method !== "GET")
      writes.push({
        endpoint,
        method: init.method,
        body: init.body ? JSON.parse(String(init.body)) : null,
      });
    const body = endpoint.endsWith("/org-specialist")
      ? {
          channel_id: "channel",
          org_id: "org",
          can_manage: admin,
          member_verified: memberVerified,
          guest_enabled: false,
          delegated_service_ids: [],
          daily_credits: null,
          default_daily_credits: "1",
          spent_today_credits: "0.000000000001",
          reserved_today_credits: "0.2",
          available_services: [{ id: "service", label: "Public lookup" }],
          readiness_reason: reason,
        }
      : endpoint.endsWith("/member-link")
        ? { id: "challenge", state: "issued", code: "nyxorg_test", expires_at: expiresAt }
        : endpoint.endsWith("/confirm")
          ? { confirmed: true }
          : { id: "challenge", state: challengeState, sender_label: "My chat" };
    return new Response(JSON.stringify(body));
  };
});
afterEach(() => {
  cleanup();
  client.clear();
  globalThis.__nyxidAssistantHttpMock = undefined;
});
function view() {
  render(
    <QueryClientProvider client={client}>
      <OrgSpecialistChannelSettings id="channel" />
    </QueryClientProvider>,
  );
}
it("lets an admin enable a model-only budget without granting services", async () => {
  view();
  expect(
    await screen.findByText(/Organization spend today: 0.000000000001/),
  ).toBeVisible();
  expect(
    screen.getByText(/organization pays for every guest model/),
  ).toBeVisible();
  await userEvent.click(
    screen.getByRole("checkbox", { name: "Enable guests for this binding" }),
  );
  await userEvent.click(
    screen.getByRole("button", { name: "Save guest access" }),
  );
  await waitFor(() =>
    expect(writes[0]).toEqual({
      endpoint: "/assistant/nyxagent/channels/channel/org-specialist",
      method: "PUT",
      body: { enabled: true, service_ids: [], daily_credits: "1" },
    }),
  );
});
it("members can confirm only their own chat account and cannot edit guest exposure", async () => {
  admin = false;
  reason = "org_wallet_not_provisioned";
  view();
  expect(await screen.findByText(/Only organization admins/)).toBeVisible();
  expect(screen.queryByRole("checkbox")).not.toBeInTheDocument();
  expect(screen.queryByRole("link", { name: "Open organization Billing" })).not.toBeInTheDocument();
  await userEvent.click(
    screen.getByRole("button", { name: "Verify my chat account" }),
  );
  await userEvent.click(
    await screen.findByRole("button", { name: "Confirm my account" }),
  );
  await waitFor(() =>
    expect(writes[1]).toMatchObject({
      endpoint:
        "/assistant/nyxagent/channels/channel/member-link/challenge/confirm",
      method: "POST",
      body: null,
    }),
  );
  expect(
    await screen.findByText("Your chat account is verified."),
  ).toBeVisible();
});
it("explains a missing wallet and disables guest enabling", async () => {
  reason = "org_wallet_not_provisioned";
  view();
  expect(
    await screen.findByText(/Provision the organization billing wallet/),
  ).toBeVisible();
  expect(screen.getByRole("link", { name: "Open organization Billing" })).toHaveAttribute("href", "/billing?ownerId=org");
  expect(
    screen.getByRole("checkbox", { name: "Enable guests for this binding" }),
  ).toBeDisabled();
});
it("keeps credit precision and rejects absent, zero and fractional-overflow budgets", () => {
  for (const daily_credits of [
    "",
    "0",
    "0.000",
    "-1",
    "0.0000000000001",
    "NaN",
  ])
    expect(
      orgChannelGuestFormSchema.safeParse({
        enabled: true,
        service_ids: [],
        daily_credits,
      }).success,
    ).toBe(false);
  expect(
    orgChannelGuestFormSchema.parse({
      enabled: true,
      service_ids: [],
      daily_credits: "0.000000000001",
    }).daily_credits,
  ).toBe("0.000000000001");
});


it("waits for an observed DM before offering confirmation", async () => {
  challengeState = "issued";
  view();
  await userEvent.click(screen.getByRole("button", { name: "Verify my chat account" }));
  expect(await screen.findByText(/Waiting for NyxID to observe your private message/)).toBeVisible();
  expect(screen.getByText(/Never send it to a group/)).toBeVisible();
  expect(screen.queryByRole("button", { name: "Confirm my account" })).not.toBeInTheDocument();
  expect(writes).toHaveLength(1);
});

it("hides expired codes and confirmation, then allows a fresh code without auto-confirming", async () => {
  expiresAt = new Date(Date.now() - 1000).toISOString();
  view();
  await userEvent.click(screen.getByRole("button", { name: "Verify my chat account" }));
  expect(await screen.findByText(/This verification code expired/)).toBeVisible();
  expect(screen.queryByRole("button", { name: "Confirm my account" })).not.toBeInTheDocument();
  expect(screen.queryByText("nyxorg_test")).not.toBeInTheDocument();
  expiresAt = new Date(Date.now() + 600_000).toISOString();
  await userEvent.click(screen.getByRole("button", { name: "Generate a new code" }));
  expect(await screen.findByRole("button", { name: "Confirm my account" })).toBeVisible();
  expect(writes).toHaveLength(2);
  expect(writes.every((write) => write.endpoint.endsWith("/member-link"))).toBe(true);
});

it("shows a persisted member verification after reloading without issuing a code", async () => {
  memberVerified = true;
  view();
  expect(await screen.findByText("Your chat account is verified.")).toBeVisible();
  expect(writes).toEqual([]);
});

it("does not keep a local confirmation success visible after losing access", async () => {
  view();
  await userEvent.click(screen.getByRole("button", { name: "Verify my chat account" }));
  await userEvent.click(await screen.findByRole("button", { name: "Confirm my account" }));
  expect(await screen.findByText("Your chat account is verified.")).toBeVisible();
  settingsUnavailable = true;
  await client.invalidateQueries({ queryKey: ["org-specialist-channel", "member", "channel"] });
  expect(await screen.findByRole("alert")).toHaveTextContent("Organization access is unavailable");
  expect(screen.queryByText("Your chat account is verified.")).not.toBeInTheDocument();
});
