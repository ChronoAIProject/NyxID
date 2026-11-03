import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { hookHarness } from "../assistant/assistantHookTestHarness";
import * as sharing from "./sharingContracts";
import * as operations from "./operationContracts";
import { agentSchema } from "./agentContracts";

type Element = { type: any; props: Record<string, any> };
const elements = (root: any): Element[] =>
  !root || typeof root !== "object" || !root.props
    ? []
    : [root, ...[root.props.children].flat(Infinity).flatMap(elements)];
const text = (root: any): string =>
  !root
    ? ""
    : typeof root === "string"
      ? root
      : typeof root === "object" && root.props
        ? [root.props.message, root.props.children]
            .flat(Infinity)
            .map(text)
            .join(" ")
        : "";
const agent = agentSchema.parse({
  id: "agent-1",
  kind: "specialist",
  name: "research",
  description: "Research",
  status: "idle",
  can_maintain: true,
  guest_readiness: { status: "ready" },
});
const service = {
  service_id: "exact",
  service_slug: "github",
  service_name: "GitHub",
  revision: 2,
  all_operations: true,
  allows_explicit_rules: false,
  endpoint_ids: [],
  rules: [],
  operations: [],
};

function setup() {
  let currentAgent = { ...agent, share_role: "manager" as const };
  const writes: Array<{ id: string; patch: Record<string, unknown> }> = [];
  let actionError: unknown;
  let query: any = {
    data: [service],
    isError: false,
    isFetching: false,
    dataUpdatedAt: 100,
  };
  const harness = hookHarness<Element>({
    "react/jsx-runtime": {
      jsx: (type: unknown, props: unknown) => ({ type, props }),
      jsxs: (type: unknown, props: unknown) => ({ type, props }),
    },
    "react-native": { Alert: {}, Switch: "Switch", View: "View" },
    "@tanstack/react-query": { useQuery: () => query },
    "../../components/workspace/WorkspaceUI": {
      ActionButton: "ActionButton",
      Card: "Card",
      Field: "Field",
      Label: "Label",
      Notice: "Notice",
    },
    "../../components/workspace/WorkspaceSelectionField": {
      WorkspaceSelectionField: "WorkspaceSelectionField",
    },
    "../../lib/api/collaborationApi": { collaborationApi: {} },
    "../../lib/api/agentsApi": {
      agentsApi: {
        detail: async () => ({ agent: currentAgent }),
        update: async (
          id: string,
          patch: Record<string, unknown>,
          options: any,
        ) => {
          options.authorityGuard();
          writes.push({ id, patch });
          currentAgent = { ...currentAgent, ...patch };
        },
      },
    },
    "../../lib/api/collaborationTransport": { operationPayload: () => {} },
    "../groups/useCollaborationSession": {
      useCollaborationSession: () => ({}),
    },
    "./sharingContracts": sharing,
    "./operationContracts": operations,
  });
  const module = harness.load(
    fileURLToPath(new URL("./AgentCollaborationPanel.tsx", import.meta.url)),
  ) as any;
  const session = {
    key: ["collaboration", 1],
    active: true,
    activationAt: 50,
    busy: false,
    valid: () => true,
    guard: () => {},
    run: async (action: () => Promise<any>, success?: (value: any) => void) => {
      try {
        const result = await action();
        success?.(result);
      } catch (error) {
        actionError = error;
      }
    },
  };
  return {
    harness,
    module,
    session,
    writes,
    get actionError() {
      return actionError;
    },
    setAgent: (patch: Record<string, unknown>) => {
      currentAgent = { ...currentAgent, ...patch };
    },
    setQuery: (next: any) => {
      query = { ...query, ...next };
      harness.render();
    },
  };
}

test("operation list hides retained data on failed query, background freshness gap or refresh", () => {
  const env = setup();
  env.harness.mount(() =>
    env.module.OperationList({ session: env.session, agent, guests: false }),
  );
  assert.equal(
    elements(env.harness.flush()).filter(
      (row) =>
        typeof row.type === "function" && row.type.name === "OperationDraft",
    ).length,
    1,
  );
  for (const state of [
    { isError: true },
    { isError: false, dataUpdatedAt: 10 },
    { dataUpdatedAt: 100, isFetching: true },
  ]) {
    env.setQuery(state);
    assert.equal(
      elements(env.harness.flush()).filter(
        (row) =>
          typeof row.type === "function" && row.type.name === "OperationDraft",
      ).length,
      0,
    );
  }
});

test("shared managers save only profile fields and can save again against the successful baseline", async () => {
  const env = setup();
  env.harness.mount(() =>
    env.module.SharedProfile({ session: env.session, agent }),
  );
  const field = (label: string) =>
    elements(env.harness.flush()).find((row) => row.props.label === label)!;
  field("Shared agent display name").props.onChangeText("Published name");
  env.harness.flush();
  field("Save shared profile").props.onPress();
  await env.harness.settle();
  assert.deepEqual(JSON.parse(JSON.stringify(env.writes)), [
    {
      id: agent.id,
      patch: {
        display_name: "Published name",
        description: "Research",
        persona: "",
      },
    },
  ]);
  field("Shared agent purpose").props.onChangeText("Second change");
  env.harness.flush();
  field("Save shared profile").props.onPress();
  await env.harness.settle();
  assert.equal(env.writes.length, 2);
  assert.equal(env.writes[1]?.patch.description, "Second change");
  assert.equal(env.actionError, undefined);
});

test("revoked managers and concurrent profile changes cannot publish a draft", async () => {
  for (const patch of [
    { share_role: "user" },
    { description: "Changed elsewhere" },
  ]) {
    const env = setup();
    env.harness.mount(() =>
      env.module.SharedProfile({ session: env.session, agent }),
    );
    const field = (label: string) =>
      elements(env.harness.flush()).find((row) => row.props.label === label)!;
    field("Shared agent purpose").props.onChangeText("Draft");
    env.harness.flush();
    env.setAgent(patch);
    field("Save shared profile").props.onPress();
    await env.harness.settle();
    assert.equal(env.writes.length, 0);
    assert.ok(env.actionError);
  }
});

test("visitor readiness never falls back to the stale agent ready flag after a failed or stale read", () => {
  const env = setup();
  env.harness.mount(() =>
    env.module.VisitorReadiness({ session: env.session, agent }),
  );
  env.setQuery({ data: { status: "ready" }, isError: true });
  assert.match(text(env.harness.flush()), /unverified/);
  assert.doesNotMatch(text(env.harness.flush()), /Model ready for visitors/);
  env.setQuery({ isError: false, dataUpdatedAt: 10 });
  assert.match(text(env.harness.flush()), /unverified/);
  env.setQuery({ dataUpdatedAt: 100 });
  assert.match(text(env.harness.flush()), /Model ready for visitors/);
});
