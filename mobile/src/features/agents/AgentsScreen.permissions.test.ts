import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { hookHarness } from "../assistant/assistantHookTestHarness";
import * as contracts from "./agentContracts";
import type { AgentPermissions } from "./AgentPermissionsField";

type Element = { type: unknown; props: Record<string, any> };
type ApiCall = { method: string; args: unknown[] };
const plain = <T>(value: T): T => JSON.parse(JSON.stringify(value));
const agent = contracts.agentSchema.parse({
  id: "agent-exact-7",
  kind: "specialist",
  name: "researcher",
  description: "Research documents",
  status: "idle",
  can_maintain: true,
  can_use: true,
  owner_kind: "person",
  services: ["github"],
  account_read: false,
  guest_access: { github: "read" },
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

// Execute only the permission form wrapper, preserving a stable hook order.
// The permissions field remains mocked at the native presentation boundary.
function renderPermissionForm(root: Element): Element {
  if (
    typeof root.type === "function" &&
    ["AgentGrantsForm", "CreateAgentForm"].includes(root.type.name)
  ) {
    return root.type(root.props);
  }
  const children = root.props.children;
  const renderChild = (child: unknown): unknown =>
    Array.isArray(child)
      ? child.map(renderChild)
      : child && typeof child === "object" && "props" in child
        ? renderPermissionForm(child as Element)
        : child;
  return { ...root, props: { ...root.props, children: renderChild(children) } };
}

function setup(creating = false, listing = false) {
  const calls: ApiCall[] = [];
  let latest = plain(agent);
  const client = {
    invalidateQueries: async () => {
      calls.push({ method: "invalidate", args: [] });
    },
  };
  const harness = hookHarness<Element>({
    "react/jsx-runtime": {
      jsx: (type: unknown, props: unknown) => ({ type, props }),
      jsxs: (type: unknown, props: unknown) => ({ type, props }),
      Fragment: "Fragment",
    },
    "react-native": {
      ...Object.fromEntries(
        ["ActivityIndicator", "Pressable", "Text", "TextInput", "View"].map(
          (name) => [name, name],
        ),
      ),
      AppState: {
        currentState: "active",
        addEventListener: () => ({ remove: () => {} }),
      },
      Keyboard: { dismiss: () => {} },
      StyleSheet: { create: (styles: unknown) => styles },
    },
    "@react-navigation/native": { useIsFocused: () => true },
    "@tanstack/react-query": {
      useQueryClient: () => client,
      useQuery: ({ queryKey }: { queryKey: string[] }) => ({
        isPending: false,
        isError: false,
        refetch: async () => {},
        data:
          queryKey[2] === "detail"
            ? { agent, memory: [], threads: [] }
            : queryKey[2] === "list"
              ? { agents: [agent], limits: { max_live_subagents: 10 } }
              : queryKey[2] === "connections"
                ? [
                    {
                      id: "connection-github",
                      slug: "github",
                      label: "GitHub",
                      is_active: true,
                    },
                    {
                      id: "connection-docs",
                      slug: "documents",
                      label: "Documents",
                      is_active: true,
                    },
                  ]
                : [],
      }),
    },
    "lucide-react-native": Object.fromEntries(
      ["Bot", "ChevronRight", "Search", "SlidersHorizontal", "Users", "X"].map(
        (name) => [name, name],
      ),
    ),
    "../../components/workspace/WorkspaceUI": Object.fromEntries(
      ["ActionButton", "Card", "Field", "Label", "Notice", "WorkspacePage"].map(
        (name) => [name, name],
      ),
    ),
    "../../lib/api/agentsApi": {
      agentsApi: {
        detail: async (id: string) => {
          calls.push({ method: "detail", args: [id] });
          return { agent: latest, memory: [], threads: [] };
        },
        grants: async (...args: unknown[]) => {
          calls.push({ method: "grants", args: plain(args) });
        },
        create: async (...args: unknown[]) => {
          calls.push({ method: "create", args: plain(args) });
          return { id: agent.id };
        },
      },
    },
    "../../lib/api/servicesApi": { servicesApi: {} },
    "../../app/WorkspaceHeader": { useWorkspaceBack: () => {} },
    "../../theme/designTokens": { fonts: {} },
    "../../theme/ThemeContext": {
      useTheme: () => ({ colors: { primaryTone: {} } }),
    },
    "./agentContracts": contracts,
    "./AgentModelField": { AgentModelField: "AgentModelField" },
    "./AgentPermissionsField": {
      AgentPermissionsField: "AgentPermissionsField",
    },
    "../../components/workspace/WorkspaceSelectionField": {
      WorkspaceSelectionSheet: "WorkspaceSelectionSheet",
    },
  });
  const { AgentsScreen } = harness.load(
    fileURLToPath(new URL("./AgentsScreen.tsx", import.meta.url)),
  ) as { AgentsScreen: (props: { agentId?: string }) => Element };
  harness.mount(() =>
    renderPermissionForm(
      AgentsScreen({ agentId: creating || listing ? undefined : agent.id }),
    ),
  );
  const all = () => elements(harness.value());
  const one = (predicate: (row: Element) => boolean) => {
    const rows = all().filter(predicate);
    assert.equal(rows.length, 1, "Expected one element");
    return rows[0];
  };
  const action = (label: string) =>
    one((row) => row.type === "ActionButton" && row.props.label === label);
  if (creating) {
    action("Create an agent").props.onPress();
    harness.flush();
  }
  return {
    harness,
    calls,
    one,
    action,
    field: () => one((row) => row.type === "AgentPermissionsField"),
    latest: (patch: Partial<contracts.AssistantAgent>) => {
      latest = { ...latest, ...patch };
    },
  };
}

test("the permissions form returns successful save and submits the exact changed grants", async () => {
  const state = setup();
  const permissions: AgentPermissions = {
    services: ["github", "documents"],
    account_read: true,
    guest_access: { github: "all", documents: "use" },
  };
  const result = await state.field().props.onSave(permissions);
  state.harness.flush();
  assert.equal(result, true);
  assert.deepEqual(state.calls, [
    { method: "detail", args: [agent.id] },
    {
      method: "grants",
      args: [
        agent.id,
        {
          services: ["github", "documents"],
          account_read: true,
          guest_access: { github: "all" },
        },
      ],
    },
    { method: "invalidate", args: [] },
  ]);
});

test("concurrent permission changes, revoked maintenance and destroyed state return false without writing grants", async () => {
  for (const patch of [
    { services: ["github", "other-access"] },
    { can_maintain: false },
    { status: "destroyed" as const },
  ]) {
    const state = setup();
    state.latest(patch);
    const result = await state
      .field()
      .props.onSave({ ...plain(agent), services: ["documents"] });
    state.harness.flush();
    assert.equal(result, false);
    assert.deepEqual(state.calls, [{ method: "detail", args: [agent.id] }]);
    assert.ok(state.field().props.saveError);
  }
});

test("creation permission confirmation updates only the local draft until Create agent", async () => {
  const state = setup(true);
  const result = await state.field().props.onSave({
    services: ["documents"],
    account_read: true,
    guest_access: {},
  });
  state.harness.flush();
  assert.equal(result, true);
  assert.deepEqual(state.calls, []);
  assert.deepEqual(plain(state.field().props.value), {
    services: ["documents"],
    account_read: true,
    guest_access: {},
  });
  state
    .one(
      (row) =>
        typeof row.type === "function" && row.type.name === "ProfileFields",
    )
    .props.onChange({ name: "researcher", description: "Review documents" });
  state.harness.flush();
  assert.equal(state.action("Create agent").props.disabled, false);
  state.action("Create agent").props.onPress();
  await state.harness.settle();
  const creates = state.calls.filter((call) => call.method === "create");
  assert.equal(creates.length, 1);
  assert.deepEqual(creates[0].args, [
    {
      name: "researcher",
      display_name: "",
      description: "Review documents",
      persona: "",
      services: ["documents"],
      account_read: true,
    },
  ]);
});

test("agent filters open in a sheet and apply only after confirmation", () => {
  const state = setup(false, true);
  const filter = () =>
    state.one((row) => row.props.accessibilityLabel === "Filter agents");
  const sheet = () =>
    state.one((row) => row.type === "WorkspaceSelectionSheet");
  filter().props.onPress();
  state.harness.flush();
  assert.deepEqual(plain(sheet().props.value), []);
  sheet().props.onClose();
  state.harness.flush();
  assert.equal(filter().props.accessibilityState.selected, false);
  filter().props.onPress();
  state.harness.flush();
  sheet().props.onChange(["destroyed"]);
  sheet().props.onClose();
  state.harness.flush();
  assert.equal(filter().props.accessibilityState.selected, true);
  filter().props.onPress();
  state.harness.flush();
  assert.deepEqual(plain(sheet().props.value), ["destroyed"]);
  assert.deepEqual(state.calls, []);
});
