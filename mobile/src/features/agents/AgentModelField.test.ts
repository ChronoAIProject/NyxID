import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { hookHarness } from "../assistant/assistantHookTestHarness";

type Element = { type: unknown; props: Record<string, any> };
type Model = { id: string; label: string; available: boolean };

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

function setup(value: string | undefined = "model-a") {
  const calls: (string | undefined)[] = [];
  const props = {
    value,
    disabled: false,
    models: [
      { id: "model-a", label: "Fast model", available: true },
      { id: "model-b", label: "Research model", available: true },
      { id: "model-c", label: "Retired model", available: false },
    ] as Model[],
    onChange: (model: string | undefined) => calls.push(model),
  };
  const harness = hookHarness<Element>({
    "react/jsx-runtime": {
      jsx: (type: unknown, props: unknown) => ({ type, props }),
      jsxs: (type: unknown, props: unknown) => ({ type, props }),
    },
    "react-native": {
      ...Object.fromEntries(
        ["FlatList", "KeyboardAvoidingView", "Modal", "Pressable", "View"].map(
          (name) => [name, name],
        ),
      ),
      Keyboard: { dismiss: () => {} },
      Platform: { OS: "ios" },
      StyleSheet: { create: (styles: unknown) => styles },
    },
    "lucide-react-native": { Check: "Check", ChevronDown: "ChevronDown" },
    "react-native-safe-area-context": {
      SafeAreaProvider: "SafeAreaProvider",
      SafeAreaView: "SafeAreaView",
    },
    "../../components/workspace/WorkspaceUI": {
      ActionButton: "ActionButton",
      Field: "Field",
      Label: "Label",
    },
    "../../theme/designTokens": { fonts: {}, TOUCH_TARGET: 44 },
    "../../theme/ThemeContext": {
      useTheme: () => ({ colors: { primaryTone: {} } }),
    },
  });
  const { AgentModelField } = harness.load(
    fileURLToPath(new URL("./AgentModelField.tsx", import.meta.url)),
  ) as { AgentModelField: (input: typeof props) => Element };
  harness.mount(() => AgentModelField(props));
  const all = () => elements(harness.value());
  const find = (predicate: (element: Element) => boolean) => {
    const result = all().find(predicate);
    assert.ok(result, "Expected element was rendered");
    return result;
  };
  const modal = () => find((element) => element.type === "Modal");
  const list = () => find((element) => element.type === "FlatList");
  const apply = () => find((element) => element.props.label === "Apply model");
  const open = () => {
    find((element) =>
      element.props.accessibilityLabel?.startsWith("Model for new threads:"),
    ).props.onPress();
    harness.flush();
  };
  const choose = (id: string) => {
    const item = list().props.data.find((model: Model) => model.id === id);
    assert.ok(item);
    const row = list().props.renderItem({ item });
    row.props.onPress();
    harness.flush();
    return row as Element;
  };
  return { harness, props, calls, find, modal, list, apply, open, choose };
}

test("model field opens a single searchable sheet and applies only to the profile draft", () => {
  const ui = setup();
  assert.equal(ui.modal().props.visible, false);
  ui.open();
  assert.equal(ui.modal().props.visible, true);
  assert.equal(ui.modal().props.presentationStyle, "pageSheet");
  ui.choose("model-b");
  assert.deepEqual(ui.calls, []);
  assert.equal(ui.apply().props.disabled, false);
  ui.apply().props.onPress();
  ui.harness.flush();
  assert.deepEqual(ui.calls, ["model-b"]);
  assert.equal(ui.modal().props.visible, false);
});

test("cancel and system dismissal discard model and search drafts", () => {
  const ui = setup();
  for (const cancel of ["button", "system"] as const) {
    ui.open();
    ui.choose("model-b");
    ui.find(
      (element) => element.props.label === "Search models",
    ).props.onChangeText("research");
    ui.harness.flush();
    if (cancel === "system") ui.modal().props.onRequestClose();
    else
      ui.find(
        (element) =>
          element.props.accessibilityLabel === "Cancel model selection",
      ).props.onPress();
    ui.harness.flush();
    assert.deepEqual(ui.calls, []);
    ui.open();
    assert.equal(ui.apply().props.disabled, true);
    assert.equal(
      ui.find((element) => element.props.label === "Search models").props.value,
      "",
    );
  }
});

test("search matches model labels and opaque IDs without changing the selection", () => {
  const ui = setup();
  ui.open();
  const search = (query: string) => {
    ui.find(
      (element) => element.props.label === "Search models",
    ).props.onChangeText(query);
    ui.harness.flush();
  };
  search(" RESEARCH ");
  assert.deepEqual(
    Array.from(ui.list().props.data, (row: Model) => row.id),
    ["model-b"],
  );
  search("MODEL-A");
  assert.deepEqual(
    Array.from(ui.list().props.data, (row: Model) => row.id),
    ["model-a"],
  );
  search("missing");
  assert.equal(ui.list().props.data.length, 0);
  assert.equal(ui.apply().props.disabled, true);
  assert.deepEqual(ui.calls, []);
});

test("missing and unavailable current models remain visible without granting a replacement", () => {
  for (const current of ["unknown-model", "model-c"]) {
    const ui = setup(current);
    ui.open();
    const row = ui.choose(current);
    assert.equal(row.props.disabled, true);
    assert.equal(row.props.accessibilityState.checked, true);
    assert.equal(ui.apply().props.disabled, true);
    ui.apply().props.onPress();
    assert.deepEqual(ui.calls, []);
    ui.choose("model-b");
    ui.props.models = ui.props.models.filter((model) => model.id !== "model-b");
    ui.harness.render();
    assert.equal(ui.apply().props.disabled, true);
    ui.apply().props.onPress();
    assert.deepEqual(ui.calls, []);
  }
});

test("disabled state and external profile changes close and discard an open draft", () => {
  const ui = setup();
  ui.open();
  ui.choose("model-b");
  ui.props.disabled = true;
  ui.harness.render();
  assert.equal(ui.modal().props.visible, false);
  ui.open();
  assert.equal(ui.modal().props.visible, false);
  assert.deepEqual(ui.calls, []);
  ui.props.disabled = false;
  ui.harness.render();
  ui.open();
  ui.choose("model-b");
  ui.props.value = "model-c";
  ui.harness.render();
  assert.equal(ui.modal().props.visible, false);
  ui.open();
  assert.equal(ui.apply().props.disabled, true);
  assert.deepEqual(ui.calls, []);
});

test("an inherited workspace model stays inherited until an available model is applied", () => {
  const ui = setup("model-a");
  ui.props.value = undefined;
  ui.harness.render();
  ui.open();
  assert.equal(ui.apply().props.disabled, true);
  ui.apply().props.onPress();
  assert.deepEqual(ui.calls, []);
  ui.choose("model-b");
  ui.apply().props.onPress();
  ui.harness.flush();
  assert.deepEqual(ui.calls, ["model-b"]);
});
