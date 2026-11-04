import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { deferred, hookHarness } from "../assistant/assistantHookTestHarness";
import {
  inviteToken,
  invitationLink,
} from "../../lib/api/collaborationTransport";
import { groupSchema } from "./groupContracts";

type Element = { type: any; props: Record<string, any> };
const elements = (root: any): Element[] =>
  !root || typeof root !== "object" || !root.props
    ? []
    : [root, ...[root.props.children].flat(Infinity).flatMap(elements)];
const TOKEN = "a".repeat(64);
const group = groupSchema.parse({
  id: "nyxg-0123456789abcdef0123456789abcdef",
  name: "Team",
  members: [{ id: "agent-1", name: "research", kind: "specialist" }],
  lead_agent_id: "agent-1",
  created_at: "2026-11-03T01:00:00Z",
  collaboration: {
    version: 1,
    creator_user_id: "actor",
    membership_revision: 7,
    history_disclosure: "Full history",
    can_manage: true,
    your_request_ids: [],
    transfer_to_user_id: null,
    guest_access: [],
    proposals: [],
    invites: [],
  },
});

function setup(
  component:
    | "CollaborationEntry"
    | "CollaborationGroupControls"
    | "GroupAgentAccessRequest",
  target: typeof group = group,
) {
  const session: any = {
    key: ["collaboration", 1],
    epoch: 1,
    active: true,
    readable: true,
    activationAt: 0,
    busy: false,
    enabled: true,
    identity: { data: { id: "actor" } },
    capability: {
      data: true,
      isError: false,
      isFetching: false,
      dataUpdatedAt: 100,
    },
    valid: () => session.active,
  };
  session.run = async (
    action: () => Promise<unknown>,
    success?: (result: unknown) => void,
  ) => {
    session.busy = true;
    session.error = undefined;
    harness.render();
    try {
      const result = await action();
      success?.(result);
      return result;
    } catch (cause) {
      session.error = cause instanceof Error ? cause.message : "Could not save";
      return undefined;
    } finally {
      session.busy = false;
      harness.render();
    }
  };
  const alerts: Array<{
    title: string;
    buttons: Array<{ text: string; onPress?: () => void }>;
  }> = [];
  const changes: unknown[] = [];
  const query = {
    fetching: false,
    error: false,
    refetch: async (): Promise<unknown> => undefined,
  };
  session.clearError = () => {
    session.error = undefined;
  };
  const mocks: Record<string, unknown> = {
    "../../components/Skeleton": { SkeletonRows: "SkeletonRows" },
    "react/jsx-runtime": {
      jsx: (type: unknown, props: unknown) => ({ type, props }),
      jsxs: (type: unknown, props: unknown) => ({ type, props }),
    },
    "react-native": {
      Alert: {
        alert: (
          title: string,
          _message: string,
          buttons: Array<{ text: string; onPress?: () => void }>,
        ) => alerts.push({ title, buttons }),
      },
      View: "View",
    },
    "expo-clipboard": { setStringAsync: async () => {} },
    "@tanstack/react-query": {
      useQuery: (options: any) => ({
        data: options.queryKey.includes("group")
          ? group
          : options.queryKey.includes("agents")
            ? { agents: [] }
            : [],
        isError: options.queryKey.includes("group") && query.error,
        isFetching: options.queryKey.includes("group") && query.fetching,
        dataUpdatedAt: 100,
        refetch: () => query.refetch(),
      }),
    },
    "../../components/workspace/WorkspaceUI": {
      ActionButton: "ActionButton",
      Card: "Card",
      Field: "Field",
      Label: "Label",
      Notice: "Notice",
      WorkspacePage: "WorkspacePage",
    },
    "../../components/workspace/WorkspaceSelectionField": {
      WorkspaceSelectionField: "WorkspaceSelectionField",
    },
    "../../lib/api/groupsApi": { groupsApi: { detail: async () => group } },
    "../../lib/api/agentsApi": { agentsApi: {} },
    "../../lib/api/collaborationApi": {
      collaborationApi: {
        change: async (_id: string, _revision: number, change: unknown) => {
          changes.push(change);
          return {
            group,
            invite: { token: TOKEN, expires_at: "2099-11-03T01:00:00Z" },
          };
        },
        askOwner: async (_group: string, agent: string) => {
          changes.push({ ask: agent });
          return { id: "req", status: "pending", expires_at: "2099-01-01" };
        },
        cancelGuestRequest: async (
          _group: string,
          agent: string,
          request: string,
        ) => {
          changes.push({ cancel: agent, request });
        },
        preview: async () => ({
          group_id: group.id,
          group_name: group.name,
          history_disclosure: "Full history",
          expires_at: "2099-11-03T01:00:00Z",
        }),
      },
    },
    "../../lib/api/collaborationTransport": { inviteToken, invitationLink },
    "../../lib/env": { NYXID_FRONTEND_URL: "https://nyxid.ai" },
    "./useCollaborationSession": { useCollaborationSession: () => session },
  };
  const harness = hookHarness<Element>(mocks);
  mocks["../../hooks/usePullRefresh"] = harness.load(
    fileURLToPath(new URL("../../hooks/usePullRefresh.ts", import.meta.url)),
  );
  const module = harness.load(
    fileURLToPath(new URL("./CollaborationGroupControls.tsx", import.meta.url)),
  ) as any;
  harness.mount(() =>
    module[component]({
      group: target,
      member: target.members[0],
      onOpenGroup: () => {},
      onSaved: () => {},
      onRemoved: () => {},
    }),
  );
  const find = (type: string, label: string) => {
    const result = elements(harness.flush()).find(
      (row) => row.type === type && row.props.label === label,
    );
    assert.ok(result, label);
    return result;
  };
  return {
    harness,
    session,
    find,
    alerts,
    changes,
    query,
    all: () => elements(harness.flush()),
    setActive: (active: boolean) => {
      session.active = active;
      session.readable = active;
      session.enabled = active;
      harness.render();
    },
  };
}

test("pasted invitation and preview are cleared across background and account boundaries", async () => {
  const env = setup("CollaborationEntry");
  env.find("ActionButton", "Join with invitation").props.onPress();
  env.find("Field", "Invitation link or token").props.onChangeText(TOKEN);
  env.find("ActionButton", "Preview invitation").props.onPress();
  await env.harness.settle();
  assert.ok(
    elements(env.harness.flush()).some(
      (row) => row.props.label === "Join group",
    ),
  );
  env.setActive(false);
  assert.equal(env.harness.flush(), null);
  env.setActive(true);
  assert.equal(env.find("Field", "Invitation link or token").props.value, "");
  assert.equal(
    elements(env.harness.flush()).some(
      (row) => row.props.label === "Join group",
    ),
    false,
  );
  env.find("Field", "Invitation link or token").props.onChangeText(TOKEN);
  env.session.epoch = 2;
  env.harness.render();
  assert.equal(env.find("Field", "Invitation link or token").props.value, "");
});

test("freshly issued invitation is hidden and cleared when the app backgrounds", async () => {
  const env = setup("CollaborationGroupControls");
  env.find("ActionButton", "Create invitation link").props.onPress();
  await env.harness.settle();
  assert.ok(
    elements(env.harness.flush()).some(
      (row) => row.props.label === "Copy invitation link",
    ),
  );
  env.setActive(false);
  assert.equal(
    elements(env.harness.flush()).some(
      (row) => row.props.label === "Copy invitation link",
    ),
    false,
  );
  env.setActive(true);
  assert.equal(
    elements(env.harness.flush()).some(
      (row) => row.props.label === "Copy invitation link",
    ),
    false,
  );
});

test("invalid invitation feedback is dismissed when cancelled, corrected or entering another mode", async () => {
  const env = setup("CollaborationEntry");
  const notices = () =>
    elements(env.harness.flush()).filter((row) => row.type === "Notice");
  env.find("ActionButton", "Join with invitation").props.onPress();
  env.find("Field", "Invitation link or token").props.onChangeText("invalid");
  env.find("ActionButton", "Preview invitation").props.onPress();
  await env.harness.settle();
  assert.equal(notices().length, 1);
  env.find("ActionButton", "Cancel").props.onPress();
  assert.equal(notices().length, 0);
  env.find("ActionButton", "Create multi-person group").props.onPress();
  assert.equal(notices().length, 0);
  assert.equal(env.find("ActionButton", "Create group").props.disabled, true);
  env.find("ActionButton", "Cancel").props.onPress();
  // Entry handlers also discard feedback from the prior mode/context.
  env.session.error = "Old create error";
  env.find("ActionButton", "Join with invitation").props.onPress();
  assert.equal(notices().length, 0);
  env.find("Field", "Invitation link or token").props.onChangeText("invalid");
  env.find("ActionButton", "Preview invitation").props.onPress();
  await env.harness.settle();
  assert.equal(notices().length, 1);
  env.find("Field", "Invitation link or token").props.onChangeText(TOKEN);
  assert.equal(notices().length, 0);
});

test("removing an agent from a group waits for an explicit confirmation", async () => {
  const env = setup("CollaborationGroupControls");
  env.find("ActionButton", "Remove agent research").props.onPress();
  await env.harness.settle();
  assert.equal(env.changes.length, 0);
  assert.equal(env.alerts.length, 1);
  env.alerts[0]!.buttons.find((b) => b.text === "Confirm")!.onPress!();
  await env.harness.settle();
  assert.deepEqual(JSON.parse(JSON.stringify(env.changes)), [
    { action: "remove_agent", agent_id: "agent-1" },
  ]);
});

test("participants can ask an agent's owner for access and cancel a pending request", async () => {
  const policy = {
    agent_id: "agent-1",
    enabled: false,
    revision: 1,
    can_enable: false,
    can_disable: false,
    can_request: true,
    lane: null,
  };
  const asking = groupSchema.parse({
    ...group,
    members: [{ ...group.members[0], owner_name: "Ada", unavailable: true }],
    collaboration: { ...group.collaboration, guest_access: [policy] },
  });
  const ask = setup("GroupAgentAccessRequest", asking);
  ask.find("ActionButton", "Ask Ada to allow participants").props.onPress();
  await ask.harness.settle();
  assert.deepEqual(JSON.parse(JSON.stringify(ask.changes)), [
    { ask: "agent-1" },
  ]);

  const pending = groupSchema.parse({
    ...asking,
    collaboration: {
      ...asking.collaboration,
      guest_access: [
        {
          ...policy,
          can_request: false,
          request: {
            id: "req",
            status: "pending",
            expires_at: "2099-01-01T00:00:00Z",
            can_cancel: true,
          },
        },
      ],
    },
  });
  const cancel = setup("GroupAgentAccessRequest", pending);
  cancel
    .find("ActionButton", "Cancel access request for research")
    .props.onPress();
  await cancel.harness.settle();
  assert.deepEqual(JSON.parse(JSON.stringify(cancel.changes)), [
    { cancel: "agent-1", request: "req" },
  ]);
});

test("group settings refresh spins only for a pull and first load shows a skeleton", async () => {
  const env = setup("CollaborationGroupControls");
  const page = () => env.all().find((row) => row.type === "WorkspacePage")!;
  env.query.fetching = true;
  env.harness.render();
  assert.equal(page().props.refreshing, false, "the 30s poll stays silent");
  const pending = deferred<void>();
  env.query.refetch = () => pending.promise;
  env.session.identity.refetch = async () => {};
  page().props.onRefresh();
  env.harness.flush();
  assert.equal(page().props.refreshing, true);
  pending.resolve();
  await env.harness.settle();
  assert.equal(page().props.refreshing, false);
  env.session.readable = false;
  env.harness.render();
  const skeleton = env.all().find((row) => row.type === "SkeletonRows");
  assert.equal(
    skeleton?.props.accessibilityLabel,
    "Loading current group rights",
  );
  env.query.error = true;
  env.harness.render();
  assert.equal(
    env.all().some((row) => row.type === "SkeletonRows"),
    false,
    "a failed load shows its error, not a loader",
  );
  env.harness.unmount();
});
