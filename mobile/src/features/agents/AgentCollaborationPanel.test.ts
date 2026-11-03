import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { hookHarness } from "../assistant/assistantHookTestHarness";
import * as sharing from "./sharingContracts";
import * as operations from "./operationContracts";
import { agentSchema } from "./agentContracts";
import { operationPayload } from "../../lib/api/collaborationTransport";

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

function setup(queryOverride: Record<string, unknown> = {}) {
  let currentAgent = { ...agent, share_role: "manager" as const };
  const writes: Array<{ id: string; patch: Record<string, unknown> }> = [];
  const consentWrites: Array<{ id: string; decision: string }> = [];
  const operationWrites: Array<{
    id: string;
    guests: boolean;
    serviceId: string;
    selection: operations.OperationSelection;
  }> = [];
  const admissions: unknown[] = [];
  const queries: any[] = [];
  let actionError: unknown;
  let query: any = {
    data: [service],
    isError: false,
    isFetching: false,
    dataUpdatedAt: 100,
    ...queryOverride,
  };
  const harness = hookHarness<Element>({
    "react/jsx-runtime": {
      jsx: (type: unknown, props: unknown) => ({ type, props }),
      jsxs: (type: unknown, props: unknown) => ({ type, props }),
    },
    "react-native": { Alert: {}, Switch: "Switch", View: "View" },
    "@tanstack/react-query": {
      useQuery: (options: any) => {
        queries.push(options);
        return query;
      },
    },
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
    "../../lib/api/collaborationApi": {
      collaborationApi: {
        decideConsent: async (id: string, decision: string, options: any) => {
          options.authorityGuard();
          consentWrites.push({ id, decision });
        },
        saveOperations: async (
          id: string,
          guests: boolean,
          serviceId: string,
          selection: operations.OperationSelection,
          options: any,
        ) => {
          options.authorityGuard();
          operationWrites.push({ id, guests, serviceId, selection });
        },
      },
    },
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
    "../../lib/api/collaborationTransport": { operationPayload },
    "../groups/useCollaborationSession": {
      useCollaborationSession: () => session,
    },
    "./sharingContracts": sharing,
    "./operationContracts": operations,
  });
  const module = harness.load(
    fileURLToPath(new URL("./AgentCollaborationPanel.tsx", import.meta.url)),
  ) as any;
  const session = {
    key: ["collaboration", 1],
    epoch: 1,
    active: true,
    enabled: true,
    readable: true,
    identity: { isError: false },
    activationAt: 50,
    busy: false,
    valid: () => true,
    guard: () => {},
    run: async (
      action: () => Promise<any>,
      success?: (value: any) => void,
      authority?: unknown,
    ) => {
      try {
        admissions.push(authority);
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
    consentWrites,
    operationWrites,
    admissions,
    queries,
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

test("operation list keeps drafts through refresh but hides failed or pre-activation data", () => {
  const env = setup();
  env.harness.mount(() =>
    env.module.OperationList({ session: env.session, agent, guests: false }),
  );
  const drafts = () =>
    elements(env.harness.flush()).filter(
      (row) =>
        typeof row.type === "function" && row.type.name === "OperationDraft",
    ).length;
  assert.equal(drafts(), 1);
  // A background refetch (for example after saving another service) must not
  // unmount open drafts and discard their unsaved edits.
  env.setQuery({ dataUpdatedAt: 100, isFetching: true });
  assert.equal(drafts(), 1);
  for (const state of [
    { isFetching: false, isError: true },
    { isError: false, dataUpdatedAt: 10 },
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

test("owner and guest operation saves require a semantic change and do not dispatch unchanged drafts", async () => {
  for (const guests of [false, true]) {
    const env = setup();
    const current = {
      ...service,
      all_operations: false,
      endpoint_ids: ["read-a", "read-b"],
      inputs: { "read-a": { body: { project: "retained" } } },
      operations: ["read-a", "read-b"].map((endpoint_id) => ({
        endpoint_id,
        method: "GET",
        path: `/${endpoint_id}`,
        summary: null,
        read_only: true,
        changes_existing: false,
      })),
    };
    env.harness.mount(() =>
      env.module.OperationDraft({
        session: env.session,
        agent,
        service: current,
        guests,
      }),
    );
    const field = (label: string) =>
      elements(env.harness.flush()).find((row) => row.props.label === label)!;
    const save = () =>
      field(guests ? "Save guest operations" : "Save operations");
    assert.equal(save().props.disabled, true);
    save().props.onPress();
    await env.harness.settle();
    assert.equal(env.admissions.length, 0);
    assert.equal(env.operationWrites.length, 0);
    // Different ordering (including a duplicate) does not change an ID set.
    field("Selected endpoints").props.onChange(["read-b", "read-a", "read-a"]);
    assert.equal(save().props.disabled, true);
    field("Selected endpoints").props.onChange(["read-b"]);
    assert.equal(save().props.disabled, false);
    field("Selected endpoints").props.onChange(["read-a", "read-b"]);
    assert.equal(save().props.disabled, true);
    save().props.onPress();
    await env.harness.settle();
    assert.equal(env.admissions.length, 0);
    assert.equal(env.operationWrites.length, 0);
  }
});

test("all-operations mode ignores hidden selections and reverting mode restores the clean baseline", () => {
  for (const guests of [false, true]) {
    const env = setup();
    env.harness.mount(() =>
      env.module.OperationDraft({
        session: env.session,
        agent,
        service,
        guests,
      }),
    );
    const field = (label: string) =>
      elements(env.harness.flush()).find((row) => row.props.label === label)!;
    const save = () =>
      field(guests ? "Save guest operations" : "Save operations");
    const mode = () =>
      field(`GitHub ${guests ? "guest " : ""}operation access`);
    assert.equal(save().props.disabled, true);
    mode().props.onChange(["selected"]);
    assert.equal(save().props.disabled, false);
    mode().props.onChange(["all"]);
    assert.equal(save().props.disabled, true);
  }
});

test("explicit operation rules compare as sets and their edits remain in the submitted payload", async () => {
  const env = setup();
  env.setAgent({ share_role: undefined });
  const current = {
    ...service,
    all_operations: false,
    allows_explicit_rules: true,
    rules: [
      { method: "GET", path_template: "/a" },
      { method: "GET", path_template: "/b" },
    ],
  };
  env.harness.mount(() =>
    env.module.OperationDraft({
      session: env.session,
      agent,
      service: current,
      guests: false,
    }),
  );
  const field = (label: string) =>
    elements(env.harness.flush()).find((row) => row.props.label === label)!;
  assert.equal(field("Save operations").props.disabled, true);
  field("Rule 1 path").props.onChangeText("/b");
  field("Rule 2 path").props.onChangeText("/a");
  assert.equal(field("Save operations").props.disabled, true);
  field("Rule 1 path").props.onChangeText("/changed");
  assert.equal(field("Save operations").props.disabled, false);
  field("Save operations").props.onPress();
  await env.harness.settle();
  assert.equal(env.actionError, undefined);
  assert.deepEqual(
    JSON.parse(JSON.stringify(env.operationWrites[0]?.selection)),
    {
      expected_revision: 2,
      all_operations: false,
      endpoint_ids: [],
      rules: [
        { method: "GET", path_template: "/changed" },
        { method: "GET", path_template: "/a" },
      ],
    },
  );
});

test("select all reads uses the full live catalog and retains selected endpoint value limits", async () => {
  const env = setup();
  env.setAgent({ share_role: undefined });
  const op = (endpoint_id: string, extra: Record<string, unknown> = {}) => ({
    endpoint_id,
    method: "GET",
    path: `/${endpoint_id}`,
    summary: null,
    read_only: true,
    changes_existing: false,
    ...extra,
  });
  const current = {
    ...service,
    all_operations: false,
    endpoint_ids: ["read-a", "write", "removed"],
    inputs: {
      "read-a": { body: { project: "retained" } },
      write: { body: { project: "drop" } },
      removed: { body: { project: "drop" } },
    },
    operations: [
      op("read-a"),
      op("read-b"),
      op("write", { method: "POST", read_only: false }),
      op("changes-existing", { changes_existing: true }),
    ],
  };
  env.harness.mount(() =>
    env.module.OperationDraft({
      session: env.session,
      agent,
      service: current,
      guests: false,
    }),
  );
  const field = (label: string) =>
    elements(env.harness.flush()).find((row) => row.props.label === label)!;
  field("Search GitHub operations").props.onChangeText("POST");
  field("Select all reads").props.onPress();
  assert.deepEqual(Array.from(field("Selected endpoints").props.value), [
    "read-a",
    "read-b",
  ]);
  assert.equal(field("Save operations").props.disabled, false);
  field("Save operations").props.onPress();
  await env.harness.settle();
  assert.equal(env.actionError, undefined);
  assert.deepEqual(
    JSON.parse(JSON.stringify(env.operationWrites[0]?.selection)),
    {
      expected_revision: 2,
      all_operations: false,
      endpoint_ids: ["read-a", "read-b"],
      rules: [],
      inputs: { "read-a": { body: { project: "retained" } } },
    },
  );
});

const consent = {
  id: "consent-1",
  connection_id: "connection-1",
  connection_name: "GitHub",
  operation: "Read issues",
  decision: "always",
  expires_at: null,
};

test("flag rollback keeps consent management available without sharing policy authority", () => {
  for (const isError of [false, true]) {
    const env = setup({
      // A retained policy from before rollback must not expose creation/editing.
      data: { role: "owner", revision: 2 },
      isError,
    });
    env.session.enabled = false;
    env.harness.mount(() => env.module.AgentCollaborationPanel({ agent }));
    const controls = elements(env.harness.flush()).map((row) => row.type?.name);
    assert.ok(controls.includes("SharedConsentCards"));
    assert.ok(!controls.includes("SharingEditor"));
    assert.ok(!controls.includes("SharedProfile"));
    assert.ok(!controls.includes("SharedNotesEditor"));
    assert.equal(env.queries.at(-1).enabled, false);
    assert.doesNotMatch(
      text(env.harness.flush()),
      /Sharing settings are unavailable/,
    );
  }
});

test("flag-off consent inventory exposes only revocation, including pending cards", async () => {
  const env = setup({
    data: {
      consents: [consent, { ...consent, id: "pending", decision: "pending" }],
    },
  });
  env.session.enabled = false;
  env.harness.mount(() =>
    env.module.SharedConsentCards({ session: env.session, id: agent.id }),
  );
  const buttons = elements(env.harness.flush()).filter(
    (row) => row.type === "ActionButton",
  );
  assert.deepEqual(
    buttons.map((row) => row.props.label),
    ["Revoke consent", "Revoke consent", "Refresh connection consent"],
  );
  assert.match(text(env.harness.flush()), /sharing is disabled/);
  for (const button of buttons.slice(0, 2)) await button.props.onPress();
  assert.deepEqual(JSON.parse(JSON.stringify(env.consentWrites)), [
    { id: consent.id, decision: "revoked" },
    { id: "pending", decision: "revoked" },
  ]);
  assert.deepEqual(JSON.parse(JSON.stringify(env.admissions)), [
    { feature: null, capability: false },
    { feature: null, capability: false },
  ]);
});

test("positive consent decisions retain feature admission and hide failed or stale inventory", async () => {
  const env = setup({
    data: { consents: [{ ...consent, decision: "pending" }] },
  });
  env.harness.mount(() =>
    env.module.SharedConsentCards({ session: env.session, id: agent.id }),
  );
  const buttons = () =>
    elements(env.harness.flush()).filter(
      (row) =>
        row.type === "ActionButton" &&
        row.props.label !== "Refresh connection consent",
    );
  assert.deepEqual(
    buttons().map((row) => row.props.label),
    ["Allow once", "Always", "Skip"],
  );
  for (const button of buttons()) await button.props.onPress();
  assert.ok(env.admissions.every((authority) => authority === undefined));
  // The five-second poll must not make the decision buttons blink out.
  env.setQuery({ isFetching: true });
  assert.equal(buttons().length, 3);
  for (const state of [
    { isFetching: false, isError: true },
    { isError: false, dataUpdatedAt: 10 },
  ]) {
    env.setQuery(state);
    assert.equal(buttons().length, 0);
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
