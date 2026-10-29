import assert from "node:assert/strict";
import { URL, fileURLToPath } from "node:url";
import test from "node:test";
import { hookHarness } from "../assistant/assistantHookTestHarness";
import { agentSchema } from "../agents/agentContracts";
import * as contracts from "./groupContracts";
import * as state from "./groupState";

const ORG = "1785c70a-9a5b-41f8-b73e-6ff523cdd98e";
const SELF = "2785c70a-9a5b-41f8-b73e-6ff523cdd98e";
const OTHER = "3785c70a-9a5b-41f8-b73e-6ff523cdd98e";
const MISSING = "4785c70a-9a5b-41f8-b73e-6ff523cdd98e";
const GROUP_ID = "nyxg-0123456789abcdef0123456789abcdef";
const plain = <T>(value: T): T => JSON.parse(JSON.stringify(value));
type Element = { type: unknown; props: Record<string, any> };
const agents = [
  { id: "personal", name: "nyxbot", kind: "nyxbot", owner_kind: "person" },
  {
    id: "org-lead",
    name: "lead",
    kind: "specialist",
    owner_kind: "org",
    owner_id: ORG,
  },
  {
    id: "org-other",
    name: "research",
    kind: "specialist",
    owner_kind: "org",
    owner_id: ORG,
  },
  {
    id: "other-org",
    name: "other-org",
    kind: "specialist",
    owner_kind: "org",
    owner_id: OTHER,
  },
].map((row) =>
  agentSchema.parse({
    ...row,
    description: `${row.name} description`,
    status: "idle",
  }),
);
const group = contracts.groupSchema.parse({
  id: GROUP_ID,
  name: "Research",
  owner: { type: "org", id: ORG, name: "Research org" },
  your_role: "creator",
  members: [{ id: "org-lead", name: "lead", kind: "specialist" }],
  lead_agent_id: "org-lead",
  participants: [
    { id: SELF, display_name: "Creator" },
    { id: MISSING, display_name: "Existing participant" },
  ],
  created_at: "2026-10-15T08:00:00Z",
});
function elements(root: Element): Element[] {
  return [
    root,
    ...[root.props.children]
      .flat(Infinity)
      .flatMap((child) =>
        child && typeof child === "object" && "props" in child
          ? elements(child)
          : [],
      ),
  ];
}
function setup({
  creating = false,
  role = "creator",
  missingAgent = false,
} = {}) {
  const calls: { method: string; args: unknown[] }[] = [];
  let latest = { ...plain(group), your_role: role };
  const original = { ...plain(group), your_role: role };
  if (missingAgent)
    original.members.push({
      id: "gone",
      name: "Old agent",
      kind: "specialist",
      destroyed: true,
      working: false,
    });
  let active = true;
  let participantError = false;
  let orgFeatureEnabled = true;
  let identityAvailable = true;
  let backOptions: {
    discardOnClose?: { title: string; message: string };
  } = {};
  const queryClient = {
    invalidateQueries: async () => {},
    removeQueries: () => {},
  };
  const harness = hookHarness<Element>({
    "../../components/AgentAvatar": { AgentAvatar: "AgentAvatar", AgentAvatarStack: "AgentAvatarStack" },
    "react/jsx-runtime": {
      jsx: (type: unknown, props: unknown) => ({ type, props }),
      jsxs: (type: unknown, props: unknown) => ({ type, props }),
      Fragment: "Fragment",
    },
    "react-native": {
      ActivityIndicator: "ActivityIndicator",
      Alert: { alert: () => {} },
    },
    "@tanstack/react-query": {
      useQueryClient: () => queryClient,
      useQuery: ({ queryKey }: { queryKey: unknown[] }) => {
        const key = queryKey.includes("group-picker") ? "agents" : queryKey[3];
        return {
          isPending: false,
          isError: key === "participants" && participantError,
          refetch: async () => {},
          data:
            key === "agents"
              ? { agents }
              : key === "detail"
                ? creating
                  ? undefined
                  : original
                : key === "organizations"
                  ? [
                      {
                        id: ORG,
                        slug: "research",
                        display_name: "Research org",
                        your_role: "admin",
                      },
                      {
                        id: OTHER,
                        slug: "other-org",
                        display_name: "Other org",
                        your_role: "admin",
                      },
                    ]
                  : key === "features"
                    ? orgFeatureEnabled
                      ? ["assistant:org-agents"]
                      : []
                    : key === "identity"
                      ? identityAvailable
                        ? { id: SELF }
                        : undefined
                      : [
                          {
                            user_id: SELF,
                            display_name: "Creator",
                            email: "creator@example.com",
                            role: "admin",
                            revoked_at: null,
                          },
                          {
                            user_id: OTHER,
                            display_name: "Researcher",
                            email: "researcher@example.com",
                            role: "member",
                            revoked_at: null,
                          },
                          {
                            user_id: "viewer",
                            display_name: "Viewer",
                            role: "viewer",
                            revoked_at: null,
                          },
                          {
                            user_id: "revoked",
                            display_name: "Revoked",
                            role: "member",
                            revoked_at: "2026-10-14T08:00:00Z",
                          },
                        ],
        };
      },
    },
    "../../components/workspace/WorkspaceUI": Object.fromEntries(
      ["ActionButton", "Card", "Field", "Label", "Notice", "WorkspacePage"].map(
        (name) => [name, name],
      ),
    ),
    "../../components/workspace/WorkspaceSelectionField": {
      WorkspaceSelectionField: "WorkspaceSelectionField",
    },
    "../../lib/api/agentsApi": { agentsApi: {} },
    "../../lib/api/groupsApi": {
      groupsApi: {
        detail: async (...args: unknown[]) => {
          calls.push({ method: "detail", args });
          return latest;
        },
        create: async (...args: unknown[]) => {
          calls.push({ method: "create", args: plain(args) });
          return group;
        },
        update: async (...args: unknown[]) => {
          calls.push({ method: "update", args: plain(args) });
          return group;
        },
      },
    },
    "../../lib/auth/authEpoch": { getAuthEpoch: () => 7 },
    "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
    "../../app/WorkspaceHeader": {
      useWorkspaceBack: (options: typeof backOptions) => {
        backOptions = options;
      },
    },
    "./groupContracts": contracts,
    "./groupState": state,
    "./useGroupRuntime": {
      groupKeys: (epoch: number) => ["workspace", "groups", epoch],
      groupError: (cause: Error) => cause.message,
      useGroupAvailability: () => ({
        epoch: 7,
        active,
        available: () => active,
      }),
    },
  });
  const { GroupEditorScreen } = harness.load(
    fileURLToPath(new URL("./GroupEditorScreen.tsx", import.meta.url)),
  ) as { GroupEditorScreen: (props: unknown) => Element };
  harness.mount(() => {
    const wrapper = GroupEditorScreen({
      groupId: creating ? undefined : GROUP_ID,
      onBack: () => {},
      onSaved: () => {},
    });
    assert.equal(typeof wrapper.type, "function");
    return (wrapper.type as (props: unknown) => Element)(wrapper.props);
  });
  const find = (type: string, label: string) => {
    const row = elements(harness.flush()).find(
      (item) => item.type === type && item.props.label === label,
    );
    assert.ok(row, `Expected ${type}: ${label}`);
    return row.props;
  };
  return {
    harness,
    calls,
    closeGuard: () => {
      harness.flush();
      return backOptions.discardOnClose;
    },
    field: (label: string) => find("WorkspaceSelectionField", label),
    button: (label: string) => find("ActionButton", label),
    name: (value: string) => find("Field", "Name").onChangeText(value),
    hasButton: (label: string) =>
      elements(harness.flush()).some(
        (item) => item.type === "ActionButton" && item.props.label === label,
      ),
    notices: () =>
      elements(harness.flush())
        .filter((item) => item.type === "Notice")
        .map((item) => item.props.message)
        .join("\n"),
    latest: (patch: Record<string, unknown>) => {
      latest = { ...latest, ...patch };
    },
    offline: () => {
      active = false;
      harness.render();
    },
    failParticipants: () => {
      participantError = true;
      harness.render();
    },
    ownershipAvailability: (feature: boolean, identity: boolean) => {
      orgFeatureEnabled = feature;
      identityAvailable = identity;
      harness.render();
    },
  };
}

test("group choices use compact searchable fields with member and participant limits", () => {
  const env = setup();
  const members = env.field("Members");
  assert.equal(members.multiple, true);
  assert.equal(members.maxSelected, 8);
  assert.deepEqual(plain(members.options.map((row: any) => row.value)), [
    "org-lead",
    "org-other",
  ]);
  const people = env.field("Participants");
  assert.equal(people.maxSelected, 16);
  assert.deepEqual(plain(people.options.map((row: any) => row.value)), [
    SELF,
    OTHER,
    MISSING,
  ]);
  const missing = people.options.find((row: any) => row.value === MISSING);
  assert.equal(missing.disabled, false);
  assert.equal(missing.label, "Existing participant");
  assert.ok(people.value.includes(MISSING));
  assert.match(missing.description, /Remove this participant before saving/);
  assert.deepEqual(
    plain(env.field("Lead agent").options.map((row: any) => row.value)),
    ["org-lead"],
  );
});

test("applying member, lead, and participant sheets only edits draft until Save changes", async () => {
  const env = setup();
  env.field("Members").onChange(["org-lead", "org-other"]);
  env.field("Lead agent").onChange(["org-other"]);
  env.field("Participants").onChange([SELF, OTHER, MISSING]);
  assert.ok(env.field("Participants").value.includes(MISSING));
  env.field("Participants").onChange([SELF, OTHER]);
  assert.deepEqual(plain(env.field("Participants").value), [SELF, OTHER]);
  assert.deepEqual(env.calls, []);
  env.button("Save changes").onPress();
  await env.harness.settle();
  assert.deepEqual(env.calls, [
    { method: "detail", args: [GROUP_ID] },
    {
      method: "update",
      args: [
        GROUP_ID,
        {
          member_agent_ids: ["org-lead", "org-other"],
          participant_user_ids: [SELF, OTHER],
          lead_agent_id: "org-other",
        },
      ],
    },
  ]);
});

test("ownership Apply resets dependent drafts and pins creator for a new organization group", async () => {
  const env = setup({ creating: true });
  assert.deepEqual(plain(env.field("Members").value), ["personal"]);
  env.field("Ownership").onChange([ORG]);
  assert.deepEqual(plain(env.field("Members").value), []);
  assert.deepEqual(plain(env.field("Participants").value), [SELF]);
  assert.equal(
    env.field("Participants").options.find((row: any) => row.value === SELF)
      .disabled,
    true,
  );
  assert.deepEqual(env.calls, []);
  env.name("New research group");
  env.field("Members").onChange(["org-lead"]);
  env.button("Create crew").onPress();
  await env.harness.settle();
  assert.deepEqual(env.calls, [
    {
      method: "create",
      args: [
        {
          name: "New research group",
          org: ORG,
          member_agent_ids: ["org-lead"],
          participant_user_ids: [SELF],
        },
      ],
    },
  ]);
});

test("an organization draft can return to Personal after organization eligibility becomes unavailable", () => {
  for (const [feature, identity] of [
    [false, true],
    [true, false],
  ]) {
    const env = setup({ creating: true });
    env.field("Ownership").onChange([ORG]);
    env.ownershipAvailability(feature!, identity!);
    const ownership = env.field("Ownership");
    assert.deepEqual(plain(ownership.value), [ORG]);
    assert.equal(
      ownership.options.find((row: any) => row.value === ORG).disabled,
      false,
    );
    assert.equal(
      ownership.options.find((row: any) => row.value === OTHER).disabled,
      true,
    );
    ownership.onChange(["personal"]);
    assert.deepEqual(plain(env.field("Ownership").value), ["personal"]);
    assert.deepEqual(plain(env.field("Members").value), ["personal"]);
    assert.equal(
      env.field("Ownership").options.find((row: any) => row.value === ORG)
        .disabled,
      true,
    );
    assert.deepEqual(env.calls, []);
  }
});

test("removed lead requires another selected agent before any group update", async () => {
  const env = setup();
  env.field("Members").onChange(["org-other"]);
  assert.deepEqual(plain(env.field("Lead agent").value), []);
  assert.deepEqual(
    plain(env.field("Lead agent").options.map((row: any) => row.value)),
    ["org-other"],
  );
  env.button("Save changes").onPress();
  await env.harness.settle();
  assert.deepEqual(env.calls, []);
  assert.match(env.notices(), /Choose a lead/);
});

test("group selection preserves live conflict checks and the local draft", async () => {
  const env = setup();
  env.field("Members").onChange(["org-lead", "org-other"]);
  env.latest({
    members: [
      ...group.members,
      { id: "external-agent", name: "external", kind: "specialist" },
    ],
  });
  env.button("Save changes").onPress();
  await env.harness.settle();
  assert.deepEqual(
    env.calls.map((call) => call.method),
    ["detail"],
  );
  assert.match(env.notices(), /changed elsewhere/);
  assert.deepEqual(plain(env.field("Members").value), [
    "org-lead",
    "org-other",
  ]);
});

test("read-only members can inspect selections and revoked management cannot save", async () => {
  const viewer = setup({ role: "participant" });
  assert.equal(viewer.field("Members").readOnly, true);
  assert.equal(viewer.field("Participants").readOnly, true);
  assert.equal(viewer.hasButton("Save changes"), false);
  const env = setup();
  env.field("Members").onChange(["org-lead", "org-other"]);
  env.latest({ your_role: "participant" });
  env.button("Save changes").onPress();
  await env.harness.settle();
  assert.deepEqual(
    env.calls.map((call) => call.method),
    ["detail"],
  );
  assert.match(env.notices(), /no longer manage/);
});

test("offline or failed participant queries disable corresponding sheet changes", () => {
  const env = setup();
  env.failParticipants();
  assert.equal(env.field("Participants").disabled, true);
  env.offline();
  assert.equal(env.field("Members").disabled, true);
  assert.equal(env.field("Lead agent").disabled, true);
  assert.equal(env.button("Save changes").disabled, true);
});

test("drawer dismissal asks before dropping a group draft and clears after reverting", () => {
  const env = setup();
  assert.equal(env.closeGuard(), undefined);
  env.name("Unsaved research group");
  assert.deepEqual(plain(env.closeGuard()), {
    title: "Discard crew changes?",
    message: "Your changes have not been saved.",
  });
  env.name(group.name);
  assert.equal(env.closeGuard(), undefined);
});

test("drawer dismissal no longer treats a saved group as an unsaved draft", async () => {
  const env = setup();
  env.field("Members").onChange(["org-lead", "org-other"]);
  assert.ok(env.closeGuard());
  env.button("Save changes").onPress();
  await env.harness.settle();
  assert.equal(env.closeGuard(), undefined);
});
