import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { CollaborationGuestControls } from "./collaboration-guest-controls";

vi.mock("@/hooks/use-feature-flag", () => ({ useFeature: () => true }));
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (select: (s: { user: { id: string } }) => unknown) =>
    select({ user: { id: "outside-owner" } }),
}));
let client: QueryClient;
let pending: boolean;
let enabled: boolean;
let writes: { endpoint: string; method: string; body: unknown }[];
beforeEach(() => {
  client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  pending = true;
  enabled = false;
  writes = [];
  globalThis.__nyxidAssistantHttpMock = ({ endpoint, init }) => {
    if (init.method && init.method !== "GET") {
      writes.push({
        endpoint,
        method: init.method,
        body: init.body ? JSON.parse(String(init.body)) : null,
      });
      return new Response(JSON.stringify({ group: null, invite: null }));
    }
    return new Response(
      JSON.stringify({
        groups: [
          {
            group_id: "group",
            membership_revision: 3,
            guest_revision: 2,
            enabled,
            request: pending
              ? {
                  id: "request",
                  status: "pending",
                  expires_at: "2027-01-01T00:00:00Z",
                  can_cancel: true,
                }
              : null,
          },
        ],
      }),
    );
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
      <CollaborationGuestControls agentId="agent" />
    </QueryClientProvider>,
  );
}
it("lets an outside owner approve a pending request with one explicit decision and its binding", async () => {
  view();
  const approve = await screen.findByRole("button", {
    name: "Approve participant access",
  });
  expect(screen.getByText(/They use your identity and billing/)).toBeVisible();
  expect(writes).toHaveLength(0);
  await userEvent.click(approve);
  await waitFor(() =>
    expect(writes[0]).toEqual({
      endpoint: "/assistant/nyxagent/groups/group/membership",
      method: "POST",
      body: {
        action: "guest_access",
        agent_id: "agent",
        enabled: true,
        expected_revision: 3,
        guest_revision: 2,
        request_id: "request",
      },
    }),
  );
});
it("can cancel the request without enabling the policy", async () => {
  view();
  await userEvent.click(
    await screen.findByRole("button", { name: "Cancel request" }),
  );
  await waitFor(() =>
    expect(writes[0]).toMatchObject({
      endpoint:
        "/assistant/nyxagent/groups/group/agents/agent/guest-requests/request",
      method: "DELETE",
    }),
  );
});
it("preserves the existing confirmation for enabling without a request", async () => {
  pending = false;
  view();
  await userEvent.click(
    await screen.findByRole("button", { name: "Enable guests" }),
  );
  expect(writes).toHaveLength(0);
  expect(screen.getByRole("dialog")).toHaveTextContent("and you pay");
});

it("shows revocation if an older replica already enabled a policy with a pending request", async () => {
  enabled = true;
  view();
  expect(await screen.findByRole("button", { name: "Disable guests" })).toBeVisible();
  expect(screen.queryByRole("button", { name: "Approve participant access" })).not.toBeInTheDocument();
});
