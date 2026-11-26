import assert from "node:assert/strict";
import test from "node:test";
import { agentSchema } from "../../features/agents/agentContracts";
import { workspaceAgentPickerSections } from "./workspaceAgentPickerModel";
import type { WorkspaceRecent } from "./workspaceSidebarModel";

const agent = (id: string, patch: Record<string, unknown> = {}) =>
  agentSchema.parse({
    id,
    name: id,
    display_name: id,
    kind: id === "nyxbot" ? "nyxbot" : "specialist",
    description: "Assistant",
    status: "idle",
    can_use: true,
    ...patch,
  });
const roster = [
  agent("nyxbot"),
  agent("research"),
  agent("writer"),
  agent("ops"),
  agent("design"),
];
const thread = (
  agentId: string | undefined,
  n: number,
  status?: string,
): WorkspaceRecent => ({
  key: `chat:${n}`,
  title: `Thread ${n}`,
  activityAt: `2026-11-${String(20 - n).padStart(2, "0")}T00:00:00Z`,
  status,
  target: { kind: "agent", agentId, conversationId: `nyxa-${n}` },
});
const ids = (rows: readonly { id: string }[]) => rows.map((row) => row.id);

test("Recent keeps Recents' newest-first order, one row per agent, capped at three", () => {
  const picker = workspaceAgentPickerSections(roster, [
    thread("writer", 1, "Running"),
    thread("writer", 2),
    thread("research", 3),
    thread("ops", 4, "Needs you"),
    thread("design", 5),
  ]);
  assert.deepEqual(
    picker.recent.map((entry) => entry.agent.id),
    ["writer", "research", "ops"],
  );
  // Each agent keeps its newest thread and that thread's working indicator.
  assert.equal(picker.recent[0]!.row.key, "chat:1");
  assert.equal(picker.recent[0]!.row.status, "Running");
  assert.equal(picker.recent[2]!.row.status, "Needs you");
  assert.deepEqual(ids(picker.roster), ["nyxbot", "design"]);
});

test("the roster never repeats a Recent agent and keeps its own order", () => {
  const picker = workspaceAgentPickerSections(roster, [
    thread("design", 1),
    thread("nyxbot", 2),
  ]);
  assert.deepEqual(ids(picker.roster), ["research", "writer", "ops"]);
  const shown = [...picker.recent.map((entry) => entry.agent.id), ...ids(picker.roster)];
  assert.equal(new Set(shown).size, roster.length);
});

test("legacy unscoped threads belong to NyxBot and resolve to its current id", () => {
  const picker = workspaceAgentPickerSections(roster, [thread(undefined, 1)]);
  assert.equal(picker.recent[0]!.agent.id, "nyxbot");
  assert.deepEqual(picker.recent[0]!.row.target, {
    kind: "agent",
    agentId: "nyxbot",
    conversationId: "nyxa-1",
  });
});

test("crews, thread-less targets and unusable or unknown agents never enter Recent", () => {
  const agents = [
    ...roster,
    agent("viewer", { can_use: false }),
    agent("gone", { status: "destroyed" }),
  ];
  const picker = workspaceAgentPickerSections(agents, [
    {
      key: "crew:g",
      title: "Launch crew",
      activityAt: "2026-11-19T00:00:00Z",
      target: { kind: "group", groupId: "nyxg-1" },
    },
    {
      key: "agent:writer",
      title: "Writer",
      activityAt: "2026-11-18T00:00:00Z",
      target: { kind: "agent", agentId: "writer" },
    },
    thread("viewer", 3),
    thread("gone", 4),
    thread("missing", 5),
    thread("ops", 6),
  ]);
  assert.deepEqual(
    picker.recent.map((entry) => entry.agent.id),
    ["ops"],
  );
});

test("without history the picker is unchanged", () => {
  const picker = workspaceAgentPickerSections(roster, []);
  assert.deepEqual(picker.recent, []);
  assert.deepEqual(ids(picker.roster), ids(roster));
});
