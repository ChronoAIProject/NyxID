import { confirmChanges } from "@/test-helpers/confirm-changes";
import type { ComponentProps } from "react";
import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useAuthStore } from "@/stores/auth-store";
import { AgentDetailsSheet } from "./nyxbot-agent-details";

vi.mock("@tanstack/react-router", () => ({
  useRouter: () => undefined,
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
  useChannelBots: () => ({ isPending: false, error: null, refetch: vi.fn(), data: availableBots }),
}));

let availableBots: Record<string, unknown>[];
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
  availableBots = [];
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
  await confirmChanges();
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
  await confirmChanges();
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
  await confirmChanges();
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
  await confirmChanges();
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
  await confirmChanges();
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
  await confirmChanges();
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
  await confirmChanges();
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
  await confirmChanges();
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
  await confirmChanges();
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
  await confirmChanges();
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

it("preserves drafts across tabs and saves all dirty sections from any tab", async () => {
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
  await confirmChanges();
  await waitFor(() =>
    expect(
      screen.queryByRole("button", { name: "Save changes" }),
    ).not.toBeInTheDocument(),
  );
  await user.click(screen.getByRole("tab", { name: "Access" }));
  expect(within(grants).getByRole("checkbox", { name: /Slack/ })).toBeChecked();
  expect(screen.queryByRole("button", { name: "Save changes" })).toBeNull();
  expect(writes.map((write) => write.method)).toEqual(["PATCH", "PUT"]);
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
  await confirmChanges();
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
  await confirmChanges();
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
  await confirmChanges();
    await waitFor(() => expect(writes).toHaveLength(2));
    await waitFor(() =>
      expect(
        screen.getByRole("region", { name: "Unsaved changes" }),
      ).toHaveTextContent(`Unsaved: ${failed}`),
    );
    if (failed === "grants")
      await user.click(screen.getByRole("button", { name: "Back to Access" }));
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
  await confirmChanges();
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
  await user.click(screen.getByRole("button", { name: "Back to Access" }));
  expect(screen.getByRole("checkbox", { name: /Slack/ })).not.toBeChecked();
  await user.click(screen.getByRole("button", { name: /Work Mac/ }));
  expect(
    screen.getByRole("checkbox", { name: "Shell commands" }),
  ).not.toBeChecked();
  expect(writes).toEqual([]);
});

it.each(["X", "Escape"])("guards dirty sheet dismissal via %s and preserves edits on Keep editing", async (method) => {
  const onOpenChange = vi.fn();
  const { user } = renderSheet({ onOpenChange });
  const description = await screen.findByRole("textbox", { name: "Role" });
  await user.type(description, " unsaved");
  if (method === "X") await user.click(screen.getByRole("button", { name: "Close agent details" }));
  else await user.keyboard("{Escape}");
  expect(onOpenChange).not.toHaveBeenCalled();
  const prompt = screen.getByRole("dialog", { name: "Discard unsaved changes?" });
  await user.click(within(prompt).getByRole("button", { name: "Keep editing" }));
  expect(description).toHaveValue("Finds urgent issues unsaved");
  expect(onOpenChange).not.toHaveBeenCalled();
  await user.keyboard("{Escape}");
  await user.click(within(screen.getByRole("dialog", { name: "Discard unsaved changes?" })).getByRole("button", { name: "Discard" }));
  expect(onOpenChange).toHaveBeenCalledWith(false);
  expect(writes).toEqual([]);
});

it("closes a pristine sheet immediately", async () => {
  const onOpenChange = vi.fn();
  const { user } = renderSheet({ onOpenChange });
  await screen.findByRole("textbox", { name: "Role" });
  await user.click(screen.getByRole("button", { name: "Close agent details" }));
  expect(onOpenChange).toHaveBeenCalledWith(false);
  expect(screen.queryByRole("dialog", { name: "Discard unsaved changes?" })).toBeNull();
});

async function openSearch(user: ReturnType<typeof userEvent.setup>) {
  const control = await waitFor(() => screen.queryByRole("searchbox") ?? screen.getByRole("button", { name: /^Search .+ agent details$/ }));
  if (control.getAttribute("role") !== "searchbox") await user.click(control);
  return screen.findByRole("searchbox");
}

it("searches across mounted tabs, reveals a memory row and moves focus to it", async () => {
  const { user } = renderSheet();
  const search = await openSearch(user);
  await openSearch(user);
  await user.click(search);
  await user.paste("weekly monday");
  const results = screen.getByRole("list", { name: "Memory search results" });
  expect(screen.getByRole("status")).toHaveTextContent("1 result for weekly monday");
  expect(within(results).getAllByRole("mark")).toHaveLength(2);
  expect(screen.queryByRole("form", { name: "Profile" })).not.toBeInTheDocument();
  await user.click(within(results).getByRole("button", { name: /Weekly digests on Mondays Memory/ }));
  expect(search).toHaveValue("");
  expect(screen.getByRole("tab", { name: "Memory" })).toHaveAttribute("aria-selected", "true");
  await waitFor(() => expect(document.querySelector('[data-drawer-search-target="memory-n1"]')).toHaveFocus());
  expect(document.activeElement).toHaveAttribute("data-search-highlight");
});

it("preserves profile and grants drafts and a single coordinated save bar while searching", async () => {
  const { user } = renderSheet();
  const role = await screen.findByRole("textbox", { name: "Role" });
  await user.clear(role);
  await user.type(role, "Fresh unsaved role");
  await user.click(screen.getByRole("tab", { name: "Access" }));
  await user.click(screen.getByRole("checkbox", { name: /Slack/ }));
  const search = await openSearch(user);
  await openSearch(user);
  await user.click(search);
  await user.paste("fresh unsaved");
  const bars = screen.getAllByRole("region", { name: "Unsaved changes" });
  expect(bars).toHaveLength(1);
  expect(bars[0]).toHaveTextContent("Unsaved: profile, grants");
  await user.click(screen.getByRole("button", { name: "Role Overview › Profile" }));
  expect(role).toHaveValue("Fresh unsaved role");
  await user.click(screen.getByRole("tab", { name: "Access" }));
  expect(screen.getByRole("checkbox", { name: /Slack/ })).toBeChecked();
  await openSearch(user);
  await user.click(search);
  await user.paste("slack");
  await user.click(screen.getByRole("button", { name: "Save changes" }));
  await confirmChanges();
  await waitFor(() => expect(screen.queryByRole("region", { name: "Unsaved changes" })).not.toBeInTheDocument());
  expect(writes.map((write) => write.method)).toEqual(["PATCH", "PUT"]);
  expect(search).toHaveValue("slack");
});

it("clears the query, collapses empty search, then follows the dirty dismissal guard", async () => {
  const close = vi.fn();
  const { user } = renderSheet({ onOpenChange: close });
  const role = await screen.findByRole("textbox", { name: "Role" });
  await user.type(role, " draft");
  const search = await openSearch(user);
  await openSearch(user);
  await user.click(search);
  await user.paste("role");
  await user.keyboard("{Escape}");
  expect(search).toHaveValue("");
  expect(search).toHaveFocus();
  expect(close).not.toHaveBeenCalled();
  expect(screen.queryByRole("dialog", { name: "Discard unsaved changes?" })).not.toBeInTheDocument();
  await user.keyboard("{Escape}");
  expect(screen.queryByRole("searchbox")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Search researcher agent details" })).toHaveAttribute("aria-expanded", "false");
  expect(close).not.toHaveBeenCalled();
  await user.keyboard("{Escape}");
  expect(screen.getByRole("dialog", { name: "Discard unsaved changes?" })).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Keep editing" }));
  expect(role).toHaveValue("Finds urgent issues draft");
});

it("supports drawer shortcuts, result arrows and Enter without stealing editable shortcuts", async () => {
  const { user } = renderSheet();
  const tab = await screen.findByRole("tab", { name: "Overview" });
  expect(screen.queryByRole("searchbox")).toBeNull();
  tab.focus();
  await user.keyboard("/");
  const search = await screen.findByRole("searchbox");
  expect(search).toHaveFocus();
  await openSearch(user);
  await user.click(search);
  await user.paste("account");
  await user.keyboard("{ArrowDown}");
  const result = document.activeElement!;
  expect(result).toHaveRole("button");
  await user.keyboard("{ArrowUp}{ArrowDown}{Enter}");
  await waitFor(() => expect(search).toHaveValue(""));
  expect(screen.getByRole("tab", { name: "Access" })).toHaveAttribute("aria-selected", "true");
  const selected = screen.getByRole("tab", { name: "Access" });
  selected.focus();
  await user.keyboard("{Control>}f{/Control}");
  expect(search).toHaveFocus();
  await user.click(screen.getByRole("tab", { name: "Overview" }));
  const role = screen.getByRole("textbox", { name: "Role" });
  await user.click(role);
  await user.keyboard("/");
  expect(role).toHaveFocus();
  expect(role).toHaveValue("Finds urgent issues/");
});

it("reveals a machine drill-in and guards leaving its dirty draft through search", async () => {
  machineAccessFixture();
  const { user } = renderSheet();
  const search = await openSearch(user);
  await openSearch(user);
  await user.click(search);
  await user.paste("work mac");
  await user.click(await screen.findByRole("button", { name: "Work Mac Access › Machine access" }));
  const target = document.querySelector('[data-drawer-search-target="machine-detail-node"]');
  await waitFor(() => expect(target).toHaveFocus());
  const shell = screen.getByRole("checkbox", { name: "Shell commands" });
  await user.click(shell);
  await user.type(search, "persona");
  expect(search).toHaveValue("persona");
  expect(screen.getAllByRole("region", { name: "Unsaved changes" })).toHaveLength(1);
  await user.click(screen.getByRole("button", { name: "Persona Overview › Profile" }));
  expect(screen.getByRole("dialog", { name: "Discard unsaved changes?" })).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Keep editing" }));
  expect(search).toHaveValue("persona");
  await user.click(screen.getByRole("button", { name: "Clear agent search" }));
  expect(shell).toBeChecked();
  await user.click(screen.getByRole("button", { name: "Back to Access" }));
  await user.click(screen.getByRole("button", { name: "Discard" }));
  expect(screen.getByRole("button", { name: /Work Mac/ })).toBeVisible();
});

it("searches only entries available to an organization viewer and preserves read-only controls", async () => {
  agent = agentRow({ owner_kind: "org", owner_id: "team", org_role: "viewer", can_maintain: false });
  const { user } = renderSheet();
  const search = await openSearch(user);
  await user.type(search, "persona");
  await user.click(screen.getByRole("button", { name: "Persona Overview › Profile" }));
  expect(screen.getByRole("textbox", { name: "Persona" })).toBeDisabled();
  for (const query of ["connect another bot", "weekly digests", "destroy", "share agent", "find skills", "read my account"]) {
    await openSearch(user);
  await user.clear(search);
    await user.type(search, query);
    expect(screen.getByText(`No results for “${query}”.`)).toBeInTheDocument();
  }
  expect(writes).toHaveLength(0);
});

it("keeps destroyed-agent search available with deletion and remembered notes but no channel actions", async () => {
  agent = agentRow({ status: "destroyed", destroyed_at: at });
  const { user } = renderSheet();
  const search = await openSearch(user);
  await openSearch(user);
  await user.click(search);
  await user.paste("delete");
  expect(screen.getByRole("button", { name: "Delete permanently Overview" })).toBeInTheDocument();
  await openSearch(user);
  await user.clear(search);
  await openSearch(user);
  await user.click(search);
  await user.paste("weekly");
  expect(screen.getByRole("button", { name: /Weekly digests on Mondays Memory/ })).toBeInTheDocument();
  await openSearch(user);
  await user.clear(search);
  await openSearch(user);
  await user.click(search);
  await user.paste("connect another bot");
  expect(screen.getByText("No results for “connect another bot”.")).toBeInTheDocument();
});

it("opens the exact service operation scope from search without changing its revision fence", async () => {
  const owner = useAuthStore.getState().user!;
  useAuthStore.getState().setUser({ ...owner, capabilities: { enabled_features: ["assistant:operation-scopes"] } });
  const original = globalThis.__nyxidAssistantHttpMock!;
  globalThis.__nyxidAssistantHttpMock = (request) => request.endpoint.endsWith("/operations")
    ? new Response(JSON.stringify([{ service_id: "notion-id", service_slug: "api-notion", service_name: "Notion", revision: 7,
      all_operations: true, allows_explicit_rules: true, endpoint_ids: [], rules: [], operations: [] }])) : original(request);
  const { user } = renderSheet();
  const search = await openSearch(user);
  await openSearch(user);
  await user.click(search);
  await user.paste("notion operations");
  await user.click(await screen.findByRole("button", { name: "Notion · Operations Access › Service operations" }));
  const view = document.querySelector('[data-operation-view="notion-id"]')!;
  await waitFor(() => expect(document.querySelector('[data-drawer-search-target="operations-notion-id"]')).toHaveFocus());
  expect(view).toHaveTextContent("Revision 7");
  expect(screen.getByRole("tab", { name: "Access", hidden: true })).toHaveAttribute("aria-selected", "true");
  expect(search).toBeVisible();
  await user.click(within(view as HTMLElement).getByRole("button", { name: "Back to Access" }));
  expect(screen.getByRole("button", { name: /Notion All operations/ })).toHaveFocus();
  expect(writes).toHaveLength(0);
});

it("finds attached skills by description and version and opens retained history from search", async () => {
  const original = globalThis.__nyxidAssistantHttpMock!;
  globalThis.__nyxidAssistantHttpMock = (request) => {
    if (request.endpoint.endsWith("/skills")) return new Response(JSON.stringify({ revision: 6,
      skills: [{ source: "ornn", skill_id: "faq", name: "kek-faq-reference", version: "1.2.3", sha256: "abc" }],
      metadata: { faq: { description: "Café booking reference", size_bytes: 80 } },
    }));
    if (request.endpoint.endsWith("/skills/history")) return new Response(JSON.stringify({ items: [
      { id: "activity", event_type: "assistant_skills_updated", revision: 6, created_at: at, skills: [{ skill_id: "faq", name: "kek-faq-reference", version: "1.2.3" }] },
    ] }));
    return original(request);
  };
  const { user } = renderSheet();
  const search = await openSearch(user);
  await openSearch(user);
  await user.click(search);
  await user.paste("cafe 1.2.3");
  await user.click(await screen.findByRole("button", { name: "kek-faq-reference Skills › kek-faq-reference" }));
  await waitFor(() => expect(document.querySelector('[data-drawer-search-target="skill-faq"]')).toHaveFocus());
  expect(screen.getByRole("heading", { name: "kek-faq-reference" })).toBeVisible();
  await openSearch(user);
  await user.click(search);
  await user.paste("history");
  await user.click(screen.getByRole("button", { name: "History Skills › History" }));
  await waitFor(() => expect(document.querySelector('[data-drawer-search-target="skill-history"]')).toHaveFocus());
  expect(screen.getByRole("list", { name: "Skill history" })).toHaveTextContent("Attached skills changed");
  await user.click(screen.getByRole("button", { name: "Back to Skills" }));
  expect(screen.getByRole("button", { name: "History" })).toHaveFocus();
  expect(screen.getByRole("button", { name: /kek-faq-reference.*1.2.3/ })).toBeVisible();
});

it("indexes connected bot handles, unopened chats and available bots, revealing the exact channel page", async () => {
  availableBots = [{ id: "new-bot", label: "Help desk", platform: "slack" }];
  const original = globalThis.__nyxidAssistantHttpMock!;
  globalThis.__nyxidAssistantHttpMock = (request) => {
    if (request.endpoint === "/assistant/nyxagent/channels") return new Response(JSON.stringify({ channel_agents: [{
      id: "connected", channel_bot_id: "bot", platform: "telegram", bot_label: "Helper", bot_username: "helper_bot",
      transport: "webhook", status: "active", owner_linked: true, agent_id: agent.id, created_at: at,
    }] }));
    if (request.endpoint.endsWith("/channels/connected/chats")) return new Response(JSON.stringify({ chats: [{
      id: "room", channel_agent_id: "connected", platform: "telegram", bot_label: "Helper", kind: "group", title: "Café research room",
      reply_mode: "mention", members: "owner", owner_seen: true, allow_posts: false, last_message_at: at,
    }] }));
    return original(request);
  };
  const { user } = renderSheet();
  const search = await openSearch(user);
  await openSearch(user);
  await user.click(search);
  await user.paste("helper_bot");
  expect(await screen.findByRole("button", { name: "Helper Channels › Connected bot" })).toBeInTheDocument();
  await openSearch(user);
  await user.clear(search);
  await openSearch(user);
  await user.click(search);
  await user.paste("cafe room");
  await user.click(await screen.findByRole("button", { name: "Café research room Channels › Helper › Chats" }));
  await waitFor(() => expect(document.querySelector('[data-drawer-search-target="channel-chat-room"]')).toHaveFocus());
  expect(screen.getByRole("heading", { name: /Café research room · Group/ })).toBeVisible();
  await openSearch(user);
  await user.click(search);
  await user.paste("help desk");
  await user.click(screen.getByRole("button", { name: "Help desk Channels › Available bots" }));
  await waitFor(() => expect(document.querySelector('[data-drawer-search-target="available-bot-new-bot"]')).toHaveFocus());
  expect(screen.getByRole("button", { name: /Connect Help desk to researcher/ })).toBeVisible();
  await user.click(screen.getByRole("button", { name: "Back to Channels" }));
  expect(screen.getByRole("button", { name: "Chats and who can talk" })).toBeVisible();
  expect(writes).toHaveLength(0);
});

it.each(["user", "manager"] as const)("keeps shared %s search scoped to its rendered profile and memory", async (share_role) => {
  agent = agentRow({ share_role, can_maintain: false, owner_name: "Alex", persona: "Friendly guide" });
  const original = globalThis.__nyxidAssistantHttpMock!;
  globalThis.__nyxidAssistantHttpMock = (request) => {
    if (/\/(private-notes|shared-memory)$/.test(request.endpoint)) return new Response(JSON.stringify({ revision: 2, notes: [{ id: "shared-note", text: "Café team convention" }] }));
    if (request.endpoint.endsWith("/consents")) return new Response(JSON.stringify({ consents: [] }));
    return original(request);
  };
  const { user } = renderSheet();
  const search = await openSearch(user);
  await openSearch(user);
  await user.click(search);
  await user.paste("friendly");
  expect(screen.getByRole("button", { name: "Persona Overview" })).toBeInTheDocument();
  await openSearch(user);
  await user.clear(search);
  await openSearch(user);
  await user.click(search);
  await user.paste("cafe convention");
  expect((await screen.findAllByRole("button", { name: /Memory/ })).length).toBeGreaterThan(0);
  await openSearch(user);
  await user.clear(search);
  await openSearch(user);
  await user.click(search);
  await user.paste("connect another bot");
  expect(screen.getByText("No results for “connect another bot”.")).toBeInTheDocument();
  expect(writes).toHaveLength(0);
});

it("indexes NyxBot's full-access row without specialist-only profile or lifecycle entries", async () => {
  agent = agentRow({ kind: "nyxbot", name: "nyxbot" });
  const { user } = renderSheet();
  const search = await openSearch(user);
  await openSearch(user);
  await user.click(search);
  await user.paste("connected services account");
  await user.click(screen.getByRole("button", { name: "Connected services and account Access" }));
  await waitFor(() => expect(document.querySelector('[data-drawer-search-target="nyxbot-full-access"]')).toHaveFocus());
  expect(screen.queryByRole("form", { name: "Grants" })).not.toBeInTheDocument();
  await openSearch(user);
  await user.click(search);
  await user.paste("destroy");
  expect(screen.getByText("No results for “destroy”.")).toBeInTheDocument();
});

it("honors an incoming tab deep link while searching and preserves the mounted profile draft", async () => {
  const props = { agentId: String(agent.id), agents: [], open: true, onOpenChange: vi.fn(), onDeleted: vi.fn() };
  const { rerender } = render(<QueryClientProvider client={client}><AgentDetailsSheet {...props} /></QueryClientProvider>);
  const user = userEvent.setup();
  const role = await screen.findByRole("textbox", { name: "Role" });
  await user.type(role, " draft");
  const search = await openSearch(user);
  await openSearch(user);
  await user.click(search);
  await user.paste("missing");
  rerender(<QueryClientProvider client={client}><AgentDetailsSheet {...props} tab="skills" /></QueryClientProvider>);
  await waitFor(() => expect(search).toHaveValue(""));
  await waitFor(() => expect(screen.getByRole("heading", { name: "Skills", level: 2 })).toHaveFocus());
  await user.click(screen.getByRole("tab", { name: "Overview" }));
  expect(role).toHaveValue("Finds urgent issues draft");
});

it("pins the same save bar outside scrolling content through tabs, drill-ins and search", async () => {
  machineAccessFixture();
  const { user } = renderSheet();
  await user.type(await screen.findByRole("textbox", { name: "Role" }), " draft");
  const bar = screen.getByRole("region", { name: "Unsaved changes" });
  expect(bar).toHaveAttribute("data-viewport-save-bar");
  expect(bar.closest("[data-form-viewport], [data-compact-scroll], [data-panel]")).toBeNull();
  await user.click(screen.getByRole("tab", { name: "Access" }));
  expect(bar).toBeVisible();
  await user.click(await screen.findByRole("button", { name: /Work Mac/ }));
  expect(bar).toBeVisible();
  expect(bar.closest("[inert], [aria-hidden=true]")).toBeNull();
  await user.type(await openSearch(user), "persona");
  expect(bar).toBeVisible();
  expect(screen.getAllByRole("button", { name: "Save changes" })).toHaveLength(1);
  expect(screen.getByRole("button", { name: "Discard" })).toBeVisible();
});

it("adds named help to every rendered drawer section and keeps help text searchable", async () => {
  const { user } = renderSheet();
  await screen.findByRole("form", { name: "Profile" });
  for (const name of ["Overview", "Access", "Skills", "Memory", "Channels"]) {
    const tab = screen.queryByRole("tab", { name });
    if (!tab) continue;
    await user.click(tab);
    const panel = screen.getByRole("tabpanel", { name });
    for (const heading of panel.querySelectorAll("h3")) {
      const title = heading.getAttribute("aria-label")!;
      expect(within(heading).getByRole("button", { name: `About ${title}` })).toBeInTheDocument();
    }
  }
  await user.click(screen.getByRole("tab", { name: "Access" }));
  await user.click(screen.getByRole("button", { name: "About Services" }));
  expect(await screen.findByRole("tooltip", { name: "About Services" })).toHaveTextContent("which connected services this specialist may call");
  await user.keyboard("{Escape}");
  await user.type(await openSearch(user), "connected services specialist");
  expect(screen.getByRole("list", { name: "Access search results" })).toHaveTextContent("Services");
});

it("reviews profile values before saving and keeps Enter on the same review path", async () => {
  const { user } = renderSheet();
  const role = await screen.findByRole("textbox", { name: "Role" });
  await user.clear(role);
  await user.type(role, "Review this role");
  await user.click(screen.getByRole("textbox", { name: "Name" }));
  await user.keyboard("{Enter}");
  const review = await screen.findByRole("dialog", { name: "Review changes" });
  expect(writes).toEqual([]);
  expect(review).toHaveTextContent("Role / description");
  expect(review).toHaveTextContent("Finds urgent issues → Review this role");
  await user.click(within(review).getByRole("button", { name: "Keep editing" }));
  expect(role).toHaveValue("Review this role");
  expect(writes).toEqual([]);
});

it("toggles the circular search control, collapses empty blur, and preserves nonempty queries", async () => {
  const { user } = renderSheet();
  const toggle = await screen.findByRole("button", { name: "Search researcher agent details" });
  expect(toggle).toHaveAttribute("aria-expanded", "false");
  expect(screen.queryByRole("searchbox")).toBeNull();
  await user.click(toggle);
  const input = screen.getByRole("searchbox", { name: "Search researcher agent details" });
  expect(input).toHaveFocus();
  await user.click(screen.getByRole("tab", { name: "Skills" }));
  expect(screen.queryByRole("searchbox")).toBeNull();
  await openSearch(user);
  await user.type(input, "persona");
  await user.click(screen.getByRole("tab", { name: "Overview" }));
  expect(input).toBeVisible();
  expect(input).toHaveValue("persona");
});
