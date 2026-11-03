import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { hookHarness } from "../assistant/assistantHookTestHarness";
import * as contracts from "./machineContracts";
import type { MachineNode, SavedLogin } from "./machineContracts";

type Element = { type: unknown; props: Record<string, any> };
const jsx = {
  Fragment: "Fragment",
  jsx: (type: unknown, props: unknown) => ({ type, props }),
  jsxs: (type: unknown, props: unknown) => ({ type, props }),
};
const elements = (node: any): Element[] =>
  node?.props
    ? [node, ...[node.props.children].flat(Infinity).flatMap(elements)]
    : [];
const text = (node: any): string =>
  typeof node === "string" || typeof node === "number"
    ? String(node)
    : node?.props
      ? [
          node.props.label,
          node.props.detail,
          node.props.value,
          node.props.message,
          node.props.footer,
          node.props.children,
        ]
          .flat(Infinity)
          .map(text)
          .join(" ")
      : "";
// Values built inside the vm harness carry another realm's prototypes.
const plain = (value: unknown) => JSON.parse(JSON.stringify(value));
const file = (name: string) =>
  fileURLToPath(new URL(`./${name}`, import.meta.url));
const ui = {
  "react/jsx-runtime": jsx,
  "../../components/settings/SettingsList": {
    SettingsGroup: "Group",
    SettingsRow: "Row",
    SettingsCheckRow: "CheckRow",
    SettingsMenuRow: "MenuRow",
  },
  "../../components/workspace/WorkspaceUI": {
    Label: "Label",
    Notice: "Notice",
    WorkspacePage: "Page",
  },
};
function render<P>(
  name: string,
  exported: string,
  props: P,
  mocks: Record<string, unknown>,
) {
  const harness = hookHarness<Element>({ ...ui, ...mocks });
  const Component = harness.load(file(name))[exported] as (p: P) => Element;
  harness.mount(() => Component(props));
  const all = () => elements(harness.value());
  const row = (label: string) => {
    const found = all().find(
      (node) =>
        node.props.label === label || node.props.accessibilityLabel === label,
    );
    assert.ok(found, label);
    return found;
  };
  return { harness, all, row, text: () => text(harness.value()) };
}

const node = {
  id: "node",
  name: "Studio Mac",
  owner: { id: "user", kind: "user", display_name: "Calvin" },
  status: "online",
  is_connected: true,
  machine: {
    version: 2,
    shell: true,
    files: true,
    computer: false,
    browser: true,
    os: "macos",
    arch: "arm64",
    roots: ["/Users/calvin/work"],
    commands_isolated: false,
    browser_isolated: false,
    computer_ready: false,
    saved_login_ready: true,
  },
  machine_confirm: "none",
  allow_single_user_saved_logins: false,
  metadata: { agent_version: "0.41.1" },
} satisfies MachineNode;

function policyEditor(overrides: Partial<MachineNode> = {}) {
  const saved: any[] = [];
  const x = render(
    "MachinePolicyScreen.tsx",
    "PolicyEditor",
    {
      node: { ...node, ...overrides },
      update: undefined,
      onBack: () => {},
      onRefresh: async () => node,
    },
    {
      "@tanstack/react-query": {
        useQuery: () => ({}),
        useQueryClient: () => ({ invalidateQueries: async () => {} }),
      },
      "../../app/WorkspaceHeader": { useWorkspaceBack: () => {} },
      "../../lib/api/machinesApi": {
        machinesApi: {
          policy: async (_node: unknown, body: unknown) => saved.push(body),
        },
      },
      "./MachineUI": {
        confirmDiscard: (_dirty: boolean, leave: () => void) => leave(),
        machineError: String,
        machineKeys: (epoch: number) => ["machines", epoch],
        useMachineRuntime: () => ({ active: true }),
        useMachineWrite: () => ({
          busy: false,
          uncertain: false,
          active: true,
          epoch: 1,
          current: () => true,
          reviewed: () => {},
          run: (action: (current: () => boolean) => Promise<void>) =>
            action(() => true),
        }),
      },
      "./MachinePresentation": {
        MachineSwitch: "Switch",
        machinePageContent: {},
      },
      "./machineContracts": contracts,
    },
  );
  return { ...x, saved };
}

test("owner confirmation is one menu row; Save is a tinted row gated on a real change", async () => {
  const x = policyEditor();
  const menuRows = x.all().filter((n) => n.type === "MenuRow");
  assert.equal(menuRows.length, 1);
  const menu = () => x.all().find((n) => n.type === "MenuRow")!;
  assert.equal(menu().props.label, "Machine confirmation");
  assert.equal(menu().props.value, "none");
  assert.deepEqual(plain(menu().props.options.map((o: any) => o.label)), [
    "No machine confirmation",
    "Confirm changes",
    "Confirm every operation",
  ]);
  assert.equal(
    x.all().some((n) => /Selected:/.test(String(n.props.label))),
    false,
  );
  const save = () => x.row("Save owner policy");
  assert.equal(save().props.tone, "action");
  assert.equal(save().props.disabled, true);
  menu().props.onChange("all");
  x.harness.flush();
  assert.equal(menu().props.value, "all");
  assert.equal(save().props.disabled, false);
  await save().props.onPress();
  await x.harness.settle();
  assert.deepEqual(plain(x.saved), [
    {
      machine_confirm: "all",
      allow_single_user_saved_logins: false,
      acknowledge_single_user_risk: false,
    },
  ]);
});

test("single-user saved-login typing still requires the explicit risk acknowledgement", () => {
  const x = policyEditor();
  const toggle = x
    .all()
    .find(
      (n) =>
        n.props.accessibilityLabel ===
        "Allow saved-login typing on a single-user browser",
    )!;
  toggle.props.onChange(true);
  x.harness.flush();
  assert.match(x.text(), /Commands run as the browser user/);
  assert.equal(x.row("Save owner policy").props.disabled, true);
  x.row(
    "I acknowledge the risk of typed saved-login values being read",
  ).props.onChange(true);
  x.harness.flush();
  assert.equal(x.row("Save owner policy").props.disabled, false);
});

const login = {
  id: "login",
  owner_id: "user",
  label: "GitHub",
  allowed_origins: ["https://github.com"],
  username_hint: "c***@example.com",
  has_password: true,
  has_totp: true,
  confirm_each_sign_in: true,
  created_at: "2026-10-01T00:00:00Z",
  updated_at: "2026-10-02T00:00:00Z",
  last_used_at: null,
} satisfies SavedLogin;

function savedLogin(row: Record<string, unknown>) {
  let alert: any[] | undefined;
  const removed: string[] = [];
  const x = render(
    "SavedLogins.tsx",
    "SavedLoginScreen",
    {
      route: {
        params: { loginId: "login", ownerLabel: "Your personal account" },
      },
      navigation: { goBack: () => {} },
    },
    {
      "react-native": { Alert: { alert: (...args: any[]) => (alert = args) } },
      "@tanstack/react-query": {
        useQuery: () => ({ data: [row], isPending: false }),
        useQueryClient: () => ({ invalidateQueries: async () => {} }),
      },
      "../../app/WorkspaceHeader": { useWorkspaceBack: () => {} },
      "../../components/SegmentControl": { SegmentControl: "Segments" },
      "../../lib/api/machinesApi": {
        machinesApi: {
          removeLogin: async (target: SavedLogin) => removed.push(target.id),
        },
      },
      "./MachineUI": {
        machineError: String,
        machineKeys: (epoch: number) => ["machines", epoch],
        useMachineRuntime: () => ({ active: true, current: () => true }),
        useMachineWrite: () => ({
          busy: false,
          uncertain: false,
          active: true,
          epoch: 1,
          current: () => true,
          reviewed: () => {},
          run: (action: (current: () => boolean) => Promise<void>) =>
            action(() => true),
        }),
      },
      "./MachinePresentation": {
        formatMachineDate: (value?: string | null) => value ?? "Never",
        machinePageContent: {},
        machineTabs: [],
      },
      "./machineContracts": contracts,
    },
  );
  return { ...x, removed, alert: () => alert };
}

test("saved-login detail shows metadata only and never a secret value", () => {
  // Even if a server ever returned secret material, the page has no field for it.
  const x = savedLogin({
    ...login,
    password: "hunter2",
    totp_secret: "JBSWY3DP",
  });
  const shown = x.text();
  assert.doesNotMatch(shown, /hunter2|JBSWY3DP/);
  assert.equal(x.row("Password").props.value, "Stored");
  assert.equal(x.row("Authenticator (TOTP)").props.value, "Stored");
  assert.equal(x.row("Owner").props.value, "Your personal account");
  assert.ok(x.row("https://github.com").props.mono);
});

test("saved-login deletion is a solid red row that deletes only after the destructive confirmation", async () => {
  const x = savedLogin(login);
  const remove = x.row("Review deletion of GitHub");
  assert.equal(remove.props.destructive, true);
  assert.equal(remove.props.centered, true);
  remove.props.onPress();
  const [title, message, buttons] = x.alert()!;
  assert.match(title, /Delete GitHub from Your personal account/);
  assert.match(message, /cannot be undone/);
  assert.deepEqual(x.removed, []);
  assert.equal(buttons[0].style, "cancel");
  buttons[0].onPress?.();
  assert.deepEqual(x.removed, []);
  assert.equal(buttons[1].style, "destructive");
  buttons[1].onPress();
  await x.harness.settle();
  assert.deepEqual(x.removed, ["login"]);
  assert.match(x.text(), /Saved login deleted/);
});

test("activity agent filter is a checkmark list that returns its choice to the activity page", () => {
  const moves: unknown[][] = [];
  const x = render(
    "MachineActivityScreen.tsx",
    "MachineActivityFilterScreen",
    {
      route: {
        params: {
          machineId: "node",
          ownerId: "user",
          agent: "",
          agents: [{ id: "agent-1", label: "Researcher" }],
        },
      },
      navigation: { popTo: (...args: unknown[]) => moves.push(args) },
    },
    {
      "@tanstack/react-query": { useQuery: () => ({}) },
      "../../lib/api/machinesApi": { machinesApi: {} },
      "./MachineUI": {},
      "./MachinePresentation": { machinePageContent: {} },
    },
  );
  const checks = x.all().filter((n) => n.type === "CheckRow");
  assert.deepEqual(
    plain(checks.map((n) => [n.props.label, n.props.selected])),
    [
      ["All agents", true],
      ["Unattributed", false],
      ["Researcher", false],
    ],
  );
  checks[2]!.props.onPress();
  assert.deepEqual(plain(moves), [
    [
      "MachineActivity",
      { machineId: "node", ownerId: "user", agent: "agent-1" },
      { merge: true },
    ],
  ]);
});

test("machine detail is label/value groups with policy, activity and desktop as rows", () => {
  const opened: string[] = [];
  const x = render(
    "MachinesScreen.tsx",
    "MachineDetails",
    {
      node,
      update: undefined,
      onPolicy: () => opened.push("policy"),
      onActivity: () => opened.push("activity"),
      onDesktop: () => opened.push("desktop"),
    },
    {
      "@tanstack/react-query": { useQuery: () => ({}) },
      "../../app/WorkspaceHeader": { useWorkspaceBack: () => {} },
      "../../components/SegmentControl": { SegmentControl: "Segments" },
      "./SavedLogins": { SavedLogins: "SavedLogins" },
      "./MachineDesktopEntries": { MachineDesktopEntries: "Desktop" },
      "./MachinePresentation": {
        machineConnectivity: (n: MachineNode) =>
          n.is_connected ? "Online" : "Offline",
        machineIsolationLabel: () => "Not isolated",
        machinePageContent: {},
        machineTabs: [],
      },
      "../../lib/api/machinesApi": { machinesApi: {} },
      "./machineContracts": contracts,
      "./MachineUI": {},
    },
  );
  assert.equal(x.row("Status").props.value, "Online");
  assert.equal(x.row("Platform").props.value, "macos/arm64");
  assert.equal(x.row("Commands").props.value, "Available");
  assert.equal(x.row("Computer").props.value, "Off");
  assert.equal(x.row("Command isolation").props.valueTone, "danger");
  assert.match(
    x.text(),
    /agent commands can read this node's stored credentials/,
  );
  assert.equal(
    x.all().some((n) => n.type === "Card"),
    false,
  );
  x.row("Owner Policy and Updates").props.onPress();
  x.row("Machine Activity").props.onPress();
  assert.deepEqual(opened, ["policy", "activity"]);
  assert.ok(x.all().some((n) => n.type === "Desktop"));
});
