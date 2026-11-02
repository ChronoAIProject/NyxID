import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import {
  CollaborationCards,
  CollaborationSettings,
  GroupInviteDialog,
} from "./collaboration-group-controls";
import { useAuthStore } from "@/stores/auth-store";
import { collaborationDecisionSchema } from "@/schemas/collaboration-group";
import type { AssistantGroup } from "@/schemas/assistant-nyxagent";

vi.mock("@/hooks/use-orgs", () => ({ useOrgs: () => ({ data: [] }) }));
vi.mock("@/hooks/use-org-members", () => ({
  useOrgMembers: () => ({ data: [] }),
}));
const group: AssistantGroup = {
  id: "group",
  name: "Research",
  members: [
    {
      id: "agent",
      name: "Researcher",
      kind: "specialist",
      destroyed: false,
      working: false,
    },
  ],
  participants: [
    { id: "owner", display_name: "Alice" },
    { id: "recipient", display_name: "Bob" },
  ],
  lead_agent_id: "agent",
  working_agent_ids: [],
  message_count: 0,
  last_message_at: null,
  created_at: "2026-10-31T00:00:00Z",
  collaboration: {
    version: 1,
    creator_user_id: "owner",
    membership_revision: 7,
    history_disclosure:
      "New participants can read the full retained group history.",
    can_manage: true,
    transfer_to_user_id: null,
    your_request_ids: [],
    proposals: [],
    invites: [],
    guest_access: [
      {
        agent_id: "agent",
        enabled: false,
        revision: 2,
        can_enable: true,
        can_disable: true,
        lane: "owner",
      },
    ],
  },
};
let client: QueryClient;
let requests: { endpoint: string; body: Record<string, unknown> }[];
beforeEach(() => {
  client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  requests = [];
  useAuthStore.getState().setUser({
    id: "owner",
    email: "owner@example.com",
    display_name: "Alice",
    avatar_url: null,
    email_verified: true,
    mfa_enabled: false,
    is_admin: false,
    is_active: true,
    created_at: group.created_at,
  });
  globalThis.__nyxidAssistantHttpMock = ({ endpoint, init }) => {
    requests.push({
      endpoint,
      body: init.body ? JSON.parse(String(init.body)) : {},
    });
    return new Response(JSON.stringify({ group, invite: null }));
  };
});
afterEach(() => {
  cleanup();
  client.clear();
  useAuthStore.getState().setUser(null);
  globalThis.__nyxidAssistantHttpMock = undefined;
});
const wrapper = ({ children }: { children: React.ReactNode }) => (
  <QueryClientProvider client={client}>{children}</QueryClientProvider>
);

it("asks the owner before enabling guests and binds both revisions", async () => {
  render(
    <CollaborationSettings
      group={group}
      agents={[]}
      onClose={vi.fn()}
      onLeft={vi.fn()}
    />,
    { wrapper },
  );
  await userEvent.click(
    screen.getByRole("button", { name: "Enable guest replies" }),
  );
  expect(requests).toHaveLength(0);
  expect(
    screen.getByText(/current and future participants/),
  ).toBeInTheDocument();
  expect(screen.getByText(/you pay/)).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Confirm" }));
  await waitFor(() =>
    expect(requests[0]?.body).toEqual({
      action: "guest_access",
      agent_id: "agent",
      enabled: true,
      guest_revision: 2,
      expected_revision: 7,
    }),
  );
});
it("keeps creator controls away from participants but always offers leave", () => {
  const participant = {
    ...group,
    collaboration: {
      ...group.collaboration!,
      can_manage: false,
      guest_access: [
        {
          ...group.collaboration!.guest_access[0]!,
          can_enable: false,
          can_disable: false,
        },
      ],
    },
  };
  render(
    <CollaborationSettings
      group={participant}
      agents={[]}
      onClose={vi.fn()}
      onLeft={vi.fn()}
    />,
    { wrapper },
  );
  expect(
    screen.queryByRole("button", { name: "Create invitation link" }),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Delete group" }),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Enable guest replies" }),
  ).not.toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: "Leave group" }),
  ).toBeInTheDocument();
});
it("shows only waiting status for someone else's card, even with unexpected private fields", () => {
  render(
    <CollaborationCards
      group={group}
      decisions={[
        {
          id: "private",
          kind: "consent",
          requester_id: "recipient",
          agent_id: "agent",
          can_decide: false,
          summary: "secret account detail",
          connection_id: "private-connection",
        },
      ]}
    />,
    { wrapper },
  );
  expect(screen.getByText("Waiting for Bob")).toBeInTheDocument();
  expect(
    screen.queryByText(/secret account|private-connection/),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Always" }),
  ).not.toBeInTheDocument();
});
it("shows the reply audience before an actor decides their consent", async () => {
  globalThis.__nyxidAssistantHttpMock = ({ endpoint, init }) => {
    requests.push({ endpoint, body: JSON.parse(String(init.body)) });
    return new Response(null, { status: 204 });
  };
  render(
    <CollaborationCards
      group={group}
      decisions={[
        {
          id: "consent",
          kind: "consent",
          requester_id: "owner",
          agent_id: "agent",
          can_decide: true,
          summary: "POST /append",
          connection_id: "my-docs",
          audience: "Everyone in this group can see the reply.",
        },
      ]}
    />,
    { wrapper },
  );
  expect(
    screen.getByText("Everyone in this group can see the reply."),
  ).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Allow once" }));
  await waitFor(() =>
    expect(requests[0]).toMatchObject({
      endpoint: "/assistant/nyxagent/groups/group/decisions/consent",
      body: { kind: "consent", decision: "once" },
    }),
  );
});
it("requires history acknowledgement after invite preview and before acceptance", async () => {
  const joined = vi.fn();
  globalThis.__nyxidAssistantHttpMock = ({ endpoint, init }) => {
    requests.push({ endpoint, body: JSON.parse(String(init.body)) });
    return new Response(
      JSON.stringify(
        endpoint.endsWith("preview")
          ? {
              group_id: group.id,
              group_name: group.name,
              expires_at: group.created_at,
              history_disclosure: group.collaboration!.history_disclosure,
              creator_name: "Alice",
              participants: group.participants,
            }
          : group,
      ),
    );
  };
  render(
    <GroupInviteDialog
      token="one-time-token"
      onClose={vi.fn()}
      onJoined={joined}
    />,
    { wrapper },
  );
  await userEvent.click(
    screen.getByRole("button", { name: "Preview invitation" }),
  );
  await screen.findByRole("button", { name: "Accept invitation" });
  expect(
    screen.getByRole("button", { name: "Accept invitation" }),
  ).toBeDisabled();
  await userEvent.click(screen.getByRole("checkbox"));
  await userEvent.click(
    screen.getByRole("button", { name: "Accept invitation" }),
  );
  await waitFor(() => expect(joined).toHaveBeenCalledWith(group));
  expect(requests[1]?.body).toEqual({
    token: "one-time-token",
    history_disclosed: true,
  });
});

it("renders consent when the connection label is no longer available", () => {
  const card = collaborationDecisionSchema.parse({
    id: "consent",
    kind: "consent",
    requester_id: "owner",
    agent_id: "agent",
    can_decide: true,
    summary: "Exact operation",
    connection_id: "my-connection",
    connection_name: null,
    audience: "Everyone in this group can see the reply.",
  });
  render(<CollaborationCards group={group} decisions={[card]} />, { wrapper });
  expect(
    screen.getByText("Your connection: my-connection"),
  ).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Skip" })).toBeInTheDocument();
});
