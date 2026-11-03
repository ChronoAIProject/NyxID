import assert from "node:assert/strict";
import { URL, fileURLToPath } from "node:url";
import test from "node:test";
import { deferred, hookHarness } from "../assistant/assistantHookTestHarness";
import * as contracts from "./agentContracts";

type Element = { type: unknown; props: Record<string, any> };
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

function setup(
  field: "name" | "display_name" | "description" | "persona",
  options: { agent?: Partial<contracts.AssistantAgent>; online?: boolean } = {},
) {
  const calls: { method: string; args: unknown[] }[] = [];
  const actions: string[] = [];
  let guard: Record<string, any> = {};
  let loaded = { ...plain(agent), ...options.agent };
  let latest = loaded;
  const update = deferred<void>();
  const harness = hookHarness<Element>({
    "react/jsx-runtime": {
      jsx: (type: unknown, props: unknown) => ({ type, props }),
      jsxs: (type: unknown, props: unknown) => ({ type, props }),
      Fragment: "Fragment",
    },
    "@react-navigation/native": { useIsFocused: () => true },
    "@tanstack/react-query": {
      useQueryClient: () => ({
        invalidateQueries: async () => {
          calls.push({ method: "invalidate", args: [] });
        },
      }),
      useQuery: () => ({
        isPending: false,
        isError: false,
        data: { agent: loaded, memory: [], threads: [] },
      }),
    },
    "../../app/WorkspaceHeader": {
      useWorkspaceBack: (value: Record<string, any>) => {
        guard = value;
      },
    },
    "../../components/settings/SettingsList": Object.fromEntries(
      ["SettingsGroup", "SettingsPage", "SettingsTextField"].map((name) => [
        name,
        name,
      ]),
    ),
    "../../components/settings/SettingsHeaderButton": {
      SettingsHeaderButton: "SettingsHeaderButton",
    },
    "../../components/workspace/WorkspaceUI": {
      Label: "Label",
      Notice: "Notice",
    },
    "../../hooks/useNetworkStatus": {
      useNetworkStatus: () => ({ isConnected: options.online !== false }),
    },
    "../../lib/api/agentsApi": {
      agentsApi: {
        detail: async (id: string) => {
          calls.push({ method: "detail", args: [id] });
          return { agent: latest };
        },
        update: (...args: unknown[]) => {
          calls.push({ method: "update", args: plain(args) });
          return update.promise;
        },
      },
    },
    "./agentContracts": contracts,
  });
  const { AgentProfileEditScreen } = harness.load(
    fileURLToPath(new URL("./AgentProfileEditScreen.tsx", import.meta.url)),
  ) as { AgentProfileEditScreen: (props: unknown) => Element };
  harness.mount(() =>
    AgentProfileEditScreen({
      navigation: { goBack: () => actions.push("back") },
      route: { params: { agentId: agent.id, field } },
    }),
  );
  const all = () => elements(harness.flush());
  const input = () => all().find((row) => row.type === "SettingsTextField")!;
  return {
    harness,
    calls,
    actions,
    update,
    guard: () => guard,
    input,
    save: () => guard.options.headerRight(),
    notices: () =>
      all()
        .filter((row) => row.type === "Notice")
        .map((row) => row.props.message)
        .join("\n"),
    type: (text: string) => {
      input().props.onChangeText(text);
      harness.flush();
    },
    changeElsewhere: (patch: Partial<contracts.AssistantAgent>) => {
      latest = { ...latest, ...patch };
    },
    reload: (patch: Partial<contracts.AssistantAgent>) => {
      loaded = { ...loaded, ...patch };
      harness.render();
    },
  };
}

test("one profile value is edited on its own page with Save in the header", async () => {
  const page = setup("display_name");
  assert.equal(page.guard().options.title, "Display name");
  assert.equal(page.input().props.value, "");
  assert.equal(page.input().props.autoFocus, true);
  assert.equal(page.save().props.enabled, false);
  assert.equal(page.guard().discardOnClose, undefined);
  page.type("Research lead");
  assert.equal(page.guard().discardOnClose.title, "Discard changes?");
  const save = page.save();
  save.props.onPress();
  save.props.onPress();
  page.harness.flush();
  assert.equal(page.save().props.busy, true);
  assert.equal(page.guard().disabled, true);
  page.update.resolve();
  await page.harness.settle();
  assert.deepEqual(page.calls, [
    { method: "detail", args: [agent.id] },
    {
      method: "update",
      args: [agent.id, { display_name: "Research lead" }],
    },
    { method: "invalidate", args: [] },
  ]);
  assert.deepEqual(page.actions, ["back"]);
  assert.equal(page.guard().discardOnClose, undefined);
});

test("a change made elsewhere since the page opened is refused, keeping the draft", async () => {
  const page = setup("description");
  page.reload({ description: "Server refreshed" });
  page.type("Review contracts");
  page.changeElsewhere({ description: "Edited elsewhere" });
  page.save().props.onPress();
  await page.harness.settle();
  assert.deepEqual(page.calls, [{ method: "detail", args: [agent.id] }]);
  assert.match(page.notices(), /changed elsewhere/);
  assert.equal(page.input().props.value, "Review contracts");
  assert.deepEqual(page.actions, []);
});

test("handles are lowercased and validated before any request", async () => {
  const page = setup("name");
  page.type("Bad Handle!");
  assert.equal(page.input().props.value, "bad handle!");
  page.save().props.onPress();
  await page.harness.settle();
  assert.deepEqual(page.calls, []);
  assert.match(page.notices(), /lowercase letters/);
});

test("read-only, NyxBot handle and offline states cannot save", () => {
  const nyxbot = setup("name", { agent: { kind: "nyxbot", name: "nyxbot" } });
  assert.equal(nyxbot.input().props.editable, false);
  assert.match(nyxbot.notices(), /handle cannot be changed/);
  assert.equal(
    setup("description", { agent: { kind: "nyxbot" } }).guard().options.title,
    "Notes",
  );
  const viewer = setup("persona", { agent: { can_maintain: false } });
  assert.equal(viewer.input().props.editable, false);
  const offline = setup("persona", { online: false });
  offline.type("Warm");
  assert.equal(offline.save().props.enabled, false);
  offline.save().props.onPress();
  assert.deepEqual(offline.calls, []);
});
