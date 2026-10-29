import assert from "node:assert/strict";
import { URL, fileURLToPath } from "node:url";
import test from "node:test";
import { deferred, hookHarness } from "../assistant/assistantHookTestHarness";
import { grantSignature } from "./agentContracts";
import type { AgentPermissions } from "./AgentPermissionsField";

type Element = { type: unknown; props: Record<string, any> };
type Option = { value: string; label: string; detail?: string };
type SheetProps = {
  options: Option[];
  value: AgentPermissions;
  disabled: boolean;
  readOnly?: boolean;
  allowAccountRead?: boolean;
  allowGuestAccess?: boolean;
  connectionsLoading?: boolean;
  connectionsError?: boolean;
  saveError?: string;
  creating?: boolean;
  onSave: (value: AgentPermissions) => Promise<boolean>;
  onClose: () => void;
};

const options = [
  { value: "github", label: "GitHub" },
  {
    value: "svc-opaque-7",
    label: "Document research",
    detail: "api-documents",
  },
  { value: "sheets", label: "Spreadsheets" },
];
const baseline: AgentPermissions = {
  services: ["github"],
  account_read: false,
  guest_access: { github: "read" },
};
const plain = <T>(value: T): T => JSON.parse(JSON.stringify(value));

function elements(root: unknown): Element[] {
  if (!root || typeof root !== "object" || !("props" in root)) return [];
  const element = root as Element;
  const children = [element.props.children].flat(Infinity);
  if (element.type === "FlatList") {
    children.push(element.props.ListHeaderComponent);
    if (element.props.data.length) {
      children.push(
        ...element.props.data.map((item: Option) =>
          element.props.renderItem({ item }),
        ),
      );
    } else children.push(element.props.ListEmptyComponent);
  }
  return [element, ...children.flatMap(elements)];
}

function setup(
  overrides: Partial<SheetProps> = {},
  component: "field" | "sheet" = "sheet",
) {
  const saves: AgentPermissions[] = [];
  let closeCount = 0;
  let dismissCount = 0;
  let saveResult: (value: AgentPermissions) => Promise<boolean> = async () =>
    true;
  const props: SheetProps = {
    options: plain(options),
    value: plain(baseline),
    disabled: false,
    onSave: async (value) => {
      saves.push(plain(value));
      return saveResult(value);
    },
    onClose: () => {
      closeCount++;
    },
    ...overrides,
  };
  const harness = hookHarness<Element>({
    "../../components/workspace/useNativeSheetDismiss": {
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
      Fragment: "Fragment",
    },
    "../../components/ServiceIcon": { ServiceIcon: "ServiceIcon" },
    "react-native": {
      ...Object.fromEntries(
        [
          "ActivityIndicator",
          "FlatList",
          "KeyboardAvoidingView",
          "Modal",
          "Pressable",
          "ScrollView",
          "Switch",
          "Text",
          "TextInput",
          "View",
        ].map((name) => [name, name]),
      ),
      Keyboard: { dismiss: () => dismissCount++ },
      Platform: { OS: "ios" },
      StyleSheet: { create: (styles: unknown) => styles },
    },
    "lucide-react-native": Object.fromEntries(
      ["Check", "ChevronLeft", "ChevronRight", "Search", "X"].map((name) => [
        name,
        name,
      ]),
    ),
    "react-native-safe-area-context": {
      SafeAreaProvider: "SafeAreaProvider",
      SafeAreaView: "SafeAreaView",
    },
    "../../components/workspace/WorkspaceUI": {
      ActionButton: "ActionButton",
      Card: "Card",
      Label: "Label",
      Notice: "Notice",
    },
    "../../theme/designTokens": { fonts: {} },
    "../../theme/ThemeContext": {
      useTheme: () => ({ colors: { primaryTone: {} } }),
    },
    "./agentContracts": { grantSignature },
  });
  const exports = harness.load(
    fileURLToPath(new URL("./AgentPermissionsField.tsx", import.meta.url)),
  ) as {
    AgentPermissionsField: (props: SheetProps) => Element;
    AgentPermissionsSheet: (props: SheetProps) => Element;
  };
  harness.mount(() =>
    component === "field"
      ? exports.AgentPermissionsField(props)
      : exports.AgentPermissionsSheet(props),
  );
  const all = () => elements(harness.value());
  const one = (predicate: (element: Element) => boolean) => {
    const result = all().filter(predicate);
    assert.equal(result.length, 1, "Expected exactly one matching element");
    assert.ok(result[0]);
    return result[0];
  };
  const label = (text: string) =>
    one((element) => element.props.accessibilityLabel === text);
  const action = (
    text = props.creating ? "Use these permissions" : "Save permissions",
  ) =>
    one(
      (element) =>
        element.type === "ActionButton" && element.props.label === text,
    );
  const checkbox = (text: string) =>
    one(
      (element) =>
        element.props.accessibilityRole === "checkbox" &&
        element.props.accessibilityLabel === text,
    );
  const select = (text: string) => {
    checkbox(text).props.onPress();
    harness.flush();
  };
  const search = (query: string) => {
    label("Search service permissions").props.onChangeText(query);
    harness.flush();
  };
  const rowIds = () =>
    plain(one((element) => element.type === "FlatList").props.data).map(
      (option: Option) => option.value,
    );
  return {
    harness,
    props,
    exports,
    saves,
    all,
    one,
    label,
    action,
    checkbox,
    select,
    search,
    rowIds,
    closed: () => closeCount,
    dismissedKeyboard: () => dismissCount,
    setSave: (next: typeof saveResult) => {
      saveResult = next;
    },
  };
}

test("the field shows a compact summary and opens the editor only on demand", () => {
  const state = setup(
    {
      value: {
        services: ["github", "svc-opaque-7", "sheets", "missing-service"],
        account_read: true,
        guest_access: {},
      },
    },
    "field",
  );
  assert.equal(state.all().filter((row) => row.type === "FlatList").length, 0);
  assert.equal(
    state
      .all()
      .filter(
        (row) =>
          [row.props.children].flat(Infinity).join("") ===
          "4 services selected",
      ).length,
    1,
  );
  assert.equal(
    state
      .all()
      .filter(
        (row) => row.props.children === "GitHub, Document research +2 more",
      ).length,
    1,
  );
  state.action("Edit permissions").props.onPress();
  state.harness.flush();
  const sheet = state.one(
    (row) => row.type === state.exports.AgentPermissionsSheet,
  );
  assert.deepEqual(plain(sheet.props.value), state.props.value);
  assert.equal(state.dismissedKeyboard(), 1);
  assert.equal(state.saves.length, 0);
  sheet.props.onClose();
  state.harness.flush();
  assert.equal(
    state
      .all()
      .filter((row) => row.type === state.exports.AgentPermissionsSheet).length,
    0,
  );
});

test("search matches service names and opaque IDs while preserving selections", () => {
  const state = setup();
  state.search("api-documents");
  assert.deepEqual(state.rowIds(), ["svc-opaque-7"]);
  assert.equal(
    state.checkbox("Document research").props.accessibilityHint,
    "api-documents",
  );
  state.search("  RESEARCH  ");
  assert.deepEqual(state.rowIds(), ["svc-opaque-7"]);
  state.select("Document research");
  state.search("svc-opaque-7");
  assert.equal(
    state.checkbox("Document research").props.accessibilityState.checked,
    true,
  );
  state.search("GitHub");
  assert.deepEqual(state.rowIds(), ["github"]);
  assert.equal(state.checkbox("GitHub").props.accessibilityState.checked, true);
  state.label("Clear service search").props.onPress();
  state.label("Selected services").props.onPress();
  state.harness.flush();
  assert.deepEqual(state.rowIds(), ["github", "svc-opaque-7"]);
  assert.equal(state.saves.length, 0);
});

test("an unavailable existing grant survives removal, reselecting and connection refresh", async () => {
  const state = setup({
    value: {
      ...plain(baseline),
      services: ["github", "legacy-service"],
      guest_access: { github: "read", "legacy-service": "all" },
    },
  });
  assert.equal(
    state.checkbox("legacy-service").props.accessibilityState.checked,
    true,
  );
  state.select("legacy-service");
  assert.ok(state.rowIds().includes("legacy-service"));
  assert.equal(
    state.checkbox("legacy-service").props.accessibilityState.checked,
    false,
  );
  state.props.options = [{ value: "sheets", label: "Updated sheets" }];
  state.harness.render();
  assert.deepEqual(state.rowIds(), ["sheets", "github", "legacy-service"]);
  state.select("legacy-service");
  state.select("Updated sheets");
  state.action().props.onPress();
  await state.harness.settle();
  assert.deepEqual(state.saves, [
    {
      services: ["github", "legacy-service", "sheets"],
      account_read: false,
      guest_access: { github: "read", "legacy-service": "all" },
    },
  ]);
  assert.equal(state.closed(), 1);
});

test("cancel and native modal dismissal discard edits without saving", () => {
  for (const method of ["cancel", "requestClose"] as const) {
    const state = setup();
    state.select("Spreadsheets");
    state.label("Read account information").props.onValueChange(true);
    state.harness.flush();
    if (method === "cancel")
      state.label("Cancel permission changes").props.onPress();
    else state.one((row) => row.type === "Modal").props.onRequestClose();
    assert.equal(state.closed(), 1);
    assert.deepEqual(state.saves, []);
    assert.deepEqual(state.props.value, baseline);
    state.harness.unmount();
    const reopened = setup({ value: state.props.value });
    assert.equal(
      reopened.checkbox("Spreadsheets").props.accessibilityState.checked,
      false,
    );
    assert.equal(reopened.label("Read account information").props.value, false);
  }
});

test("only explicit save writes; a failed save retains the draft and can be retried", async () => {
  const state = setup({ saveError: "Access changed. Retry your selection." });
  state.setSave(async () => false);
  assert.equal(state.action().props.disabled, true);
  state.action().props.onPress();
  assert.equal(state.saves.length, 0);
  state.select("Document research");
  state.search("research");
  assert.equal(state.saves.length, 0);
  state.action().props.onPress();
  await state.harness.settle();
  assert.equal(state.saves.length, 1);
  assert.equal(state.closed(), 0);
  assert.equal(
    state.label("Search service permissions").props.value,
    "research",
  );
  assert.equal(
    state.checkbox("Document research").props.accessibilityState.checked,
    true,
  );
  assert.equal(
    state
      .all()
      .filter(
        (row) => row.props.message === "Access changed. Retry your selection.",
      ).length,
    1,
  );
  state.setSave(async () => true);
  state.action().props.onPress();
  await state.harness.settle();
  assert.equal(state.saves.length, 2);
  assert.deepEqual(state.saves[1], state.saves[0]);
  assert.equal(state.closed(), 1);
});

test("same-tick duplicate submit, close and edits are blocked throughout an in-flight save", async () => {
  const state = setup();
  const pending = deferred<boolean>();
  state.setSave(() => pending.promise);
  state.select("Spreadsheets");
  const submit = state.action().props.onPress;
  const close = state.label("Cancel permission changes").props.onPress;
  const requestClose = state.one((row) => row.type === "Modal").props
    .onRequestClose;
  const toggle = state.checkbox("GitHub").props.onPress;
  const toggleAccount = state.label("Read account information").props
    .onValueChange;
  submit();
  submit();
  close();
  requestClose();
  toggle();
  toggleAccount(true);
  state.harness.flush();
  assert.equal(state.saves.length, 1);
  assert.equal(state.closed(), 0);
  assert.equal(state.action().props.loading, true);
  assert.equal(state.checkbox("GitHub").props.accessibilityState.checked, true);
  assert.equal(state.label("Read account information").props.value, false);
  assert.equal(state.label("Search service permissions").props.editable, false);
  pending.resolve(false);
  await state.harness.settle();
  assert.equal(state.closed(), 0);
  assert.equal(state.action().props.loading, false);
  state.setSave(async () => true);
  state.action().props.onPress();
  await state.harness.settle();
  assert.equal(state.saves.length, 2);
  assert.equal(state.closed(), 1);
});

test("organization flags hide account and guest controls without clearing their original values", async () => {
  const value: AgentPermissions = {
    services: ["github"],
    account_read: true,
    guest_access: { github: "all", "legacy-service": "read" },
  };
  const state = setup({
    value,
    allowAccountRead: false,
    allowGuestAccess: false,
  });
  assert.equal(state.all().filter((row) => row.type === "Switch").length, 0);
  assert.equal(
    state
      .all()
      .filter((row) =>
        row.props.accessibilityLabel?.startsWith("Guest access for "),
      ).length,
    0,
  );
  state.select("Spreadsheets");
  state.action().props.onPress();
  await state.harness.settle();
  assert.deepEqual(state.saves, [{ ...value, services: ["github", "sheets"] }]);
});

test("creation applies a local permission draft only after Use these permissions", async () => {
  const state = setup({
    creating: true,
    value: { services: [], account_read: false, guest_access: {} },
    allowGuestAccess: false,
  });
  state.select("Document research");
  state.label("Read account information").props.onValueChange(true);
  state.harness.flush();
  assert.equal(state.saves.length, 0);
  assert.deepEqual(state.props.value.services, []);
  state.action("Use these permissions").props.onPress();
  await state.harness.settle();
  assert.deepEqual(state.saves, [
    { services: ["svc-opaque-7"], account_read: true, guest_access: {} },
  ]);
  assert.equal(state.closed(), 1);
});

test("returning from guest access preserves the service draft and selected guest level", async () => {
  const state = setup();
  state.select("Spreadsheets");
  state.label("Guest access for GitHub: Read").props.onPress();
  state.harness.flush();
  state.label("All granted").props.onPress();
  state.harness.flush();
  state.one((row) => row.type === "Modal").props.onRequestClose();
  state.harness.flush();
  assert.equal(state.closed(), 0);
  assert.equal(state.saves.length, 0);
  assert.equal(
    state.checkbox("Spreadsheets").props.accessibilityState.checked,
    true,
  );
  state.label("Guest access for GitHub: All granted").props.onPress();
  state.harness.flush();
  assert.equal(
    state.label("All granted").props.accessibilityState.checked,
    true,
  );
  state.label("Back to services").props.onPress();
  state.harness.flush();
  state.action().props.onPress();
  await state.harness.settle();
  assert.deepEqual(state.saves[0], {
    services: ["github", "sheets"],
    account_read: false,
    guest_access: { github: "all" },
  });
});

test("disabled permissions stay read only and connection loading/errors preserve existing grants", () => {
  const field = setup({ disabled: true }, "field");
  assert.equal(field.action("Edit permissions").props.disabled, true);
  field.action("Edit permissions").props.onPress();
  field.harness.flush();
  assert.equal(
    field
      .all()
      .filter((row) => row.type === field.exports.AgentPermissionsSheet).length,
    0,
  );
  const state = setup({
    disabled: true,
    connectionsLoading: true,
    connectionsError: true,
    options: [],
  });
  assert.equal(state.checkbox("github").props.disabled, true);
  assert.equal(state.label("Read account information").props.disabled, true);
  assert.equal(state.action().props.disabled, true);
  assert.ok(state.label("Loading services"));
  assert.equal(
    state
      .all()
      .filter((row) =>
        row.props.message?.startsWith("Could not load available connections."),
      ).length,
    1,
  );
  state.select("github");
  state.label("Read account information").props.onValueChange(true);
  state.action().props.onPress();
  state.harness.flush();
  assert.equal(state.checkbox("github").props.accessibilityState.checked, true);
  assert.equal(state.label("Read account information").props.value, false);
  assert.equal(state.saves.length, 0);
});

test("read-only inspection opens the full service list without exposing save or allowing direct mutation callbacks", () => {
  const field = setup({ disabled: true, readOnly: true }, "field");
  assert.equal(field.action("View permissions").props.disabled, false);
  field.action("View permissions").props.onPress();
  field.harness.flush();
  const presented = field.one(
    (row) => row.type === field.exports.AgentPermissionsSheet,
  );
  assert.equal(presented.props.readOnly, true);
  assert.equal(presented.props.disabled, true);
  const state = setup(presented.props);
  assert.deepEqual(state.rowIds(), ["github", "svc-opaque-7", "sheets"]);
  assert.equal(
    state.all().filter((row) => row.type === "ActionButton").length,
    0,
  );
  assert.equal(state.checkbox("GitHub").props.disabled, true);
  assert.equal(state.checkbox("Spreadsheets").props.disabled, true);
  assert.equal(state.label("Read account information").props.disabled, true);
  assert.equal(
    state.label("Guest access for GitHub: Read").props.disabled,
    false,
  );
  state.select("GitHub");
  state.select("Spreadsheets");
  state.label("Read account information").props.onValueChange(true);
  state.label("Guest access for GitHub: Read").props.onPress();
  state.harness.flush();
  assert.equal(state.label("All granted").props.disabled, true);
  state.label("All granted").props.onPress();
  state.harness.flush();
  assert.equal(state.label("Read").props.accessibilityState.checked, true);
  state.label("Back to services").props.onPress();
  state.harness.flush();
  assert.equal(state.checkbox("GitHub").props.accessibilityState.checked, true);
  assert.equal(
    state.checkbox("Spreadsheets").props.accessibilityState.checked,
    false,
  );
  assert.equal(state.label("Read account information").props.value, false);
  assert.deepEqual(state.saves, []);
  assert.deepEqual(field.saves, []);
  state.search("research");
  assert.deepEqual(state.rowIds(), ["svc-opaque-7"]);
});

test("read-only mode also guards guest-level mutation in an already open guest view", () => {
  const state = setup();
  state.label("Guest access for GitHub: Read").props.onPress();
  state.harness.flush();
  state.props.readOnly = true;
  state.harness.render();
  assert.equal(state.label("All granted").props.disabled, true);
  state.label("All granted").props.onPress();
  state.harness.flush();
  assert.equal(state.label("Read").props.accessibilityState.checked, true);
  assert.equal(
    state.label("All granted").props.accessibilityState.checked,
    false,
  );
  assert.equal(
    state.all().filter((row) => row.type === "ActionButton").length,
    0,
  );
  state.label("Back to services").props.onPress();
  state.harness.flush();
  assert.ok(state.label("Guest access for GitHub: Read"));
  assert.equal(state.saves.length, 0);
});

test("unexpected save rejection shows a retry error without losing the draft", async () => {
  const state = setup();
  state.setSave(async () => {
    return Promise.reject({ reason: "Connection interrupted" });
  });
  state.select("Spreadsheets");
  state.action().props.onPress();
  await state.harness.settle();
  assert.equal(state.closed(), 0);
  assert.equal(
    state.checkbox("Spreadsheets").props.accessibilityState.checked,
    true,
  );
  assert.equal(
    state
      .all()
      .filter(
        (row) => row.props.message === "Could not save permissions. Try again.",
      ).length,
    1,
  );
  state.setSave(async () => true);
  state.action().props.onPress();
  await state.harness.settle();
  assert.equal(state.closed(), 1);
});
