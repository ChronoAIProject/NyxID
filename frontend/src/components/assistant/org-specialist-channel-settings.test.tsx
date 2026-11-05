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
let writes: { endpoint: string; method: string; body: unknown }[];
beforeEach(() => {
  client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  admin = true;
  reason = null;
  writes = [];
  globalThis.__nyxidAssistantHttpMock = ({ endpoint, init }) => {
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
        ? { id: "challenge", state: "issued", code: "nyxorg_test" }
        : endpoint.endsWith("/confirm")
          ? { confirmed: true }
          : { id: "challenge", state: "observed", sender_label: "My chat" };
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
  view();
  expect(await screen.findByText(/Only organization admins/)).toBeVisible();
  expect(screen.queryByRole("checkbox")).not.toBeInTheDocument();
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
