import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useAuthStore } from "@/stores/auth-store";
import type { AssistantAgent, AssistantGroup } from "@/schemas/assistant-nyxagent";
import { GroupSettingsDialog, NewGroupDialog } from "./nyxbot-group-forms";

const at = "2026-09-29T00:00:00Z";

function agent(fields: Partial<AssistantAgent>): AssistantAgent {
  return {
    id: "agent-nyxbot",
    kind: "nyxbot",
    name: "NyxBot",
    description: "",
    display_name: null,
    persona: null,
    specialty: null,
    created_by: "user",
    status: "idle",
    services: [],
    account_read: true,
    guest_access: {},
    pending_requests: [],
    last_reply: null,
    home_conversation_id: null,
    memory_count: 0,
    created_at: at,
    last_active_at: at,
    destroyed_at: null,
    pending_acknowledgements: 0,
    channels: [],
    ...fields,
  };
}

const AGENTS = [
  agent({}),
  agent({ id: "agent-researcher", kind: "specialist", name: "researcher", description: "Finds issues" }),
  agent({ id: "agent-old", kind: "specialist", name: "old", status: "destroyed", destroyed_at: at }),
];

const GROUP: AssistantGroup = {
  id: "nyxg-1",
  name: "Launch crew",
  members: [
    { id: "agent-nyxbot", name: "NyxBot", kind: "nyxbot", destroyed: false, working: false },
    { id: "agent-old", name: "old", kind: "specialist", destroyed: true, working: false },
  ],
  lead_agent_id: "agent-nyxbot",
  working_agent_ids: [],
  message_count: 1,
  last_message_at: at,
  created_at: at,
};

let requests: { method: string; endpoint: string; body: unknown }[];
let respond: (method: string) => Response;
let client: QueryClient;

beforeEach(() => {
  useAuthStore.getState().setUser({
    id: "owner",
    email: "owner@example.com",
    display_name: "Owner",
    avatar_url: null,
    email_verified: true,
    mfa_enabled: false,
    is_admin: false,
    is_active: true,
    created_at: at,
  });
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  requests = [];
  respond = (method) =>
    method === "DELETE"
      ? new Response(null, { status: 204 })
      : new Response(JSON.stringify(GROUP), { status: method === "POST" ? 201 : 200 });
  globalThis.__nyxidAssistantHttpMock = ({ endpoint, init }) => {
    const method = init.method ?? "GET";
    requests.push({ method, endpoint, body: init.body ? JSON.parse(String(init.body)) : undefined });
    return respond(method);
  };
});

afterEach(() => {
  cleanup();
  client.clear();
  globalThis.__nyxidAssistantHttpMock = undefined;
  useAuthStore.getState().setUser(null);
});

it("preselects NyxBot, offers live agents only, and creates the group", async () => {
  const user = userEvent.setup();
  const onCreated = vi.fn();
  render(
    <QueryClientProvider client={client}>
      <NewGroupDialog agents={AGENTS} onClose={vi.fn()} onCreated={onCreated} />
    </QueryClientProvider>,
  );
  const dialog = screen.getByRole("dialog", { name: "New group" });
  expect(within(dialog).getByRole("checkbox", { name: /NyxBot/ })).toBeChecked();
  expect(within(dialog).queryByRole("checkbox", { name: /old/ })).not.toBeInTheDocument();
  const create = within(dialog).getByRole("button", { name: "Create group" });
  expect(create).toBeDisabled();
  await user.type(within(dialog).getByRole("textbox", { name: "Name" }), "Launch crew");
  await user.click(within(dialog).getByRole("checkbox", { name: /researcher/ }));
  await user.click(create);
  await waitFor(() => expect(onCreated).toHaveBeenCalledWith(GROUP));
  expect(requests).toContainEqual({
    method: "POST",
    endpoint: "/assistant/nyxagent/groups",
    body: { name: "Launch crew", member_agent_ids: ["agent-nyxbot", "agent-researcher"] },
  });
});

it("shows the server's refusal", async () => {
  const user = userEvent.setup();
  respond = () =>
    new Response(JSON.stringify({ message: "researcher was destroyed" }), { status: 409 });
  render(
    <QueryClientProvider client={client}>
      <NewGroupDialog agents={AGENTS} onClose={vi.fn()} onCreated={vi.fn()} />
    </QueryClientProvider>,
  );
  await user.type(screen.getByRole("textbox", { name: "Name" }), "Crew");
  await user.click(screen.getByRole("button", { name: "Create group" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("researcher was destroyed");
});

it("saves only what changed", async () => {
  const user = userEvent.setup();
  const onOpenChange = vi.fn();
  const live = { ...GROUP, members: [GROUP.members[0]!] };
  render(
    <QueryClientProvider client={client}>
      <GroupSettingsDialog
        group={live}
        agents={AGENTS}
        open
        onOpenChange={onOpenChange}
        onDeleted={vi.fn()}
      />
    </QueryClientProvider>,
  );
  const dialog = screen.getByRole("dialog", { name: "Group settings" });
  const save = within(dialog).getByRole("button", { name: "Save" });
  expect(save).toBeDisabled();
  const name = within(dialog).getByRole("textbox", { name: "Name" });
  await user.clear(name);
  await user.type(name, "Release crew");
  await user.click(save);
  await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false));
  expect(requests).toContainEqual({
    method: "PATCH",
    endpoint: "/assistant/nyxagent/groups/nyxg-1",
    body: { name: "Release crew" },
  });
});

it("shows a destroyed member unchecked and drops it on save (the server refuses it)", async () => {
  const user = userEvent.setup();
  const onOpenChange = vi.fn();
  render(
    <QueryClientProvider client={client}>
      <GroupSettingsDialog
        group={GROUP}
        agents={AGENTS}
        open
        onOpenChange={onOpenChange}
        onDeleted={vi.fn()}
      />
    </QueryClientProvider>,
  );
  const dialog = screen.getByRole("dialog", { name: "Group settings" });
  const old = within(dialog).getByRole("checkbox", { name: /old/ });
  expect(old).not.toBeChecked();
  expect(old).toBeDisabled();
  expect(dialog).toHaveTextContent("Destroyed — removed when you save");
  // Nothing else changed, but saving removes the destroyed member.
  const save = within(dialog).getByRole("button", { name: "Save" });
  expect(save).toBeEnabled();
  await user.click(save);
  await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false));
  expect(requests).toContainEqual({
    method: "PATCH",
    endpoint: "/assistant/nyxagent/groups/nyxg-1",
    body: { member_agent_ids: ["agent-nyxbot"] },
  });
});

it("deletes only after confirmation", async () => {
  const user = userEvent.setup();
  const onDeleted = vi.fn();
  render(
    <QueryClientProvider client={client}>
      <GroupSettingsDialog group={GROUP} agents={AGENTS} open onOpenChange={vi.fn()} onDeleted={onDeleted} />
    </QueryClientProvider>,
  );
  await user.click(
    within(screen.getByRole("region", { name: "Danger zone" })).getByRole("button", {
      name: "Delete",
    }),
  );
  expect(requests.some((request) => request.method === "DELETE")).toBe(false);
  const confirm = screen.getByRole("dialog", { name: "Delete Launch crew?" });
  await user.click(within(confirm).getByRole("button", { name: "Delete group" }));
  await waitFor(() => expect(onDeleted).toHaveBeenCalledOnce());
  expect(requests).toContainEqual({
    method: "DELETE",
    endpoint: "/assistant/nyxagent/groups/nyxg-1",
    body: undefined,
  });
});
