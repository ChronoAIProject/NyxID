import assert from "node:assert/strict";
import test from "node:test";
import { agentSchema } from "../../features/agents/agentContracts";
import { conversationSchema } from "../../features/assistant/assistantTypes";
import { groupSchema } from "../../features/groups/groupContracts";
import {
  sidebarBottomSpace,
  sidebarOrderedAgents,
  workspaceRecents,
} from "./workspaceSidebarModel";
const chat = (n: number, date: string, overrides: object = {}) =>
  conversationSchema.parse({
    id: `nyxa-${String(n).padStart(32, "0")}`,
    title: `Thread ${n}`,
    model: "test",
    created_at: date,
    last_message_at: date,
    message_count: 1,
    active_turn: null,
    ...overrides,
  });
const crew = groupSchema.parse({
  id: `nyxg-${"9".padStart(32, "0")}`,
  name: "Review crew",
  members: [],
  lead_agent_id: "nyxbot",
  created_at: "2026-10-25T00:00:00Z",
  last_message_at: "2026-10-27T12:00:00Z",
  message_count: 5,
  working_agent_ids: ["nyxbot"],
});
test("Recents merges actual agent/crew activity newest first, deduplicates pages and preserves exact targets/status", () => {
  const old = chat(1, "2026-10-26T00:00:00Z");
  const renamed = chat(1, "2026-10-28T00:00:00Z", {
    title: "Renamed",
    pending_acknowledgements: 2,
    agent: { id: "specialist", name: "Agent", kind: "specialist" },
  });
  const rows = workspaceRecents(
    [
      old,
      renamed,
      chat(2, "2026-10-27T00:00:00Z"),
      chat(3, "2026-10-28T00:00:00Z", { message_count: 0 }),
    ],
    [crew, { ...crew, id: `nyxg-${"8".padStart(32, "0")}`, message_count: 0 }],
  );
  assert.deepEqual(
    rows.map((row) => row.title),
    ["Renamed", "Review crew", "Thread 2"],
  );
  assert.deepEqual(rows[0]!.target, {
    kind: "agent",
    agentId: "specialist",
    conversationId: old.id,
  });
  assert.equal(rows[0]!.status, "Needs you");
  assert.deepEqual(rows[1]!.target, { kind: "group", groupId: crew.id });
  assert.equal(rows[1]!.status, "Running");
  assert.deepEqual(
    workspaceRecents([renamed, old], [crew]),
    workspaceRecents([old, renamed], [crew]),
  );
  assert.deepEqual(workspaceRecents([], []), []);
});
test("owner sections put Personal/NyxBot first and keep organizations with identical names distinct", () => {
  const agent = (id: string, more: object = {}) =>
    agentSchema.parse({
      id,
      name: id,
      kind: "specialist",
      description: "",
      status: "idle",
      ...more,
    });
  const rows = [
    agent("org-b", { owner_kind: "org", owner_id: "b", owner_name: "Shared" }),
    agent("personal"),
    agent("nyxbot", { kind: "nyxbot" }),
    agent("org-a", { owner_kind: "org", owner_id: "a", owner_name: "Shared" }),
  ];
  assert.deepEqual(
    sidebarOrderedAgents(rows).map((row) => row.id),
    ["nyxbot", "personal", "org-a", "org-b"],
  );
  assert.equal(rows[0]!.id, "org-b");
});
test("bottom reservation grows with measured controls and safe area, keeping the final row twelve points above the bar", () => {
  for (const bar of [44, 52, 76, 104])
    for (const inset of [0, 21, 34])
      for (const viewport of [600, 874, 1024]) {
        const lastRowEnd = viewport - sidebarBottomSpace(bar, inset);
        const barTop = viewport - inset - 12 - bar;
        assert.equal(barTop - lastRowEnd, 12);
      }
});
