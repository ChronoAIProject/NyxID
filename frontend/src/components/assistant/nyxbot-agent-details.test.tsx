import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useAuthStore } from "@/stores/auth-store";
import { AgentDetailsSheet } from "./nyxbot-agent-details";

vi.mock("@/hooks/use-keys", () => ({
  useKeys: () => ({
    isPending: false,
    data: [
      { slug: "github", label: "GitHub", is_active: true },
      { slug: "slack", label: "Slack", is_active: true },
    ],
  }),
}));
vi.mock("@/hooks/use-channel-bots", () => ({
  useChannelBots: () => ({ isPending: false, error: null, refetch: vi.fn(), data: [] }),
}));

const at = "2026-09-28T00:00:00Z";
let agent: Record<string, unknown>;
let memory: { id: string; text: string; created_at: string; updated_at: string }[];
let writes: { method: string; endpoint: string; body: unknown }[];
let client: QueryClient;

function agentRow(fields: Record<string, unknown>) {
  return {
    id: "agent-researcher",
    kind: "specialist",
    name: "researcher",
    description: "Finds urgent issues",
    specialty: null,
    created_by: "nyxbot",
    status: "idle",
    services: ["github"],
    account_read: false,
    guest_access: { github: "read" },
    pending_requests: [],
    last_reply: null,
    home_conversation_id: null,
    memory_count: 2,
    created_at: at,
    last_active_at: at,
    destroyed_at: null,
    pending_acknowledgements: 0,
    channels: [],
    ...fields,
  };
}

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
  writes = [];
  agent = agentRow({});
  memory = [
    { id: "n1", text: "Weekly digests on Mondays", created_at: at, updated_at: at },
    { id: "n2", text: "Label urgent issues p0", created_at: at, updated_at: at },
  ];
  globalThis.__nyxidAssistantHttpMock = ({ endpoint, init }) => {
    const method = init.method ?? "GET";
    const body = init.body ? (JSON.parse(String(init.body)) as unknown) : null;
    if (method !== "GET") writes.push({ method, endpoint, body });
    const id = String(agent.id);
    if (endpoint === `/assistant/nyxagent/agents/${id}/memory/n1` && method === "DELETE") {
      memory = memory.filter((note) => note.id !== "n1");
      return new Response(null, { status: 204 });
    }
    if (endpoint === `/assistant/nyxagent/agents/${id}/grants`) {
      agent = { ...agent, ...(body as object) };
      return new Response(JSON.stringify({ id, ...(body as object) }));
    }
    if (endpoint === `/assistant/nyxagent/agents/${id}/destroy`) {
      agent = { ...agent, status: "destroyed", destroyed_at: at };
      return new Response(JSON.stringify({ id, destroyed_at: at }));
    }
    if (endpoint === `/assistant/nyxagent/agents/${id}` && method === "DELETE") {
      return new Response(null, { status: 204 });
    }
    if (endpoint === `/assistant/nyxagent/agents/${id}` && method === "PATCH") {
      agent = { ...agent, ...(body as object) };
      return new Response(JSON.stringify({ id, name: agent.name, description: agent.description }));
    }
    if (endpoint === `/assistant/nyxagent/agents/${id}`) {
      return new Response(JSON.stringify({ agent, memory, threads: [] }));
    }
    if (endpoint === "/assistant/nyxagent/channels") {
      return new Response(JSON.stringify({ channel_agents: [] }));
    }
    return new Response(JSON.stringify({ message: "not found" }), { status: 404 });
  };
});

afterEach(() => {
  cleanup();
  client.clear();
  globalThis.__nyxidAssistantHttpMock = undefined;
  useAuthStore.getState().setUser(null);
});

function renderSheet() {
  const onDeleted = vi.fn();
  render(
    <QueryClientProvider client={client}>
      <AgentDetailsSheet
        agentId={String(agent.id)}
        agents={[]}
        open
        onOpenChange={vi.fn()}
        onDeleted={onDeleted}
      />
    </QueryClientProvider>,
  );
  return { onDeleted, user: userEvent.setup() };
}

it("forgets a memory note", async () => {
  const { user } = renderSheet();
  const sheet = await screen.findByRole("dialog", { name: "Agent details" });
  const notes = await within(sheet).findByRole("list", { name: "researcher memory" });
  expect(notes).toHaveTextContent("Weekly digests on Mondays");
  await user.click(within(notes).getByRole("button", { name: "Forget: Weekly digests on Mondays" }));
  await waitFor(() => expect(notes).not.toHaveTextContent("Weekly digests on Mondays"));
  expect(writes).toContainEqual({
    method: "DELETE",
    endpoint: "/assistant/nyxagent/agents/agent-researcher/memory/n1",
    body: null,
  });
});

it("replaces a specialist's grants with a dirty-gated save", async () => {
  const { user } = renderSheet();
  const sheet = await screen.findByRole("dialog", { name: "Agent details" });
  const grants = await within(sheet).findByRole("form", { name: "Grants" });
  const save = within(grants).getByRole("button", { name: "Save grants" });
  expect(save).toBeDisabled();
  expect(within(grants).getByRole("checkbox", { name: /GitHub/ })).toBeChecked();
  const guests = within(grants).getByRole("list", { name: "Guest access" });
  expect(within(guests).getByRole("combobox", { name: "github" })).toHaveTextContent(
    "Look things up only",
  );
  await user.click(within(grants).getByRole("checkbox", { name: /Slack/ }));
  // A newly granted service starts at the default level.
  expect(within(guests).getByRole("combobox", { name: "slack" })).toHaveTextContent(
    "Use, but not delete",
  );
  await user.click(within(grants).getByRole("switch", { name: "Read my account" }));
  expect(save).toBeEnabled();
  await user.click(save);
  await waitFor(() =>
    expect(writes).toContainEqual({
      method: "PUT",
      endpoint: "/assistant/nyxagent/agents/agent-researcher/grants",
      // Unchanged levels are not sent: the server keeps them.
      body: { services: ["github", "slack"], account_read: true },
    }),
  );
});

it("lets the owner choose what others in the agent's chats may do with each service", async () => {
  const { user } = renderSheet();
  const sheet = await screen.findByRole("dialog", { name: "Agent details" });
  const grants = await within(sheet).findByRole("form", { name: "Grants" });
  const save = within(grants).getByRole("button", { name: "Save grants" });
  expect(save).toBeDisabled();
  await user.click(within(grants).getByRole("combobox", { name: "github" }));
  await user.click(screen.getByRole("option", { name: "Everything this agent can do" }));
  expect(save).toBeEnabled();
  await user.click(save);
  await waitFor(() =>
    expect(writes).toContainEqual({
      method: "PUT",
      endpoint: "/assistant/nyxagent/agents/agent-researcher/grants",
      body: { services: ["github"], account_read: false, guest_access: { github: "all" } },
    }),
  );
});

it("edits the role and destroys only after confirmation", async () => {
  const { user } = renderSheet();
  const sheet = await screen.findByRole("dialog", { name: "Agent details" });
  const profile = await within(sheet).findByRole("form", { name: "Profile" });
  const description = within(profile).getByRole("textbox", { name: "Role" });
  await user.clear(description);
  await user.type(description, "Tracks p0 issues");
  await user.click(within(profile).getByRole("button", { name: "Save" }));
  await waitFor(() =>
    expect(writes).toContainEqual({
      method: "PATCH",
      endpoint: "/assistant/nyxagent/agents/agent-researcher",
      body: { description: "Tracks p0 issues" },
    }),
  );

  const danger = within(sheet).getByRole("region", { name: "Danger zone" });
  await user.click(within(danger).getByRole("button", { name: "Destroy" }));
  const confirm = await screen.findByRole("dialog", { name: "Destroy researcher?" });
  expect(writes.some((write) => write.endpoint.endsWith("/destroy"))).toBe(false);
  await user.click(within(confirm).getByRole("button", { name: "Destroy" }));
  await waitFor(() =>
    expect(writes).toContainEqual({
      method: "POST",
      endpoint: "/assistant/nyxagent/agents/agent-researcher/destroy",
      body: null,
    }),
  );
  // Destroyed agents offer permanent deletion instead.
  expect(
    await within(sheet).findByRole("button", { name: "Delete permanently" }),
  ).toBeVisible();
});

it("deletes a destroyed specialist permanently after confirmation", async () => {
  agent = agentRow({ status: "destroyed", destroyed_at: at });
  const { onDeleted, user } = renderSheet();
  const sheet = await screen.findByRole("dialog", { name: "Agent details" });
  await user.click(await within(sheet).findByRole("button", { name: "Delete permanently" }));
  const confirm = await screen.findByRole("dialog", {
    name: "Delete researcher permanently?",
  });
  await user.click(within(confirm).getByRole("button", { name: "Delete permanently" }));
  await waitFor(() => expect(onDeleted).toHaveBeenCalledOnce());
  expect(writes).toContainEqual({
    method: "DELETE",
    endpoint: "/assistant/nyxagent/agents/agent-researcher",
    body: null,
  });
});

it("sets a display name and persona, and clears them by emptying the fields", async () => {
  agent = agentRow({ display_name: "Scout", persona: "Brisk and factual." });
  const { user } = renderSheet();
  const sheet = await screen.findByRole("dialog", { name: "Agent details" });
  const profile = await within(sheet).findByRole("form", { name: "Profile" });
  // The display name heads the sheet with the @handle beside it.
  expect(sheet).toHaveTextContent("Scout@researcher");
  const displayName = within(profile).getByRole("textbox", { name: "Display name" });
  const persona = within(profile).getByRole("textbox", { name: "Persona" });
  expect(displayName).toHaveValue("Scout");
  expect(persona).toHaveValue("Brisk and factual.");
  await user.clear(displayName);
  await user.type(displayName, "Luna");
  await user.clear(persona);
  await user.click(within(profile).getByRole("button", { name: "Save" }));
  await waitFor(() =>
    expect(writes).toContainEqual({
      method: "PATCH",
      endpoint: "/assistant/nyxagent/agents/agent-researcher",
      body: { display_name: "Luna", persona: "" },
    }),
  );
});

it("shows the server's refusal of a display name", async () => {
  const { user } = renderSheet();
  const sheet = await screen.findByRole("dialog", { name: "Agent details" });
  const profile = await within(sheet).findByRole("form", { name: "Profile" });
  globalThis.__nyxidAssistantHttpMock = () =>
    new Response(JSON.stringify({ message: "A display name must not contain credentials" }), {
      status: 400,
    });
  await user.type(within(profile).getByRole("textbox", { name: "Display name" }), "sk-live");
  await user.click(within(profile).getByRole("button", { name: "Save" }));
  expect(await within(profile).findByRole("alert")).toHaveTextContent(
    "A display name must not contain credentials",
  );
});

it("lets NyxBot get a display name and persona, with full access and no grants or lifecycle actions", async () => {
  agent = agentRow({ id: "agent-nyxbot", kind: "nyxbot", name: "NyxBot", created_by: "user" });
  const { user } = renderSheet();
  const sheet = await screen.findByRole("dialog", { name: "Agent details" });
  const profile = await within(sheet).findByRole("form", { name: "Profile" });
  expect(within(sheet).queryByRole("textbox", { name: "Name" })).not.toBeInTheDocument();
  await user.type(within(profile).getByRole("textbox", { name: "Display name" }), "Nyx");
  await user.type(within(profile).getByRole("textbox", { name: "Persona" }), "Warm and concise.");
  await user.click(within(profile).getByRole("button", { name: "Save" }));
  await waitFor(() =>
    expect(writes).toContainEqual({
      method: "PATCH",
      endpoint: "/assistant/nyxagent/agents/agent-nyxbot",
      body: { display_name: "Nyx", persona: "Warm and concise." },
    }),
  );
  expect(within(sheet).getByRole("region", { name: "Access" })).toHaveTextContent("full access");
  expect(within(sheet).queryByRole("form", { name: "Grants" })).not.toBeInTheDocument();
  expect(within(sheet).queryByRole("region", { name: "Danger zone" })).not.toBeInTheDocument();
});
