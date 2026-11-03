import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { hookHarness } from "../assistant/assistantHookTestHarness";
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

function setup(component: "CollaborationEntry" | "CollaborationGroupControls") {
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
  session.clearError = () => {
    session.error = undefined;
  };
  const harness = hookHarness<Element>({
    "react/jsx-runtime": {
      jsx: (type: unknown, props: unknown) => ({ type, props }),
      jsxs: (type: unknown, props: unknown) => ({ type, props }),
    },
    "react-native": { Alert: {}, View: "View" },
    "expo-clipboard": { setStringAsync: async () => {} },
    "@tanstack/react-query": {
      useQuery: (options: any) => ({
        data: options.queryKey.includes("group")
          ? group
          : options.queryKey.includes("agents")
            ? { agents: [] }
            : [],
        isError: false,
        isFetching: false,
        dataUpdatedAt: 100,
        refetch: async () => {},
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
        change: async () => ({
          group,
          invite: { token: TOKEN, expires_at: "2099-11-03T01:00:00Z" },
        }),
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
  });
  const module = harness.load(
    fileURLToPath(new URL("./CollaborationGroupControls.tsx", import.meta.url)),
  ) as any;
  harness.mount(() =>
    module[component]({
      group,
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
