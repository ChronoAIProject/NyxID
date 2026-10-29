import assert from "node:assert/strict";
import { URL, fileURLToPath } from "node:url";
import test from "node:test";
import { hookHarness } from "../../features/assistant/assistantHookTestHarness";
import type { WorkspaceSelectionOption } from "./WorkspaceSelectionField";

type Element = { type: unknown; props: Record<string, any> };
const plain = (value: unknown) => JSON.parse(JSON.stringify(value));
const options: WorkspaceSelectionOption[] = Array.from(
  { length: 9 },
  (_, i) => ({
    value: `id-${i}`,
    label: `Option ${i}`,
    description: i === 3 ? "Research documents" : undefined,
    searchText: i === 3 ? "doc-provider" : undefined,
  }),
);
function elements(root: Element): Element[] {
  const children = [root.props.children].flat(Infinity);
  if (root.type === "FlatList") {
    children.push(root.props.ListHeaderComponent);
    children.push(
      ...root.props.data.map((item: unknown) =>
        root.props.renderItem({ item }),
      ),
    );
  }
  return [
    root,
    ...children.flatMap((child) =>
      child && typeof child === "object" && "props" in child
        ? elements(child)
        : [],
    ),
  ];
}
function setup(overrides: Record<string, any> = {}, field = false) {
  const writes: string[][] = [];
  let closed = 0;
  const props: Record<string, any> = {
    label: "Members",
    options: plain(options),
    value: ["id-0"],
    multiple: true,
    onChange: (value: string[]) => writes.push(plain(value)),
    onClose: () => closed++,
    ...overrides,
  };
  const harness = hookHarness<Element>({
    "./useNativeSheetDismiss": {
      useNativeSheetDismiss: (onClose: () => void) => ({
        modalProps: { visible: true, animationType: "slide" },
        close: (after?: () => void) => {
          after?.();
          onClose();
        },
      }),
    },
    "react/jsx-runtime": {
      jsx: (type: unknown, props: unknown) => ({ type, props }),
      jsxs: (type: unknown, props: unknown) => ({ type, props }),
    },
    "react-native": {
      ...Object.fromEntries(
        [
          "FlatList",
          "KeyboardAvoidingView",
          "Modal",
          "Pressable",
          "TextInput",
          "View",
        ].map((name) => [name, name]),
      ),
      Keyboard: { dismiss: () => {} },
      Platform: { OS: "ios" },
      StyleSheet: { create: (styles: unknown) => styles },
    },
    "react-native-safe-area-context": {
      SafeAreaProvider: "SafeAreaProvider",
      SafeAreaView: "SafeAreaView",
    },
    "lucide-react-native": Object.fromEntries(
      ["Check", "ChevronDown", "Search", "X"].map((name) => [name, name]),
    ),
    "../../theme/designTokens": { fonts: {} },
    "../../theme/ThemeContext": {
      useTheme: () => ({ colors: { primaryTone: {} } }),
    },
    "./WorkspaceUI": {
      Label: "Label",
      Notice: "Notice",
      ActionButton: "ActionButton",
    },
  });
  const exports = harness.load(
    fileURLToPath(new URL("./WorkspaceSelectionField.tsx", import.meta.url)),
  ) as {
    WorkspaceSelectionField: (props: any) => Element;
    WorkspaceSelectionSheet: (props: any) => Element;
  };
  harness.mount(() =>
    (field ? exports.WorkspaceSelectionField : exports.WorkspaceSelectionSheet)(
      props,
    ),
  );
  const all = () => elements(harness.flush());
  const one = (predicate: (row: Element) => boolean) => {
    const item = all().find(predicate);
    assert.ok(item, "Expected matching control");
    return item;
  };
  const label = (name: string) =>
    one((row) => row.props.accessibilityLabel === name);
  const apply = () => one((row) => row.type === "ActionButton");
  const rowIds = () =>
    plain(
      one((row) => row.type === "FlatList").props.data.map(
        (row: any) => row.value,
      ),
    );
  return {
    props,
    harness,
    writes,
    all,
    one,
    label,
    apply,
    rowIds,
    exports,
    closed: () => closed,
    select: (name: string) => {
      label(name).props.onPress();
      harness.flush();
    },
    search: (value: string) => {
      label("Search Members").props.onChangeText(value);
      harness.flush();
    },
  };
}

test("summary stays bounded and read-only users can inspect the full selector", () => {
  const state = setup(
    { value: ["id-0", "id-1", "id-2", "id-3"], disabled: true, readOnly: true },
    true,
  );
  const trigger = state.label("Members: Option 0, Option 1 +2 more");
  assert.equal(trigger.props.disabled, false);
  trigger.props.onPress();
  state.harness.flush();
  assert.ok(
    state
      .all()
      .some((row) => row.type === state.exports.WorkspaceSelectionSheet),
  );
  assert.equal(state.writes.length, 0);
});

test("search by name, metadata and exact IDs keeps hidden selections", () => {
  const state = setup();
  for (const search of ["research", "doc-provider", "id-3"]) {
    state.search(search);
    assert.deepEqual(state.rowIds(), ["id-3"]);
  }
  state.select("Option 3");
  state.search("Option 8");
  state.select("Option 8");
  state.search("");
  state.select("Selected options");
  assert.deepEqual(state.rowIds(), ["id-0", "id-3", "id-8"]);
  assert.equal(state.writes.length, 0);
  state.apply().props.onPress();
  assert.deepEqual(state.writes, [["id-0", "id-3", "id-8"]]);
  assert.equal(state.closed(), 1);
});

test("cancel and native dismissal discard the draft without updating the parent", () => {
  for (const native of [false, true]) {
    const state = setup();
    state.select("Option 1");
    if (native) state.one((row) => row.type === "Modal").props.onRequestClose();
    else state.label("Cancel Members").props.onPress();
    assert.equal(state.closed(), 1);
    assert.deepEqual(state.writes, []);
    assert.deepEqual(state.props.value, ["id-0"]);
  }
});

test("locked choices survive clearing and maximum selection is enforced within one tick", () => {
  const state = setup({
    options: options.map((row, i) => ({ ...row, disabled: i === 0 })),
    maxSelected: 2,
  });
  state.label("Option 0").props.onPress();
  const first = state.label("Option 1").props.onPress;
  const second = state.label("Option 2").props.onPress;
  first();
  second();
  state.harness.flush();
  assert.equal(state.label("Option 2").props.accessibilityState.checked, false);
  assert.equal(state.label("Option 2").props.disabled, true);
  state.label("Clear optional choices").props.onPress();
  state.harness.flush();
  assert.equal(state.label("Option 0").props.accessibilityState.checked, true);
  assert.equal(state.label("Option 1").props.accessibilityState.checked, false);
  assert.equal(state.apply().props.disabled, true);
});

test("single choice is temporary until Apply and duplicate confirmation is ignored", () => {
  const state = setup({ multiple: false, allowEmpty: false });
  assert.equal(state.apply().props.disabled, true);
  state.select("Option 2");
  assert.equal(state.label("Option 0").props.accessibilityState.checked, false);
  assert.deepEqual(state.writes, []);
  const apply = state.apply().props.onPress;
  apply();
  apply();
  assert.deepEqual(state.writes, [["id-2"]]);
  assert.equal(state.closed(), 1);
});

test("unknown existing values remain visible and can be explicitly removed", () => {
  const state = setup({ value: ["id-0", "missing-existing"] });
  assert.equal(
    state.label("missing-existing").props.accessibilityState.checked,
    true,
  );
  state.select("missing-existing");
  assert.ok(state.rowIds().includes("missing-existing"));
  state.apply().props.onPress();
  assert.deepEqual(state.writes, [["id-0"]]);
});

test("scope or parent-value changes close and invalidate a stale selection", () => {
  for (const patch of [
    { scopeKey: "other-owner" },
    { value: ["id-7"] },
    { disabled: true },
  ]) {
    const state = setup({ scopeKey: "owner-1" });
    state.select("Option 1");
    Object.assign(state.props, patch);
    state.harness.render();
    assert.equal(state.closed(), 1);
    assert.equal(state.apply().props.disabled, true);
    state.apply().props.onPress();
    assert.deepEqual(state.writes, []);
  }
});

test("changed live eligibility blocks added choices while preserving existing grants", () => {
  const state = setup();
  state.select("Option 1");
  state.props.options = options.map((row, i) => ({
    ...row,
    disabled: i === 1,
  }));
  state.harness.render();
  assert.equal(state.apply().props.disabled, true);
  state.apply().props.onPress();
  assert.deepEqual(state.writes, []);
  state.props.options = options;
  state.harness.render();
  state.apply().props.onPress();
  assert.deepEqual(state.writes, [["id-0", "id-1"]]);
});

test("read-only choices expose no Apply action and direct callbacks cannot change them", () => {
  const state = setup({ readOnly: true, disabled: true });
  state.label("Option 1").props.onPress();
  state.harness.flush();
  assert.equal(state.label("Option 1").props.accessibilityState.checked, false);
  assert.equal(
    state.all().some((row) => row.type === "ActionButton"),
    false,
  );
  assert.deepEqual(state.writes, []);
});
