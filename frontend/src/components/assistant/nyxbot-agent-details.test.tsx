import { confirmChanges } from "@/test-helpers/confirm-changes";
import type { ComponentProps } from "react";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
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
let grantRequestBodies: string[];
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

function enableCrm() {
  const current = useAuthStore.getState().user!;
  useAuthStore.getState().setUser({
    ...current,
    capabilities: {
      ...current.capabilities,
      enabled_features: [
        ...(current.capabilities?.enabled_features ?? []),
        "cms:poc-v1",
      ],
    },
  });
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
  grantRequestBodies = [];
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
    if (endpoint === "/assistant/crm/objects") {
      return new Response(
        JSON.stringify({
          objects: [
            {
              id: "company-object",
              owner_id: "owner",
              key: "company",
              label: "Company",
              plural_label: "Companies",
              schema_revision: 1,
              fields: [],
              pipeline: null,
              default_columns: [],
              seeded: true,
              access: "work",
              created_at: at,
              updated_at: at,
            },
          ],
        }),
      );
    }
    if (endpoint === `/assistant/nyxagent/agents/${id}/grants`) {
      grantRequestBodies.push(String(init.body));
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
  vi.useRealTimers();
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
  return { onDeleted, user: userEvent.setup({ delay: null, pointerEventsCheck: 0 }) };
}

// Set up draft values in one render; keyboard/focus behavior uses user-event below.
function appendText(field: HTMLElement, text: string) {
  fireEvent.change(field, { target: { value: (field as HTMLInputElement).value + text } });
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
  const grants = await within(sheet).findByLabelText("Grants", { selector: "form" });
  expect(
    screen.queryByRole("region", { name: "Unsaved changes" }),
  ).not.toBeInTheDocument();
  expect(within(grants).getByRole("checkbox", { name: /GitHub/ })).toBeChecked();
  const guests = within(grants).getByRole("list", { name: "Guest access" });
  expect(within(guests).getByLabelText("github", { selector: '[role="combobox"]' })).toHaveTextContent(
    "Look things up only",
  );
  await user.click(within(grants).getByRole("checkbox", { name: /Slack/ }));
  // A newly granted service starts at the default level.
  expect(within(guests).getByLabelText("slack", { selector: '[role="combobox"]' })).toHaveTextContent(
    "Use, but not change or delete",
  );
  await user.click(within(grants).getByLabelText("Read my account", { selector: '[role="switch"]' }));
  const save = screen.getByText("Save changes", { selector: "button" });
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
  expect(grantRequestBodies).toEqual([
    '{"services":["github","slack"],"account_read":true}',
  ]);
});

it("lets the owner choose what others in the agent's chats may do with each service", async () => {
  const { user } = renderSheet({ tab: "grants" });
  const sheet = await screen.findByRole("dialog", { name: "Agent details" });
  const grants = await within(sheet).findByLabelText("Grants", { selector: "form" });
  expect(
    screen.queryByRole("region", { name: "Unsaved changes" }),
  ).not.toBeInTheDocument();
  await user.click(within(grants).getByLabelText("github", { selector: '[role="combobox"]' }));
  await user.click(screen.getByRole("option", { name: "Everything this agent can do" }));
  const save = screen.getByText("Save changes", { selector: "button" });
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

it("hides CRM grant controls and omits stored grants from a flag-off save", async () => {
  agent = agentRow({
    crm: [{ owner_id: "owner", object: "company", capabilities: ["read"] }],
  });
  const { user } = renderSheet({ tab: "grants" });
  const sheet = await screen.findByRole("dialog", { name: "Agent details" });
  const grants = await within(sheet).findByLabelText("Grants", { selector: "form" });
  expect(
    within(grants).queryByRole("group", { name: "CRM access" }),
  ).not.toBeInTheDocument();
  await user.click(within(grants).getByRole("checkbox", { name: /Slack/ }));
  await user.click(screen.getByText("Save changes", { selector: "button" }));
  await confirmChanges();
  await waitFor(() =>
    expect(grantRequestBodies).toEqual([
      '{"services":["github","slack"],"account_read":false}',
    ]),
  );
  expect(client.getQueryCache().findAll({ queryKey: ["crm"] })).toHaveLength(0);
});

it("preserves CRM grants when a flag-on save changes only other grants", async () => {
  enableCrm();
  agent = agentRow({
    crm: [
      {
        owner_id: "owner",
        object: "company",
        capabilities: ["read"],
      },
    ],
  });
  const { user } = renderSheet({ tab: "grants" });
  const sheet = await screen.findByRole("dialog", { name: "Agent details" });
  const grants = await within(sheet).findByLabelText("Grants", { selector: "form" });
  const crm = await within(grants).findByRole("group", { name: "CRM access" });
  expect(within(crm).getByRole("checkbox", { name: "Read" })).toBeChecked();

  await user.click(within(grants).getByRole("checkbox", { name: /Slack/ }));
  await user.click(screen.getByText("Save changes", { selector: "button" }));
  await confirmChanges();

  await waitFor(() =>
    expect(writes).toContainEqual({
      method: "PUT",
      endpoint: "/assistant/nyxagent/agents/agent-researcher/grants",
      body: { services: ["github", "slack"], account_read: false },
    }),
  );
  expect(grantRequestBodies).toEqual([
    '{"services":["github","slack"],"account_read":false}',
  ]);
});

it("sends CRM grants only after an explicit flag-on CRM edit", async () => {
  enableCrm();
  const { user } = renderSheet({ tab: "grants" });
  const sheet = await screen.findByRole("dialog", { name: "Agent details" });
  const grants = await within(sheet).findByLabelText("Grants", { selector: "form" });
  const crm = await within(grants).findByRole("group", { name: "CRM access" });

  await user.click(within(crm).getByRole("checkbox", { name: "Read" }));
  await user.click(screen.getByText("Save changes", { selector: "button" }));
  await confirmChanges();

  await waitFor(() =>
    expect(writes).toContainEqual({
      method: "PUT",
      endpoint: "/assistant/nyxagent/agents/agent-researcher/grants",
      body: {
        services: ["github"],
        crm: [
          {
            owner_id: "owner",
            object: "company",
            capabilities: ["read"],
          },
        ],
        account_read: false,
      },
    }),
  );
  expect(grantRequestBodies).toEqual([
    '{"services":["github"],"crm":[{"owner_id":"owner","object":"company","capabilities":["read"]}],"account_read":false}',
  ]);
});

it("edits the role and destroys only after confirmation", async () => {
  const { user } = renderSheet();
  const sheet = await screen.findByRole("dialog", { name: "Agent details" });
  const profile = await within(sheet).findByLabelText("Profile", { selector: "form" });
  const description = within(profile).getByLabelText("Role", { selector: "input,textarea" });
  await user.clear(description);
  appendText(description, "Tracks p0 issues");
  await user.click(screen.getByText("Save changes", { selector: "button" }));
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
  const profile = await within(sheet).findByLabelText("Profile", { selector: "form" });
  // The display name heads the sheet with the @handle beside it.
  expect(sheet).toHaveTextContent("Scout@researcher");
  const displayName = within(profile).getByLabelText("Display name", { selector: "input,textarea" });
  const persona = within(profile).getByLabelText("Persona", { selector: "input,textarea" });
  expect(displayName).toHaveValue("Scout");
  expect(persona).toHaveValue("Brisk and factual.");
  await user.clear(displayName);
  appendText(displayName, "Luna");
  await user.clear(persona);
  await user.click(screen.getByText("Save changes", { selector: "button" }));
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
  const profile = await within(sheet).findByLabelText("Profile", { selector: "form" });
  globalThis.__nyxidAssistantHttpMock = () =>
    new Response(JSON.stringify({ message: "A display name must not contain credentials" }), {
      status: 400,
    });
  appendText(within(profile).getByLabelText("Display name", { selector: "input,textarea" }), "sk-live");
  await user.click(screen.getByText("Save changes", { selector: "button" }));
  await confirmChanges();
  expect(await within(profile).findByRole("alert")).toHaveTextContent(
    "A display name must not contain credentials",
  );
});

it("lets NyxBot get a display name and persona, with full access and no grants or lifecycle actions", async () => {
  agent = agentRow({ id: "agent-nyxbot", kind: "nyxbot", name: "NyxBot", created_by: "user" });
  const { user } = renderSheet();
  const sheet = await screen.findByRole("dialog", { name: "Agent details" });
  const profile = await within(sheet).findByLabelText("Profile", { selector: "form" });
  expect(within(sheet).queryByLabelText("Name", { selector: "input,textarea" })).not.toBeInTheDocument();
  appendText(within(profile).getByLabelText("Display name", { selector: "input,textarea" }), "Nyx");
  appendText(within(profile).getByLabelText("Persona", { selector: "input,textarea" }), "Warm and concise.");
  await user.click(screen.getByText("Save changes", { selector: "button" }));
  await confirmChanges();
  await waitFor(() =>
    expect(writes).toContainEqual({
      method: "PATCH",
      endpoint: "/assistant/nyxagent/agents/agent-nyxbot",
      body: { display_name: "Nyx", persona: "Warm and concise." },
    }),
  );
  await user.click(within(sheet).getByText("Access", { selector: '[role="tab"]' }));
  expect(within(sheet).getByRole("region", { name: "Access" })).toHaveTextContent("full access");
  expect(within(sheet).queryByLabelText("Grants", { selector: "form" })).not.toBeInTheDocument();
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
  expect(within(sheet).getByLabelText("Role", { selector: "input,textarea" })).toBeDisabled();
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
  const form = await screen.findByLabelText("Shared memory", { selector: "form" });
  expect(screen.getByText(/Never save private member content/)).toBeVisible();
  const input = within(form).getByLabelText("Add shared note", { selector: "input,textarea" });
  expect(input).toHaveAttribute("maxlength", "500");
  expect(
    screen.queryByText("Save changes", { selector: "button" }),
  ).not.toBeInTheDocument();
  appendText(input, "Public team convention");
  await user.click(screen.getByText("Save changes", { selector: "button" }));
  await confirmChanges();
  await waitFor(() =>
    expect(writes).toContainEqual({
      method: "POST",
      endpoint: "/assistant/nyxagent/agents/agent-researcher/memory",
      body: { text: "Public team convention" },
    }),
  );
  expect(
    screen.queryByLabelText("Read my account", { selector: '[role="switch"]' }),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByRole("list", { name: "Guest access" }),
  ).not.toBeInTheDocument();
});

it.each(["admin", "member"])(
  "limits org guest policy controls to flagged admins (%s)",
  async (role) => {
    const current = useAuthStore.getState().user!;
    useAuthStore
      .getState()
      .setUser({
        ...current,
        capabilities: {
          ...current.capabilities,
          billing_available: false,
          enabled_features: ["nyxbot:org-specialist-channels"],
        },
      });
    agent = agentRow({
      owner_kind: "org",
      owner_id: "org-team",
      org_role: role,
      can_maintain: true,
      can_use: true,
    });
    renderSheet({ tab: "grants" });
    await screen.findByRole("form", { name: "Grants" });
    if (role === "admin")
      expect(screen.getByRole("list", { name: "Guest access" })).toBeVisible();
    else
      expect(
        screen.queryByRole("list", { name: "Guest access" }),
      ).not.toBeInTheDocument();
    expect(readinessReads).toBe(0);
  },
);

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
  expect(within(sheet).queryByLabelText("Grants", { selector: "form" })).not.toBeInTheDocument();
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
  const grants = within(sheet).getByLabelText("Grants", { selector: "form" });
  expect(within(grants).getByRole("checkbox", { name: /GitHub/ })).toBeDisabled();
  expect(within(grants).queryByRole("button", { name: "Save grants" })).not.toBeInTheDocument();
  await user.click(within(sheet).getByText("Overview", { selector: '[role="tab"]' }));
  expect(within(sheet).getByLabelText("Role", { selector: "input,textarea" })).toBeDisabled();
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
  await user.click(within(sheet).getByText("Access", { selector: '[role="tab"]' }));
  expect(within(sheet).getByRole("region", { name: "Grants" })).toBeVisible();
  expect(within(sheet).queryByRole("button", { name: "Save grants" })).not.toBeInTheDocument();
  expect(within(sheet).queryByRole("note", { name: "Agent destroyed" })).not.toBeInTheDocument();
  expect(writes).toEqual([]);
});

it("shows no read-only note for a maintainable live specialist", async () => {
  renderSheet({ tab: "grants" });
  const sheet = await screen.findByRole("dialog", { name: "Agent details" });
  await within(sheet).findByLabelText("Grants", { selector: "form" });
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
  const profile = await screen.findByLabelText("Profile", { selector: "form" });
  const model = within(profile).getByLabelText("Agent model", { selector: '[role="combobox"]' });
  await waitFor(() => expect(model).toHaveTextContent("Default model"));
  await user.click(model);
  expect(await screen.findByRole("option", { name: "Retired (unavailable)" })).toHaveAttribute(
    "aria-disabled",
    "true",
  );
  await user.click(screen.getByRole("option", { name: "Research" }));
  const mode = within(profile).getByLabelText("Agent access mode", { selector: '[role="combobox"]' });
  expect(mode).toHaveTextContent("Workspace setting (Auto)");
  await user.click(mode);
  await user.click(await screen.findByRole("option", { name: "Ask every time" }));
  await user.click(screen.getByText("Save changes", { selector: "button" }));
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
  const profile = await screen.findByLabelText("Profile", { selector: "form" });
  const mode = within(profile).getByLabelText("Agent access mode", { selector: '[role="combobox"]' });
  expect(mode).toHaveTextContent("Full access");
  await user.click(mode);
  await user.click(await screen.findByRole("option", { name: "Workspace setting (Auto)" }));
  await user.click(screen.getByText("Save changes", { selector: "button" }));
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
  useAuthStore
    .getState()
    .setUser({
      ...owner,
      capabilities: { billing_available: false, enabled_features: [] },
    });
  renderSheet();
  const profile = await screen.findByLabelText("Profile", { selector: "form" });
  expect(within(profile).queryByLabelText("Agent model", { selector: '[role="combobox"]' })).not.toBeInTheDocument();
});

it("saves the optional channel progress reply without changing grants or prompt", async () => {
  const { user } = renderSheet();
  const sheet = await screen.findByRole("dialog", { name: "Agent details" });
  const profile = await within(sheet).findByLabelText("Profile", { selector: "form" });
  const progress = within(profile).getByLabelText("Channel progress reply", { selector: '[role="switch"]' });
  expect(progress).not.toBeChecked();
  await user.click(progress);
  await user.click(screen.getByText("Save changes", { selector: "button" }));
  await confirmChanges();
  await waitFor(() => expect(writes).toContainEqual({
    method: "PATCH",
    endpoint: "/assistant/nyxagent/agents/agent-researcher",
    body: { channel_checking_notice: true },
  }));
  await waitFor(() =>
    expect(
      screen.queryByText("Save changes", { selector: "button" }),
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
  const profile = await screen.findByLabelText("Profile", { selector: "form" });
  const role = within(profile).getByLabelText("Role", { selector: "input,textarea" });
  await user.clear(role);
  appendText(role, "Unsaved profile");
  await user.click(screen.getByText("Access", { selector: '[role="tab"]' }));
  const grants = screen.getByLabelText("Grants", { selector: "form" });
  await user.click(within(grants).getByRole("checkbox", { name: /Slack/ }));
  await user.click(screen.getByText("Overview", { selector: '[role="tab"]' }));
  expect(role).toHaveValue("Unsaved profile");
  await user.click(screen.getByText("Save changes", { selector: "button" }));
  await confirmChanges();
  await waitFor(() =>
    expect(
      screen.queryByText("Save changes", { selector: "button" }),
    ).not.toBeInTheDocument(),
  );
  await user.click(screen.getByText("Access", { selector: '[role="tab"]' }));
  expect(within(grants).getByRole("checkbox", { name: /Slack/ })).toBeChecked();
  expect(screen.queryByText("Save changes", { selector: "button" })).toBeNull();
  expect(writes.map((write) => write.method)).toEqual(["PATCH", "PUT"]);
  expect(document.querySelector("[data-grouped-list] [data-grouped-list]")).toBeNull();
});

it("refreshes pristine fields without replacing a dirty profile draft", async () => {
  const { user } = renderSheet();
  const profile = await screen.findByLabelText("Profile", { selector: "form" });
  const role = within(profile).getByLabelText("Role", { selector: "input,textarea" });
  await user.clear(role);
  appendText(role, "My draft");
  agent = { ...agent, description: "Remote role", display_name: "Remote name" };
  await client.invalidateQueries();
  await waitFor(() => expect(within(profile).getByLabelText("Display name", { selector: "input,textarea" })).toHaveValue("Remote name"));
  expect(role).toHaveValue("My draft");
  expect(screen.getByText("Save changes", { selector: "button" })).toBeEnabled();
  await user.click(screen.getByRole("button", { name: "Discard" }));
  expect(role).toHaveValue("Remote role");
});

it("keeps arrow navigation on the tablist after switching sections", async () => {
  const { user } = renderSheet();
  const first = await screen.findByText("Overview", { selector: '[role="tab"]' });
  act(() => first.focus());
  await user.keyboard("{ArrowRight}");
  await waitFor(() => expect(screen.getByText("Access", { selector: '[role="tab"]' })).toHaveFocus());
  await user.keyboard("{ArrowRight}");
  await waitFor(() => expect(screen.getByText("Skills", { selector: '[role="tab"]' })).toHaveFocus());
  expect(screen.getByText("Skills", { selector: '[role="tab"]' })).toHaveFocus();
  expect(screen.getByText("Skills", { selector: '[role="tab"]' })).toHaveAttribute("aria-selected", "true");
});

it("falls back to Overview for an unavailable Channels deep link", async () => {
  agent = agentRow({ owner_kind: "org", owner_id: "team", can_maintain: false });
  renderSheet({ tab: "channels" });
  const overview = await screen.findByRole("heading", { name: "Overview", level: 2 });
  await waitFor(() => expect(overview).toHaveFocus());
  expect(screen.queryByText("Channels", { selector: '[role="tab"]' })).not.toBeInTheDocument();
  expect(screen.queryByText("Memory", { selector: '[role="tab"]' })).not.toBeInTheDocument();
});

it("offers personal add-note and prevents every memory write after destruction", async () => {
  const { user } = renderSheet({ tab: "memory" });
  const note = await screen.findByLabelText("Note");
  appendText(note, "A personal convention");
  await user.click(screen.getByText("Save changes", { selector: "button" }));
  await confirmChanges();
  await waitFor(() => expect(writes).toContainEqual({ method: "POST", endpoint: "/assistant/nyxagent/agents/agent-researcher/memory", body: { text: "A personal convention" } }));
  agent = { ...agent, status: "destroyed", destroyed_at: at };
  await client.invalidateQueries();
  await waitFor(() => expect(screen.queryByRole("button", { name: "Save note" })).not.toBeInTheDocument());
  for (const button of screen.getAllByRole("button", { name: /^Forget:/ })) expect(button).toBeDisabled();
  expect(screen.queryByText("Channels", { selector: '[role="tab"]' })).not.toBeInTheDocument();
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
  await screen.findByText("Overview", { selector: '[role="tab"]' });
  expect(screen.queryByRole("note", { name: "Read-only" })).not.toBeInTheDocument();
  expect(screen.queryByText("Channels", { selector: '[role="tab"]' })).not.toBeInTheDocument();
  expect(screen.queryByLabelText("Persona", { selector: "input,textarea" }) !== null).toBe(share_role === "manager");
  await user.click(screen.getByText("Access", { selector: '[role="tab"]' }));
  expect(screen.queryByLabelText("Grants", { selector: "form" })).not.toBeInTheDocument();
  await user.click(screen.getByText("Memory", { selector: '[role="tab"]' }));
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
  const grants = await screen.findByLabelText("Grants", { selector: "form" });
  fireEvent.click(within(grants).getByRole("checkbox", { name: /Slack/ }));
  fireEvent.click(await screen.findByRole("button", { name: /Work Mac/ }));
  fireEvent.click(screen.getByLabelText("Shell commands"));
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
  await user.click(screen.getByText("Save changes", { selector: "button" }));
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
    await user.click(screen.getByText("Save changes", { selector: "button" }));
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
    await user.click(screen.getByText("Save changes", { selector: "button" }));
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
  await editGrantsAndMachine();
  fireEvent.click(screen.getByRole("button", { name: "Discard" }));
  await waitFor(() =>
    expect(
      screen.queryByRole("region", { name: "Unsaved changes" }),
    ).not.toBeInTheDocument(),
  );
  fireEvent.click(screen.getByRole("button", { name: "Back to Access" }));
  await waitFor(() => expect(screen.getByRole("checkbox", { name: /Slack/ })).not.toBeChecked());
  fireEvent.click(screen.getByRole("button", { name: /Work Mac/ }));
  expect(
    screen.getByLabelText("Shell commands"),
  ).not.toBeChecked();
  expect(writes).toEqual([]);
});

it.each(["X", "Escape"])("guards dirty sheet dismissal via %s and preserves edits on Keep editing", async (method) => {
  const onOpenChange = vi.fn();
  const { user } = renderSheet({ onOpenChange });
  const description = await screen.findByLabelText("Role");
  appendText(description, " unsaved");
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
  await screen.findByLabelText("Role");
  await user.click(screen.getByRole("button", { name: "Close agent details" }));
  expect(onOpenChange).toHaveBeenCalledWith(false);
  expect(screen.queryByRole("dialog", { name: "Discard unsaved changes?" })).toBeNull();
});

async function openSearch(user: ReturnType<typeof userEvent.setup>) {
  const toggle = await screen.findByLabelText(/^Search .+ agent details$/, { selector: "button[data-search-toggle]" });
  if (!toggle.hidden) await user.click(toggle);
  return screen.getByLabelText(/^Search .+ agent details$/, { selector: "input" });
}

it("searches across mounted tabs, reveals a memory row and moves focus to it", async () => {
  vi.useFakeTimers();
  renderSheet();
  await act(() => vi.advanceTimersByTimeAsync(100));
  fireEvent.click(screen.getByRole("button", { name: "Search researcher agent details" }));
  const search = screen.getByRole("searchbox");
  fireEvent.change(search, { target: { value: "weekly monday" } });
  const results = screen.getByRole("list", { name: "Memory search results" });
  expect(screen.getByRole("status")).toHaveTextContent("1 result for weekly monday");
  expect(within(results).getAllByRole("mark")).toHaveLength(2);
  expect(screen.queryByRole("form", { name: "Profile" })).not.toBeInTheDocument();
  fireEvent.click(within(results).getByRole("button", { name: /Weekly digests on Mondays Memory/ }));
  await act(() => vi.advanceTimersByTimeAsync(32));
  expect(search).toHaveValue("");
  expect(screen.getByText("Memory", { selector: '[role="tab"]' })).toHaveAttribute("aria-selected", "true");
  const target = document.querySelector('[data-drawer-search-target="memory-n1"]');
  expect(target).toHaveFocus();
  expect(target).toHaveAttribute("data-search-highlight");
  await act(() => vi.advanceTimersByTimeAsync(1600));
  expect(target).not.toHaveAttribute("data-search-highlight");
});

it("preserves profile and grants drafts and a single coordinated save bar while searching", async () => {
  const { user } = renderSheet();
  const role = await screen.findByLabelText("Role");
  await user.clear(role);
  appendText(role, "Fresh unsaved role");
  await user.click(screen.getByText("Access", { selector: '[role="tab"]' }));
  await user.click(screen.getByRole("checkbox", { name: /Slack/ }));
  const search = await openSearch(user);
  fireEvent.change(search, { target: { value: "fresh unsaved" } });
  const bars = screen.getAllByRole("region", { name: "Unsaved changes" });
  expect(bars).toHaveLength(1);
  expect(bars[0]).toHaveTextContent("Unsaved: profile, grants");
  await user.click(screen.getByLabelText("Role Overview › Profile", { selector: "button" }));
  expect(role).toHaveValue("Fresh unsaved role");
  await user.click(screen.getByText("Access", { selector: '[role="tab"]' }));
  expect(screen.getByRole("checkbox", { name: /Slack/ })).toBeChecked();
  await openSearch(user);
  fireEvent.change(search, { target: { value: "slack" } });
  await user.click(screen.getByText("Save changes", { selector: "button" }));
  await confirmChanges();
  await waitFor(() => expect(screen.queryByRole("region", { name: "Unsaved changes" })).not.toBeInTheDocument());
  expect(writes.map((write) => write.method)).toEqual(["PATCH", "PUT"]);
  expect(search).toHaveValue("slack");
});

it("clears the query, collapses empty search, then follows the dirty dismissal guard", async () => {
  const close = vi.fn();
  const { user } = renderSheet({ onOpenChange: close });
  const role = await screen.findByLabelText("Role");
  appendText(role, " draft");
  const search = await openSearch(user);
  fireEvent.change(search, { target: { value: "role" } });
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
  const tab = await screen.findByText("Overview", { selector: '[role="tab"]' });
  expect(screen.queryByRole("searchbox")).toBeNull();
  act(() => tab.focus());
  await user.keyboard("/");
  const search = await screen.findByRole("searchbox");
  expect(search).toHaveFocus();
  await openSearch(user);
  fireEvent.change(search, { target: { value: "account" } });
  await user.keyboard("{ArrowDown}");
  const result = document.activeElement!;
  expect(result).toHaveRole("button");
  await user.keyboard("{ArrowUp}{ArrowDown}{Enter}");
  await waitFor(() => expect(search).toHaveValue(""));
  expect(screen.getByText("Access", { selector: '[role="tab"]' })).toHaveAttribute("aria-selected", "true");
  const selected = screen.getByText("Access", { selector: '[role="tab"]' });
  act(() => selected.focus());
  await user.keyboard("{Control>}f{/Control}");
  expect(search).toHaveFocus();
  await user.click(screen.getByText("Overview", { selector: '[role="tab"]' }));
  const role = screen.getByLabelText("Role");
  await user.click(role);
  await user.keyboard("/");
  expect(role).toHaveFocus();
  expect(role).toHaveValue("Finds urgent issues/");
});

it("reveals a machine drill-in and guards leaving its dirty draft through search", async () => {
  vi.useFakeTimers();
  machineAccessFixture();
  renderSheet();
  await act(() => vi.advanceTimersByTimeAsync(100));
  fireEvent.click(screen.getByLabelText("Search researcher agent details", { selector: "button" }));
  const search = screen.getByLabelText("Search researcher agent details", { selector: "input" });
  fireEvent.change(search, { target: { value: "work mac" } });
  fireEvent.click(screen.getByLabelText("Work Mac Access › Machine access", { selector: "button" }));
  // Search reveals and focuses on a coalesced animation frame, not a CSS transition.
  await act(() => vi.advanceTimersByTimeAsync(32));
  const target = document.querySelector('[data-drawer-search-target="machine-detail-node"]')!;
  expect(target).toHaveFocus();
  const page = within(target.closest<HTMLElement>("[data-grouped-detail]")!);
  const shell = page.getByLabelText("Shell commands");
  fireEvent.click(shell);
  fireEvent.change(search, { target: { value: "persona" } });
  expect(search).toHaveValue("persona");
  expect(screen.getAllByRole("region", { name: "Unsaved changes" })).toHaveLength(1);
  fireEvent.click(screen.getByLabelText("Persona Overview › Profile", { selector: "button" }));
  const prompt = screen.getByRole("dialog", { name: "Discard unsaved changes?" });
  fireEvent.click(within(prompt).getByRole("button", { name: "Keep editing" }));
  await act(() => vi.advanceTimersByTimeAsync(32));
  expect(prompt).not.toBeInTheDocument();
  expect(search).toHaveValue("persona");
  fireEvent.click(screen.getByLabelText("Clear agent search"));
  expect(shell).toBeChecked();
  fireEvent.click(page.getByRole("button", { name: "Back to Access" }));
  const discard = screen.getByRole("dialog", { name: "Discard unsaved changes?" });
  fireEvent.click(within(discard).getByRole("button", { name: "Discard" }));
  await act(() => vi.advanceTimersByTimeAsync(32));
  expect(document.querySelector('[data-drawer-page-trigger="machine-detail-node"]')).toBeVisible();
});

it("searches only entries available to an organization viewer and preserves read-only controls", async () => {
  agent = agentRow({ owner_kind: "org", owner_id: "team", org_role: "viewer", can_maintain: false });
  const { user } = renderSheet();
  const search = await openSearch(user);
  appendText(search, "persona");
  await user.click(screen.getByLabelText("Persona Overview › Profile", { selector: "button" }));
  expect(screen.getByLabelText("Persona")).toBeDisabled();
  for (const query of ["connect another bot", "weekly digests", "destroy", "share agent", "find skills", "read my account"]) {
    await openSearch(user);
    fireEvent.change(search, { target: { value: query } });
    expect(screen.getByText(`No results for “${query}”.`)).toBeInTheDocument();
  }
  expect(writes).toHaveLength(0);
});

it("keeps destroyed-agent search available with deletion and remembered notes but no channel actions", async () => {
  agent = agentRow({ status: "destroyed", destroyed_at: at });
  const { user } = renderSheet();
  const search = await openSearch(user);
  fireEvent.change(search, { target: { value: "delete" } });
  expect(screen.getByRole("button", { name: "Delete permanently Overview" })).toBeInTheDocument();
  await openSearch(user);
  fireEvent.change(search, { target: { value: "weekly" } });
  expect(screen.getByRole("button", { name: /Weekly digests on Mondays Memory/ })).toBeInTheDocument();
  await openSearch(user);
  fireEvent.change(search, { target: { value: "connect another bot" } });
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
  fireEvent.change(search, { target: { value: "notion operations" } });
  await user.click(await screen.findByLabelText("Notion · Operations Access › Service operations", { selector: "button" }));
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
  fireEvent.change(search, { target: { value: "cafe 1.2.3" } });
  await user.click(await screen.findByLabelText("kek-faq-reference Skills › kek-faq-reference", { selector: "button" }));
  await waitFor(() => expect(document.querySelector('[data-drawer-search-target="skill-faq"]')).toHaveFocus());
  expect(screen.getByRole("heading", { name: "kek-faq-reference" })).toBeVisible();
  await openSearch(user);
  fireEvent.change(search, { target: { value: "history" } });
  await user.click(screen.getByLabelText("History Skills › History", { selector: "button" }));
  await waitFor(() => expect(document.querySelector('[data-drawer-search-target="skill-history"]')).toHaveFocus());
  expect(screen.getByRole("list", { name: "Skill history" })).toHaveTextContent("Attached skills changed");
  await user.click(screen.getByRole("button", { name: "Back to Skills" }));
  expect(screen.getByRole("button", { name: "History" })).toHaveFocus();
  expect(screen.getByRole("button", { name: /kek-faq-reference.*1.2.3/ })).toBeVisible();
});

it("indexes connected bot handles, unopened chats and available bots, revealing the exact channel page", async () => {
  vi.useFakeTimers();
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
  renderSheet();
  await act(() => vi.advanceTimersByTimeAsync(100));
  fireEvent.click(screen.getByLabelText("Search researcher agent details", { selector: "button" }));
  const search = screen.getByLabelText("Search researcher agent details", { selector: "input" });
  fireEvent.change(search, { target: { value: "helper_bot" } });
  expect(screen.getByLabelText("Helper Channels › Connected bot", { selector: "button" })).toBeInTheDocument();
  fireEvent.change(search, { target: { value: "cafe room" } });
  fireEvent.click(screen.getByLabelText("Café research room Channels › Helper › Chats", { selector: "button" }));
  await act(() => vi.advanceTimersByTimeAsync(32));
  const chat = document.querySelector('[data-drawer-search-target="channel-chat-room"]')!;
  expect(chat).toHaveFocus();
  expect(chat).toBeVisible();
  expect(chat).toHaveTextContent("Café research room · Group");
  fireEvent.change(search, { target: { value: "help desk" } });
  // Leaving the previous drill-in resolves its guard before scheduling the focus frame.
  await act(async () => fireEvent.click(screen.getByLabelText("Help desk Channels › Available bots", { selector: "button" })));
  await act(() => vi.advanceTimersByTimeAsync(32));
  const bot = document.querySelector('[data-drawer-search-target="available-bot-new-bot"]')!;
  expect(bot).toHaveFocus();
  expect(within(bot as HTMLElement).getByRole("button", { name: "Connect Help desk to researcher" })).toBeVisible();
  expect(bot).toBeVisible();
  const page = within(bot.closest<HTMLElement>("[data-grouped-detail]")!);
  fireEvent.click(page.getByRole("button", { name: "Back to Channels" }));
  await act(() => vi.advanceTimersByTimeAsync(32));
  const channels = within(screen.getByRole("tabpanel", { name: "Channels" }));
  expect(channels.getByRole("button", { name: "Chats and who can talk" })).toBeVisible();
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
  fireEvent.change(search, { target: { value: "friendly" } });
  expect(screen.getByRole("button", { name: "Persona Overview" })).toBeInTheDocument();
  await openSearch(user);
  fireEvent.change(search, { target: { value: "cafe convention" } });
  expect((await screen.findAllByRole("button", { name: /Memory/ })).length).toBeGreaterThan(0);
  await openSearch(user);
  fireEvent.change(search, { target: { value: "connect another bot" } });
  expect(screen.getByText("No results for “connect another bot”.")).toBeInTheDocument();
  expect(writes).toHaveLength(0);
});

it("indexes NyxBot's full-access row without specialist-only profile or lifecycle entries", async () => {
  agent = agentRow({ kind: "nyxbot", name: "nyxbot" });
  const { user } = renderSheet();
  const search = await openSearch(user);
  fireEvent.change(search, { target: { value: "connected services account" } });
  await user.click(screen.getByRole("button", { name: "Connected services and account Access" }));
  await waitFor(() => expect(document.querySelector('[data-drawer-search-target="nyxbot-full-access"]')).toHaveFocus());
  expect(screen.queryByLabelText("Grants", { selector: "form" })).not.toBeInTheDocument();
  await openSearch(user);
  fireEvent.change(search, { target: { value: "destroy" } });
  expect(screen.getByText("No results for “destroy”.")).toBeInTheDocument();
});

it("honors an incoming tab deep link while searching and preserves the mounted profile draft", async () => {
  const props = { agentId: String(agent.id), agents: [], open: true, onOpenChange: vi.fn(), onDeleted: vi.fn() };
  const { rerender } = render(<QueryClientProvider client={client}><AgentDetailsSheet {...props} /></QueryClientProvider>);
  const user = userEvent.setup({ delay: null, pointerEventsCheck: 0 });
  const role = await screen.findByLabelText("Role");
  appendText(role, " draft");
  const search = await openSearch(user);
  fireEvent.change(search, { target: { value: "missing" } });
  rerender(<QueryClientProvider client={client}><AgentDetailsSheet {...props} tab="skills" /></QueryClientProvider>);
  await waitFor(() => expect(search).toHaveValue(""));
  await waitFor(() => expect(screen.getByRole("heading", { name: "Skills", level: 2 })).toHaveFocus());
  await user.click(screen.getByText("Overview", { selector: '[role="tab"]' }));
  expect(role).toHaveValue("Finds urgent issues draft");
});

it("pins the same save bar outside scrolling content through tabs, drill-ins and search", async () => {
  machineAccessFixture();
  const { user } = renderSheet();
  appendText(await screen.findByLabelText("Role"), " draft");
  const bar = screen.getByRole("region", { name: "Unsaved changes" });
  expect(bar).toHaveAttribute("data-viewport-save-bar");
  expect(bar.closest("[data-form-viewport], [data-compact-scroll], [data-panel]")).toBeNull();
  await user.click(screen.getByText("Access", { selector: '[role="tab"]' }));
  expect(bar).toBeVisible();
  await user.click(await screen.findByRole("button", { name: /Work Mac/ }));
  expect(bar).toBeVisible();
  expect(bar.closest("[inert], [aria-hidden=true]")).toBeNull();
  appendText(await openSearch(user), "persona");
  expect(bar).toBeVisible();
  expect(screen.getAllByRole("button", { name: "Save changes" })).toHaveLength(1);
  expect(screen.getByRole("button", { name: "Discard" })).toBeVisible();
});

it("adds named help to every rendered drawer section and keeps help text searchable", async () => {
  const { user } = renderSheet();
  await screen.findByLabelText("Profile", { selector: "form" });
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
  await user.click(screen.getByText("Access", { selector: '[role="tab"]' }));
  await user.click(screen.getByRole("button", { name: "About Services" }));
  expect(await screen.findByRole("tooltip", { name: "About Services" })).toHaveTextContent("which connected services this specialist may call");
  await user.keyboard("{Escape}");
  appendText(await openSearch(user), "connected services specialist");
  expect(screen.getByRole("list", { name: "Access search results" })).toHaveTextContent("Services");
});

it("reviews profile values before saving and keeps Enter on the same review path", async () => {
  const { user } = renderSheet();
  const role = await screen.findByLabelText("Role");
  await user.clear(role);
  appendText(role, "Review this role");
  await user.click(screen.getByLabelText("Name"));
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
  await user.click(screen.getByText("Skills", { selector: '[role="tab"]' }));
  expect(screen.queryByRole("searchbox")).toBeNull();
  await openSearch(user);
  appendText(input, "persona");
  await user.click(screen.getByText("Overview", { selector: '[role="tab"]' }));
  expect(input).toBeVisible();
  expect(input).toHaveValue("persona");
});

it("uses a leading search glyph and clears then closes search without losing its draft", async () => {
  const { user } = renderSheet();
  const role = await screen.findByLabelText("Role");
  await user.clear(role);
  appendText(role, "Retain this role");
  await openSearch(user);
  appendText(screen.getByRole("searchbox"), "persona");
  expect(screen.queryByRole("button", { name: "Close agent search" })).toBeNull();
  expect(document.querySelector(".agent-header-search-glyph")).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Clear agent search" }));
  expect(screen.getByRole("searchbox")).toHaveValue("");
  await user.keyboard("{Escape}");
  expect(screen.queryByRole("searchbox")).toBeNull();
  expect(screen.getByRole("dialog", { name: "Agent details" })).toBeVisible();
  await waitFor(() => expect(screen.getByRole("button", { name: "Search researcher agent details" })).toHaveFocus());
  expect(role).toHaveValue("Retain this role");
  expect(screen.getByRole("region", { name: "Unsaved changes" })).toBeVisible();
});
