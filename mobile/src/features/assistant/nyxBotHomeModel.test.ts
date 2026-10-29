import assert from "node:assert/strict";
import test from "node:test";
import { agentSchema } from "../agents/agentContracts";
import {
  agentOwnerSections,
  awayItems,
  greetingFor,
  showNyxBotHome,
} from "./nyxBotHomeModel";

const agent = (id: string, patch: object = {}) =>
  agentSchema.parse({
    id,
    name: id,
    kind: "specialist",
    status: "idle",
    description: "",
    ...patch,
  });
const reply = (created_at: string, text = "Finished the review.") => ({
  seq: 4,
  status: "completed",
  text,
  created_at,
});
const conversationId = `nyxa-${"a".repeat(32)}`;

test("greeting follows the web's local-time boundaries", () => {
  for (const [hour, expected] of [
    [0, "Good evening"],
    [4, "Good evening"],
    [5, "Good morning"],
    [11, "Good morning"],
    [12, "Good afternoon"],
    [17, "Good afternoon"],
    [18, "Good evening"],
    [23, "Good evening"],
  ] as const) {
    assert.equal(greetingFor(new Date(2026, 9, 28, hour)), expected);
  }
});

test("home appears only in the idle primary NyxBot state, including the route adoption boundary", () => {
  const idle = {
    home: true,
    contextual: false,
    nyxbot: true,
    sending: false,
    loading: false,
    inputAttachments: [],
  };
  assert.equal(showNyxBotHome(idle), true);
  for (const patch of [
    { home: false },
    { contextual: true },
    { nyxbot: false },
    { conversationId },
    { routeConversationId: conversationId },
    { sending: true },
    { loading: true },
    { openLatest: true },
    { inputText: "Sending" },
    { inputAttachments: [{}] },
  ]) {
    assert.equal(
      showNyxBotHome({ ...idle, ...patch }),
      false,
      JSON.stringify(patch),
    );
  }
});

test("away decisions precede newest work and carry the exact request/conversation identifiers", () => {
  const request = {
    request_id: "decision-one",
    conversation_id: conversationId,
    summary: "Approve publication",
  };
  const waiting = agent("waiting", {
    pending_requests: [request],
    last_reply: reply("2026-10-26T10:00:00Z"),
  });
  const finished = agent("finished", {
    last_reply: reply(
      "2026-10-27T10:00:00Z",
      "A\n long   reply " + "x".repeat(180),
    ),
  });
  const running = agent("running", {
    status: "running",
    last_active_at: "2026-10-28T10:00:00Z",
  });
  const ignored = [
    agent("destroyed", { status: "destroyed", pending_requests: [request] }),
    agent("nyxbot", {
      kind: "nyxbot",
      last_reply: reply("2026-10-29T10:00:00Z"),
    }),
    agent("quiet"),
  ];
  const rows = awayItems([waiting, finished, running, ...ignored]);
  assert.deepEqual(
    rows.map((row) => row.key),
    [
      "request:decision-one",
      "update:running",
      "update:finished",
      "update:waiting",
    ],
  );
  assert.equal(rows[0]!.kind, "request");
  if (rows[0]!.kind === "request") assert.deepEqual(rows[0]!.request, request);
  if (rows[1]!.kind === "update")
    assert.equal(rows[1]!.text, "Working on a task.");
  if (rows[2]!.kind === "update") {
    assert.equal(rows[2]!.text.length, 160);
    assert.match(rows[2]!.text, /^A long reply /);
    assert.ok(rows[2]!.text.endsWith("…"));
  }
});

test("missing optional summaries are empty and empty reply text never invents ongoing work", () => {
  assert.deepEqual(awayItems([agent("old-server")]), []);
  const rows = awayItems([
    agent("blank", { last_reply: reply("2026-10-28T10:00:00Z", "") }),
  ]);
  assert.equal(rows[0]?.kind, "update");
  if (rows[0]?.kind === "update") assert.equal(rows[0].text, "");
});

test("roster is grouped by exact owner with Personal and NyxBot first, excluding destroyed agents", () => {
  const organization = { owner_kind: "org", owner_name: "Same organization" };
  const input = [
    agent("org-b", { ...organization, owner_id: "b" }),
    agent("specialist"),
    agent("org-a", { ...organization, owner_id: "a" }),
    agent("nyxbot", { kind: "nyxbot" }),
    agent("past", { status: "destroyed" }),
  ];
  const sections = agentOwnerSections(input);
  assert.deepEqual(
    sections.map((section) => section.id),
    ["personal", "b", "a"],
  );
  assert.deepEqual(
    sections[0]!.agents.map((agent) => agent.id),
    ["nyxbot", "specialist"],
  );
  assert.deepEqual(
    input.map((agent) => agent.id),
    ["org-b", "specialist", "org-a", "nyxbot", "past"],
  );
});
