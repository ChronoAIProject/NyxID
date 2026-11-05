import assert from "node:assert/strict";
import test from "node:test";
import type { AssistantAgent } from "../agents/agentContracts";
import { chatAgentLabel, resolveChatAgentId } from "./assistantAgentContext";

const agents: Pick<AssistantAgent, "id" | "kind">[] = [
  { id: "org-specialist", kind: "specialist" },
  { id: "personal-nyxbot", kind: "nyxbot" },
  { id: "personal-specialist", kind: "specialist" },
];

test("a saved specialist thread keeps its agent when the route names another agent", () => {
  assert.equal(
    resolveChatAgentId(agents, "org-specialist", "personal-nyxbot"),
    "org-specialist",
  );
});

test("a new draft carries the resolved owner of the previous specialist thread", () => {
  const currentAgentId = resolveChatAgentId(
    agents,
    "org-specialist",
    undefined,
  );
  assert.equal(
    resolveChatAgentId(agents, undefined, currentAgentId),
    "org-specialist",
  );
});

test("default chat selects NyxBot rather than the first organization specialist", () => {
  assert.equal(
    resolveChatAgentId(agents, undefined, undefined),
    "personal-nyxbot",
  );
});

test("an explicit route retains an unloaded agent instead of switching to NyxBot", () => {
  assert.equal(
    resolveChatAgentId(agents, undefined, "not-yet-loaded"),
    "not-yet-loaded",
  );
  assert.equal(
    resolveChatAgentId(undefined, undefined, "not-yet-loaded"),
    "not-yet-loaded",
  );
});

test("default agent stays unresolved until a NyxBot is loaded", () => {
  assert.equal(resolveChatAgentId(undefined, undefined, undefined), undefined);
  assert.equal(resolveChatAgentId([], undefined, undefined), undefined);
  assert.equal(
    resolveChatAgentId([agents[0]!], undefined, undefined),
    undefined,
  );
});

test("conversation ownership survives an unloaded agent list", () => {
  assert.equal(
    resolveChatAgentId(undefined, "org-specialist", "personal-nyxbot"),
    "org-specialist",
  );
});

test("agent labels prefer a trimmed display name", () => {
  assert.equal(
    chatAgentLabel({
      name: "specialist",
      display_name: "  Research assistant  ",
    }),
    "Research assistant",
  );
});

test("agent labels fall back from an empty display name to the handle", () => {
  assert.equal(
    chatAgentLabel({ name: "  specialist  ", display_name: "  " }),
    "specialist",
  );
  assert.equal(
    chatAgentLabel({ name: "specialist", display_name: null }),
    "specialist",
  );
});

test("missing agent labels fall back to NyxBot", () => {
  assert.equal(chatAgentLabel(undefined), "NyxBot");
  assert.equal(chatAgentLabel(null), "NyxBot");
  assert.equal(chatAgentLabel({ name: "  ", display_name: "  " }), "NyxBot");
});
