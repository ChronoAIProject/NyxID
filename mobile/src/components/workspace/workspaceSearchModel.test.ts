import assert from "node:assert/strict";
import test from "node:test";
import { agentSchema } from "../../features/agents/agentContracts";
import { conversationSchema } from "../../features/assistant/assistantTypes";
import { groupSchema } from "../../features/groups/groupContracts";
import {
  filterWorkspaceRecents,
  workspaceSearchMatches,
  workspaceSearchSections,
} from "./workspaceSearchModel";
import { workspaceRecents } from "./workspaceSidebarModel";

const agent = (id: string, display_name: string, status = "idle") =>
  agentSchema.parse({
    id,
    name: `${id}-handle`,
    display_name,
    kind: id === "nyxbot" ? "nyxbot" : "specialist",
    status,
    description: "Agent",
    can_use: true,
  });
const agents = [
  agent("nyxbot", "NyxBot"),
  agent("research", "Research helper"),
  agent("destroyed", "Gone helper", "destroyed"),
];
const chat = (id: number, title: string, agentId = "research") =>
  conversationSchema.parse({
    id: `nyxa-${String(id).padStart(32, "0")}`,
    title,
    model: "server-default",
    created_at: "2026-10-01T00:00:00Z",
    last_message_at: `2026-10-${String(10 + id).padStart(2, "0")}T00:00:00Z`,
    message_count: 2,
    active_turn: null,
    agent: {
      id: agentId,
      name: `${agentId}-handle`,
      kind: agentId === "nyxbot" ? "nyxbot" : "specialist",
    },
  });
const crew = groupSchema.parse({
  id: "nyxg-11111111111111111111111111111111",
  members: [],
  lead_agent_id: "nyxbot",
  name: "Design review",
  created_at: "2026-10-01T00:00:00Z",
  last_message_at: "2026-10-28T00:00:00Z",
  message_count: 2,
});
const entries = (query: string) =>
  workspaceSearchSections(
    query,
    [chat(1, "Release findings"), chat(2, "NyxBot release", "nyxbot")],
    agents,
    [crew],
  ).flatMap((s) => s.data);

test("search uses every typed word across title, display name and handle; opaque IDs do not match", () => {
  assert.equal(
    workspaceSearchMatches(
      "release HELPER",
      "Release findings",
      "Research helper",
    ),
    true,
  );
  assert.equal(
    workspaceSearchMatches(
      "release absent",
      "Release findings",
      "Research helper",
    ),
    false,
  );
  assert.deepEqual(
    entries("findings helper").map((e) => e.key),
    [`chat:${chat(1, "").id}`],
  );
  assert.deepEqual(
    entries("research-handle")
      .filter((e) => e.key.startsWith("agent:"))
      .map((e) => e.label),
    ["Research helper"],
  );
  assert.equal(entries("gone").length, 0);
  assert.equal(entries("crew-id").length, 0);
  assert.equal(entries("nyxa-").length, 0);
});

test("empty query shows eight newest real recents and pages; typed query groups chats, agents and crews", () => {
  const rows = Array.from({ length: 10 }, (_, n) =>
    chat(n + 1, "Release review"),
  );
  const blank = workspaceSearchSections(" ", rows, agents, [crew]);
  assert.deepEqual(
    blank.map((s) => s.title),
    ["Recent chats", "Go to"],
  );
  assert.equal(blank[0]!.data.length, 8);
  assert.equal(blank[0]!.data[0]!.key, `crew:${crew.id}`);
  assert.deepEqual(
    workspaceSearchSections("review", rows, agents, [crew]).map((s) => s.title),
    ["Chats", "Crews"],
  );
  const research = workspaceSearchSections(
    "research",
    [chat(1, "Findings")],
    agents,
    [],
  );
  assert.deepEqual(
    research.map((s) => s.title),
    ["Chats", "Agents"],
  );
  assert.equal(
    workspaceSearchSections("new phrase unmatched", rows, agents, [crew])
      .length,
    0,
  );
});

test("search routes exact chats, normal new-agent chats, crews, native settings and supported pages", () => {
  assert.deepEqual(entries("findings")[0]!.target, {
    kind: "chat",
    target: {
      kind: "agent",
      agentId: "research",
      conversationId: chat(1, "").id,
    },
  });
  assert.deepEqual(
    entries("nyxbot").find((e) => e.key === "agent:nyxbot")!.target,
    { kind: "chat", target: { kind: "agent", agentId: "nyxbot" } },
  );
  assert.deepEqual(entries("design")[0]!.target, {
    kind: "chat",
    target: { kind: "group", groupId: crew.id },
  });
  assert.deepEqual(entries("saved password")[0]!.target, {
    kind: "page",
    name: "Machines",
    params: { tab: "logins" },
  });
  assert.deepEqual(entries("settings text size")[0]!.target, {
    kind: "settings",
    name: "AccountSettings",
    params: { section: "display" },
  });
  assert.deepEqual(entries("appearance dark")[0]!.target, {
    kind: "theme",
    preference: "dark",
  });
  const all = entries("").concat(entries("settings"));
  assert.ok(
    all.every((e) => !/soon|artifacts|page:home/i.test(`${e.key} ${e.label}`)),
  );
  assert.ok(entries("approvals").some((e) => e.label === "Activity"));
});

test("See all preserves newest ordering, filters by agent and title, and never confuses crew identity", () => {
  const rows = workspaceRecents(
    [chat(1, "Release findings"), chat(2, "Other findings", "nyxbot")],
    [crew],
  );
  assert.equal(filterWorkspaceRecents(rows, "")[0]!.key, `crew:${crew.id}`);
  assert.deepEqual(
    filterWorkspaceRecents(rows, "findings", "research").map((r) => r.title),
    ["Release findings"],
  );
  assert.equal(filterWorkspaceRecents(rows, "Design", "research").length, 0);
  assert.deepEqual(
    filterWorkspaceRecents(rows, "design").map((r) => r.key),
    [`crew:${crew.id}`],
  );
});
