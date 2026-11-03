import type { ComponentProps } from "react";
import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useAuthStore } from "@/stores/auth-store";
import { AgentDetailsSheet } from "./nyxbot-agent-details";

vi.mock("@tanstack/react-router", () => ({
  Link: ({ to, search, ...props }: ComponentProps<"a"> & { to: string; search?: Record<string, string> }) => (
    <a href={to + (search ? `?${new URLSearchParams(search)}` : "")} {...props} />
  ),
}));

vi.mock("@/hooks/use-keys", () => ({
  useKeys: () => ({
    isPending: false,
    data: [
      { id: "github-key", slug: "github", label: "GitHub", is_active: true },
      { id: "slack-key", slug: "slack", label: "Slack", is_active: true },
    ],
  }),
}));
vi.mock("@/hooks/use-channel-bots", () => ({
  useChannelBots: () => ({ isPending: false, error: null, refetch: vi.fn(), data: [] }),
}));

const at = "2026-09-28T00:00:00Z";
let agent: Record<string, unknown>;
let memory: { id: string; text: string; created_at: string; updated_at: string }[];
let readinessReads: number;
let writes: { method: string; endpoint: string; body: unknown }[];
let client: QueryClient;
let requestMode: "denied" | "routed_to_nyxbot";

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
    capabilities: { billing_available: false, enabled_features: ["assistant:model-controls"] },
  });
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  vi.stubGlobal("fetch", vi.fn(async (input: RequestInfo | URL) => {
    const path = new URL(input instanceof Request ? input.url : String(input)).pathname;
    const fixtures: Record<string, unknown> = {
      "/api/v1/nodes": { nodes: [] },
      "/api/v1/saved-logins": [],
      "/api/v1/triggers": { triggers: [] },
      "/api/v1/orgs": { orgs: [] },
    };
    if (!(path in fixtures)) throw new Error(`Unexpected account read: ${path}`);
    return new Response(JSON.stringify(fixtures[path]));
  }));
  writes = [];
  readinessReads = 0;
  requestMode = "denied";
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
    if (endpoint === `/assistant/nyxagent/agents/${id}/guest-readiness`) {
      readinessReads++;
      return new Response(JSON.stringify({ status: "not_ready", fix: "Send one message to this assistant yourself." }));
    }
    if (endpoint === `/assistant/nyxagent/agents/${id}/memory/n1` && method === "DELETE") {
      memory = memory.filter((note) => note.id !== "n1");
      return new Response(null, { status: 204 });
    }
    if (endpoint === `/assistant/nyxagent/agents/${id}/memory` &&
      method === "POST"
    ) {
      return new Response(JSON.stringify({ id: "new-note" }));
    }
    if (endpoint === `/assistant/nyxagent/agents/${id}/skills`) {
      return new Response(JSON.stringify({ revision: 0, skills: [], metadata: {} }));
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
    if (endpoint === "/assistant/nyxagent/settings") {
      return new Response(JSON.stringify({
        access_mode: "nyxmode",
        skip_destructive_confirmation: false,
        max_live_subagents: 8,
        max_concurrent_subagent_turns: 3,
        max_live_subagents_limit: 32,
        max_concurrent_subagent_turns_limit: 8,
        specialist_access_requests: requestMode,
      }));
    }
    if (endpoint === "/assistant/nyxagent/models") {
      return new Response(JSON.stringify([
        { id: "nyxagent/chat", label: "chat", available: true },
        { id: "nyxagent/research", label: "research", available: true },
        { id: "nyxagent/retired", label: "retired", available: false },
      ]));
    }
    return new Response(JSON.stringify({ message: "not found" }), { status: 404 });
  };
});

afterEach(() => {
  cleanup();
  client.clear();
  vi.unstubAllGlobals();
  globalThis.__nyxidAssistantHttpMock = undefined;
  useAuthStore.getState().setUser(null);
});

function renderSheet(props: Partial<ComponentProps<typeof AgentDetailsSheet>> = {}) {
  const onDeleted = vi.fn();
  render(
    <QueryClientProvider client={client}>
      <AgentDetailsSheet
        agentId={String(agent.id)}
        agents={[]}
        open
        onOpenChange={vi.fn()}
        onDeleted={onDeleted}
        {...props}
      />
    </QueryClientProvider>,
  );
  return { onDeleted, user: userEvent.setup() };
}

/**
 * No specialist-facing copy may tell the owner that NyxBot takes over a direct
 * chat: the retired phrases, plus generic takeover claims. Accurate statements
 * about work NyxBot assigned ("only work NyxBot assigned, or work in a group NyxBot is in, asks NyxBot") and
 * owner-directed tips ("ask NyxBot to change this") are allowed.
 */
const NYXBOT_TAKEOVER =
  /asks NyxBot for anything|ask NyxBot for access|reports the need to NyxBot|tells NyxBot in its reply|needs NyxBot|NyxBot (needs to|will|must) take over|hands? (it|the work) (over )?to NyxBot/;

it("forgets a memory note", async () => {
  const { user } = renderSheet({ tab: "memory" });
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
  const { user } = renderSheet({ tab: "grants" });
  const sheet = await screen.findByRole("dialog", { name: "Agent details" });
  const grants = await within(sheet).findByRole("form", { name: "Grants" });
  expect(
    screen.queryByRole("region", { name: "Unsaved changes" }),
  ).not.toBeInTheDocument();
  expect(within(grants).getByRole("checkbox", { name: /GitHub/ })).toBeChecked();
  const guests = within(grants).getByRole("list", { name: "Guest access" });
  expect(within(guests).getByRole("combobox", { name: "github" })).toHaveTextContent(
    "Look things up only",
  );
  await user.click(within(grants).getByRole("checkbox", { name: /Slack/ }));
  // A newly granted service starts at the default level.
  expect(within(guests).getByRole("combobox", { name: "slack" })).toHaveTextContent(
    "Use, but not change or delete",
  );
  await user.click(within(grants).getByRole("switch", { name: "Read my account" }));
  const save = screen.getByRole("button", { name: "Save changes" });
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
  const { user } = renderSheet({ tab: "grants" });
  const sheet = await screen.findByRole("dialog", { name: "Agent details" });
  const grants = await within(sheet).findByRole("form", { name: "Grants" });
  expect(
    screen.queryByRole("region", { name: "Unsaved changes" }),
  ).not.toBeInTheDocument();
  await user.click(within(grants).getByRole("combobox", { name: "github" }));
  await user.click(screen.getByRole("option", { name: "Everything this agent can do" }));
  const save = screen.getByRole("button", { name: "Save changes" });
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
  await user.click(screen.getByRole("button", { name: "Save changes" }));
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
  await user.click(screen.getByRole("button", { name: "Save changes" }));
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
  await user.click(screen.getByRole("button", { name: "Save changes" }));
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
  await user.click(screen.getByRole("button", { name: "Save changes" }));
  await waitFor(() =>
    expect(writes).toContainEqual({
      method: "PATCH",
      endpoint: "/assistant/nyxagent/agents/agent-nyxbot",
      body: { display_name: "Nyx", persona: "Warm and concise." },
    }),
  );
  await user.click(within(sheet).getByRole("tab", { name: "Access" }));
  expect(within(sheet).getByRole("region", { name: "Access" })).toHaveTextContent("full access");
  expect(within(sheet).queryByRole("form", { name: "Grants" })).not.toBeInTheDocument();
  expect(
    within(sheet).queryByRole("region", { name: "Danger zone" }),
  ).not.toBeInTheDocument();
});

it("shows an organization viewer a read-only profile without shared memory or lifecycle actions", async () => {
  agent = agentRow({
    owner_kind: "org",
    owner_name: "Team",
    owner_id: "org-team",
    org_role: "viewer",
    can_maintain: false,
    can_use: false,
  });
  renderSheet();
  const sheet = await screen.findByRole("dialog", { name: "Agent details" });
  expect(
    await within(sheet).findByText(/Maintained by Admins and Members/),
  ).toBeVisible();
  expect(within(sheet).getByRole("textbox", { name: "Role" })).toBeDisabled();
  expect(
    within(sheet).queryByRole("button", { name: "Save grants" }),
  ).not.toBeInTheDocument();
  expect(
    within(sheet).queryByRole("region", { name: "Danger zone" }),
  ).not.toBeInTheDocument();
  expect(
    within(sheet).queryByText("Weekly digests on Mondays"),
  ).not.toBeInTheDocument();
  expect(within(sheet).queryByText("Channel bots")).not.toBeInTheDocument();
  expect(writes).toEqual([]);
});

it("lets an organization Member edit shared memory with the privacy note and backend bound", async () => {
  agent = agentRow({
    owner_kind: "org",
    owner_name: "Team",
    owner_id: "org-team",
    org_role: "member",
    can_maintain: true,
    can_use: true,
  });
  const { user } = renderSheet({ tab: "memory" });
  const form = await screen.findByRole("form", { name: "Shared memory" });
  expect(screen.getByText(/Never save private member content/)).toBeVisible();
  const input = within(form).getByRole("textbox", { name: "Add shared note" });
  expect(input).toHaveAttribute("maxlength", "500");
  expect(
    screen.queryByRole("button", { name: "Save changes" }),
  ).not.toBeInTheDocument();
  await user.type(input, "Public team convention");
  await user.click(screen.getByRole("button", { name: "Save changes" }));
  await waitFor(() =>
    expect(writes).toContainEqual({
      method: "POST",
      endpoint: "/assistant/nyxagent/agents/agent-researcher/memory",
      body: { text: "Public team convention" },
    }),
  );
  expect(
    screen.queryByRole("switch", { name: "Read my account" }),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByRole("list", { name: "Guest access" }),
  ).not.toBeInTheDocument();
});

it("explains the specialist sandbox and who approves in each request policy", async () => {
  agent = agentRow({ owner_kind: "person", can_maintain: true, can_use: true });
  renderSheet({ tab: "grants" });
  const sheet = await screen.findByRole("dialog", { name: "Agent details" });
  const authority = await screen.findByRole("region", { name: "Access authority" });
  await waitFor(() =>
    expect(authority).toHaveTextContent(
      "When you talk to it directly, it tells you what it is missing and continues; you grant access here.",
    ),
  );
  expect(authority).toHaveTextContent("cannot grant access to itself or to other agents");
  expect(authority).toHaveTextContent("Its requests are never routed");
  // Direct chats never hand the work to NyxBot: no copy in the sheet says so.
  expect(sheet).not.toHaveTextContent(NYXBOT_TAKEOVER);
  expect(
    within(sheet).getByRole("region", { name: "Grants" }),
  ).toHaveTextContent(
    "only work NyxBot assigned, or work in a group NyxBot is in, reports the missing access to NyxBot instead",
  );
  cleanup();
  requestMode = "routed_to_nyxbot";
  client.clear();
  renderSheet({ tab: "grants" });
  const routed = await screen.findByRole("region", { name: "Access authority" });
  await waitFor(() =>
    expect(routed).toHaveTextContent(
      "Only for work NyxBot assigned, or in a group with NyxBot, do its requests go to NyxBot, which approves or denies them",
    ),
  );
  expect(routed).toHaveTextContent("When you talk to it directly, it tells you what it is missing");
  expect(screen.getByRole("dialog", { name: "Agent details" })).not.toHaveTextContent(NYXBOT_TAKEOVER);
});

it.each([
  ["overview", "Overview"],
  ["grants", "Access"],
  ["skills", "Skills"],
  ["memory", "Memory"],
  ["channels", "Channels"],
] as const)("lands a linked sheet on its %s section and focuses it", async (tab, name) => {
  renderSheet({ tab });
  const sheet = await screen.findByRole("dialog", { name: "Agent details" });
  const section = await within(sheet).findByRole("heading", { name, level: 2 });
  await waitFor(() => expect(section).toHaveFocus());
  // A specialist's grants form is there to edit; nothing in it is a NyxBot handover.
  expect(section).not.toHaveTextContent(NYXBOT_TAKEOVER);
});

it("shows an explicit no-access state for an agent the viewer cannot read, without retrying", async () => {
  const unknown = "agent-not-mine";
  globalThis.__nyxidAssistantHttpMock = ({ endpoint }) =>
    endpoint === `/assistant/nyxagent/agents/${unknown}`
      ? new Response(JSON.stringify({ message: "not found", error_code: 1000 }), { status: 404 })
      : new Response(JSON.stringify({ message: "not found", error_code: 1000 }), { status: 404 });
  renderSheet({ agentId: unknown, tab: "grants" });
  const sheet = await screen.findByRole("dialog", { name: "Agent details" });
  const note = await within(sheet).findByRole("note", { name: "No access" });
  expect(note).toHaveTextContent("You cannot view this agent.");
  expect(note).toHaveTextContent("Its grants and skills cannot be shown or changed here.");
  expect(sheet).toHaveTextContent("This agent is not available to you.");
  expect(within(sheet).queryByRole("button", { name: "Retry" })).not.toBeInTheDocument();
  expect(within(sheet).queryByRole("form", { name: "Grants" })).not.toBeInTheDocument();
  expect(within(sheet).queryByRole("region", { name: "Skills" })).not.toBeInTheDocument();
});

it("keeps the retryable banner for a transient agent read failure", async () => {
  globalThis.__nyxidAssistantHttpMock = () =>
    new Response(JSON.stringify({ message: "upstream timeout", error_code: 5000 }), { status: 503 });
  renderSheet();
  const sheet = await screen.findByRole("dialog", { name: "Agent details" });
  expect(await within(sheet).findByRole("button", { name: "Retry" })).toBeVisible();
  expect(within(sheet).queryByRole("note", { name: "No access" })).not.toBeInTheDocument();
});

it("explains that a destroyed specialist is read-only when a link lands on its grants", async () => {
  agent = agentRow({ status: "destroyed", destroyed_at: at });
  const { user } = renderSheet({ tab: "grants" });
  const sheet = await screen.findByRole("dialog", { name: "Agent details" });
  const note = await within(sheet).findByRole("note", { name: "Agent destroyed" });
  expect(note).toHaveTextContent("researcher was destroyed on");
  expect(note).toHaveTextContent("Its grants, skills and settings are read-only and it no longer runs. You can delete it permanently below.");
  const grants = within(sheet).getByRole("form", { name: "Grants" });
  expect(within(grants).getByRole("checkbox", { name: /GitHub/ })).toBeDisabled();
  expect(within(grants).queryByRole("button", { name: "Save grants" })).not.toBeInTheDocument();
  await user.click(within(sheet).getByRole("tab", { name: "Overview" }));
  expect(within(sheet).getByRole("textbox", { name: "Role" })).toBeDisabled();
});

it("explains that an organization viewer can see but not change grants and skills", async () => {
  agent = agentRow({
    owner_kind: "org",
    owner_name: "Team",
    owner_id: "org-team",
    org_role: "viewer",
    can_maintain: false,
    can_use: false,
  });
  const { user } = renderSheet({ tab: "skills" });
  const sheet = await screen.findByRole("dialog", { name: "Agent details" });
  const note = await within(sheet).findByRole("note", { name: "Read-only" });
  expect(note).toHaveTextContent("Only a maintainer of Team (an Admin or Member) can change its grants, skills and settings. You can view them.");
  expect(within(sheet).getByRole("region", { name: "Skills" })).toBeVisible();
  await user.click(within(sheet).getByRole("tab", { name: "Access" }));
  expect(within(sheet).getByRole("region", { name: "Grants" })).toBeVisible();
  expect(within(sheet).queryByRole("button", { name: "Save grants" })).not.toBeInTheDocument();
  expect(within(sheet).queryByRole("note", { name: "Agent destroyed" })).not.toBeInTheDocument();
  expect(writes).toEqual([]);
});

it("shows no read-only note for a maintainable live specialist", async () => {
  renderSheet({ tab: "grants" });
  const sheet = await screen.findByRole("dialog", { name: "Agent details" });
  await within(sheet).findByRole("form", { name: "Grants" });
  expect(within(sheet).queryByRole("note")).not.toBeInTheDocument();
});

it("names an organization maintainer as the approver for an organization viewer", async () => {
  agent = agentRow({
    owner_kind: "org",
    owner_name: "Team",
    owner_id: "org-team",
    org_role: "viewer",
    can_maintain: false,
    can_use: false,
  });
  renderSheet({ tab: "grants" });
  const authority = await screen.findByRole("region", { name: "Access authority" });
  expect(authority).toHaveTextContent(
    "Only a maintainer of Team (an Admin or Member) can approve or grant its access",
  );
});

it("selects the agent's model from the live list and its own access mode", async () => {
  agent = agentRow({ model: "nyxagent/chat", access_mode_override: null });
  const { user } = renderSheet();
  const profile = await screen.findByRole("form", { name: "Profile" });
  const model = within(profile).getByRole("combobox", { name: "Agent model" });
  await waitFor(() => expect(model).toHaveTextContent("Default model"));
  await user.click(model);
  expect(await screen.findByRole("option", { name: "Retired (unavailable)" })).toHaveAttribute(
    "aria-disabled",
    "true",
  );
  await user.click(screen.getByRole("option", { name: "Research" }));
  const mode = within(profile).getByRole("combobox", { name: "Agent access mode" });
  expect(mode).toHaveTextContent("Workspace setting (Auto)");
  await user.click(mode);
  await user.click(await screen.findByRole("option", { name: "Ask every time" }));
  await user.click(screen.getByRole("button", { name: "Save changes" }));
  await waitFor(() =>
    expect(writes).toContainEqual({
      method: "PATCH",
      endpoint: "/assistant/nyxagent/agents/agent-researcher",
      body: { model: "nyxagent/research", access_mode_override: "ask_every_time" },
    }),
  );
});

it("clears an agent's own access mode back to the workspace setting", async () => {
  agent = agentRow({ model: "nyxagent/chat", access_mode_override: "autopilot" });
  const { user } = renderSheet();
  const profile = await screen.findByRole("form", { name: "Profile" });
  const mode = within(profile).getByRole("combobox", { name: "Agent access mode" });
  expect(mode).toHaveTextContent("Full access");
  await user.click(mode);
  await user.click(await screen.findByRole("option", { name: "Workspace setting (Auto)" }));
  await user.click(screen.getByRole("button", { name: "Save changes" }));
  await waitFor(() =>
    expect(writes).toContainEqual({
      method: "PATCH",
      endpoint: "/assistant/nyxagent/agents/agent-researcher",
      body: { access_mode_override: null },
    }),
  );
});

it("hides the agent model selector without the internal model-controls flag", async () => {
  const owner = useAuthStore.getState().user!;
  useAuthStore.getState().setUser({ ...owner, capabilities: { billing_available: false, enabled_features: [] } });
  renderSheet();
  const profile = await screen.findByRole("form", { name: "Profile" });
  expect(within(profile).queryByRole("combobox", { name: "Agent model" })).not.toBeInTheDocument();
});

it("saves the optional channel progress reply without changing grants or prompt", async () => {
  const { user } = renderSheet();
  const sheet = await screen.findByRole("dialog", { name: "Agent details" });
  const profile = await within(sheet).findByRole("form", { name: "Profile" });
  const progress = within(profile).getByRole("switch", { name: "Channel progress reply" });
  expect(progress).not.toBeChecked();
  await user.click(progress);
  await user.click(screen.getByRole("button", { name: "Save changes" }));
  await waitFor(() => expect(writes).toContainEqual({
    method: "PATCH",
    endpoint: "/assistant/nyxagent/agents/agent-researcher",
    body: { channel_checking_notice: true },
  }));
  await waitFor(() =>
    expect(
      screen.queryByRole("button", { name: "Save changes" }),
    ).not.toBeInTheDocument(),
  );
});

it("fetches visitor readiness only in the open owner Grants settings", async () => {
  renderSheet({ open: false });
  expect(readinessReads).toBe(0);
  cleanup();
  renderSheet({ tab: "grants" });
  expect(await screen.findByText("Send one message to this assistant yourself.")).toBeVisible();
  expect(readinessReads).toBe(1);
});

it("preserves independent profile and grants drafts across tabs and a profile save", async () => {
  const { user } = renderSheet();
  const profile = await screen.findByRole("form", { name: "Profile" });
  const role = within(profile).getByRole("textbox", { name: "Role" });
  await user.clear(role);
  await user.type(role, "Unsaved profile");
  await user.click(screen.getByRole("tab", { name: "Access" }));
  const grants = screen.getByRole("form", { name: "Grants" });
  await user.click(within(grants).getByRole("checkbox", { name: /Slack/ }));
  await user.click(screen.getByRole("tab", { name: "Overview" }));
  expect(role).toHaveValue("Unsaved profile");
  await user.click(screen.getByRole("button", { name: "Save changes" }));
  await waitFor(() =>
    expect(
      screen.queryByRole("button", { name: "Save changes" }),
    ).not.toBeInTheDocument(),
  );
  await user.click(screen.getByRole("tab", { name: "Access" }));
  expect(within(grants).getByRole("checkbox", { name: /Slack/ })).toBeChecked();
  expect(screen.getByRole("button", { name: "Save changes" })).toBeEnabled();
  expect(writes).toHaveLength(1);
  expect(writes[0]?.method).toBe("PATCH");
  expect(document.querySelector("[data-grouped-list] [data-grouped-list]")).toBeNull();
});

it("refreshes pristine fields without replacing a dirty profile draft", async () => {
  const { user } = renderSheet();
  const profile = await screen.findByRole("form", { name: "Profile" });
  const role = within(profile).getByRole("textbox", { name: "Role" });
  await user.clear(role);
  await user.type(role, "My draft");
  agent = { ...agent, description: "Remote role", display_name: "Remote name" };
  await client.invalidateQueries();
  await waitFor(() => expect(within(profile).getByRole("textbox", { name: "Display name" })).toHaveValue("Remote name"));
  expect(role).toHaveValue("My draft");
  expect(screen.getByRole("button", { name: "Save changes" })).toBeEnabled();
  await user.click(screen.getByRole("button", { name: "Discard" }));
  expect(role).toHaveValue("Remote role");
});

it("keeps arrow navigation on the tablist after switching sections", async () => {
  const { user } = renderSheet();
  const first = await screen.findByRole("tab", { name: "Overview" });
  first.focus();
  await user.keyboard("{ArrowRight}{ArrowRight}");
  expect(screen.getByRole("tab", { name: "Skills" })).toHaveFocus();
  expect(screen.getByRole("tab", { name: "Skills" })).toHaveAttribute("aria-selected", "true");
});

it("falls back to Overview for an unavailable Channels deep link", async () => {
  agent = agentRow({ owner_kind: "org", owner_id: "team", can_maintain: false });
  renderSheet({ tab: "channels" });
  const overview = await screen.findByRole("heading", { name: "Overview", level: 2 });
  await waitFor(() => expect(overview).toHaveFocus());
  expect(screen.queryByRole("tab", { name: "Channels" })).not.toBeInTheDocument();
  expect(screen.queryByRole("tab", { name: "Memory" })).not.toBeInTheDocument();
});

it("offers personal add-note and prevents every memory write after destruction", async () => {
  const { user } = renderSheet({ tab: "memory" });
  const note = await screen.findByRole("textbox", { name: "Note" });
  await user.type(note, "A personal convention");
  await user.click(screen.getByRole("button", { name: "Save changes" }));
  await waitFor(() => expect(writes).toContainEqual({ method: "POST", endpoint: "/assistant/nyxagent/agents/agent-researcher/memory", body: { text: "A personal convention" } }));
  agent = { ...agent, status: "destroyed", destroyed_at: at };
  await client.invalidateQueries();
  await waitFor(() => expect(screen.queryByRole("button", { name: "Save note" })).not.toBeInTheDocument());
  for (const button of screen.getAllByRole("button", { name: /^Forget:/ })) expect(button).toBeDisabled();
  expect(screen.queryByRole("tab", { name: "Channels" })).not.toBeInTheDocument();
});

it.each(["user", "manager"] as const)("maps a shared %s's profile, consent and notes into the same tabs", async (share_role) => {
  agent = agentRow({ share_role, can_maintain: false, owner_name: "Alex" });
  const original = globalThis.__nyxidAssistantHttpMock!;
  globalThis.__nyxidAssistantHttpMock = (request) => {
    if (/\/(private-notes|shared-memory)$/.test(request.endpoint)) return new Response(JSON.stringify({ revision: 1, notes: [] }));
    if (request.endpoint.endsWith("/consents")) return new Response(JSON.stringify({ consents: [] }));
    return original(request);
  };
  const { user } = renderSheet();
  await screen.findByRole("tab", { name: "Overview" });
  expect(screen.queryByRole("note", { name: "Read-only" })).not.toBeInTheDocument();
  expect(screen.queryByRole("tab", { name: "Channels" })).not.toBeInTheDocument();
  expect(screen.queryByRole("textbox", { name: "Persona" }) !== null).toBe(share_role === "manager");
  await user.click(screen.getByRole("tab", { name: "Access" }));
  expect(screen.queryByRole("form", { name: "Grants" })).not.toBeInTheDocument();
  await user.click(screen.getByRole("tab", { name: "Memory" }));
  expect(await screen.findByRole("heading", { name: "Your private notes" })).toBeVisible();
  expect(screen.getByRole("heading", { name: "Published team memory" })).toBeVisible();
});

function machineAccessFixture() {
  const user = useAuthStore.getState().user!;
  useAuthStore.getState().setUser({
    ...user,
    capabilities: {
      billing_available: false,
      enabled_features: [
        "assistant:model-controls",
        "assistant:machine-capabilities",
      ],
    },
  });
  let machine = {
    node_id: "node",
    name: "Work Mac",
    revision: 3,
    protocol_v2: true,
    can_edit: true,
    mode: "shared_legacy",
    legacy: false,
    saved_login_ids: null,
    capabilities: {
      shell: false,
      files: false,
      browser: false,
      computer: false,
      developer_browser: false,
    },
    ceiling: {
      shell: true,
      files: true,
      browser: true,
      computer: true,
      developer_browser: true,
    },
    separated: { available: true, landlock_abi: 6, reason: null },
  };
  const original = globalThis.__nyxidAssistantHttpMock!;
  const control = { fail: "" };
  globalThis.__nyxidAssistantHttpMock = (request) => {
    const { endpoint, init } = request;
    if (endpoint.endsWith("/machines"))
      return new Response(JSON.stringify([machine]));
    const machineWrite =
      endpoint.endsWith("/machines/node") && init.method === "PUT";
    const grantsWrite = endpoint.endsWith("/grants") && init.method === "PUT";
    if (machineWrite || (grantsWrite && control.fail === "grants")) {
      const body = JSON.parse(String(init.body));
      writes.push({ endpoint, method: init.method!, body });
      if (control.fail === (machineWrite ? "machine access" : "grants")) {
        return new Response(
          JSON.stringify({ message: "Access changed; review and retry." }),
          { status: 409 },
        );
      }
      machine = {
        ...machine,
        revision: machine.revision + 1,
        capabilities: body.capabilities,
      };
      return new Response(JSON.stringify(machine));
    }
    return original(request);
  };
  return control;
}

async function editGrantsAndMachine() {
  const { user } = renderSheet({ tab: "grants" });
  const grants = await screen.findByRole("form", { name: "Grants" });
  await user.click(within(grants).getByRole("checkbox", { name: /Slack/ }));
  await user.click(await screen.findByRole("button", { name: /Work Mac/ }));
  await user.click(screen.getByRole("checkbox", { name: "Shell commands" }));
  await user.click(screen.getByRole("button", { name: "Back to Access" }));
  const bar = screen.getByRole("region", { name: "Unsaved changes" });
  expect(
    screen.getAllByRole("region", { name: "Unsaved changes" }),
  ).toHaveLength(1);
  expect(bar).toHaveTextContent("Unsaved: grants, machine access");
  return user;
}

it("saves dirty grants and machine access in order with independent payloads and fences", async () => {
  machineAccessFixture();
  const user = await editGrantsAndMachine();
  await user.click(screen.getByRole("button", { name: "Save changes" }));
  await waitFor(() =>
    expect(
      screen.queryByRole("region", { name: "Unsaved changes" }),
    ).not.toBeInTheDocument(),
  );
  expect(writes).toEqual([
    {
      method: "PUT",
      endpoint: "/assistant/nyxagent/agents/agent-researcher/grants",
      body: { services: ["github", "slack"], account_read: false },
    },
    {
      method: "PUT",
      endpoint: "/assistant/nyxagent/agents/agent-researcher/machines/node",
      body: {
        expected_revision: 3,
        capabilities: {
          shell: true,
          files: false,
          browser: false,
          computer: false,
          developer_browser: false,
        },
        saved_login_ids: null,
      },
    },
  ]);
});

it.each(["grants", "machine access"])(
  "keeps failed %s dirty while the independent form succeeds",
  async (failed) => {
    const control = machineAccessFixture();
    control.fail = failed;
    const user = await editGrantsAndMachine();
    await user.click(screen.getByRole("button", { name: "Save changes" }));
    await waitFor(() => expect(writes).toHaveLength(2));
    await waitFor(() =>
      expect(
        screen.getByRole("region", { name: "Unsaved changes" }),
      ).toHaveTextContent(`Unsaved: ${failed}`),
    );
    if (failed === "machine access")
      await user.click(screen.getByRole("button", { name: /Work Mac/ }));
    const failedForm = screen.getByRole("form", {
      name: failed === "grants" ? "Grants" : "Work Mac capabilities",
    });
    expect(
      within(failedForm)
        .getByText(
          failed === "grants"
            ? "Access changed; review and retry."
            : "Could not save capabilities. Reload if access changed.",
        )
        .closest("[data-grouped-footer]"),
    ).toBeInTheDocument();
    control.fail = "";
    await user.click(screen.getByRole("button", { name: "Save changes" }));
    await waitFor(() =>
      expect(
        screen.queryByRole("region", { name: "Unsaved changes" }),
      ).not.toBeInTheDocument(),
    );
    expect(writes).toHaveLength(3);
    expect(writes[2]!.endpoint).toMatch(
      failed === "grants" ? /\/grants$/ : /\/machines\/node$/,
    );
  },
);

it("discards every dirty Access form including retained machine drafts", async () => {
  machineAccessFixture();
  const user = await editGrantsAndMachine();
  await user.click(screen.getByRole("button", { name: "Discard" }));
  await waitFor(() =>
    expect(
      screen.queryByRole("region", { name: "Unsaved changes" }),
    ).not.toBeInTheDocument(),
  );
  expect(screen.getByRole("checkbox", { name: /Slack/ })).not.toBeChecked();
  await user.click(screen.getByRole("button", { name: /Work Mac/ }));
  expect(
    screen.getByRole("checkbox", { name: "Shell commands" }),
  ).not.toBeChecked();
  expect(writes).toEqual([]);
});
