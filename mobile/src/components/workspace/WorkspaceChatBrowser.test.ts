import assert from "node:assert/strict";
import { URL, fileURLToPath } from "node:url";
import test from "node:test";
import { hookHarness } from "../../features/assistant/assistantHookTestHarness";
import { searchDrawerItems } from "./workspaceChatListModel";

type Element = { type: unknown; props: Record<string, any> };
type Item = { id: string; title: string };
type Props = {
  visible?: boolean;
  title: string;
  scopeKey: string;
  description?: string;
  items: readonly Item[];
  searchText: (item: Item) => string;
  renderItem: (item: Item) => Element;
  onClose: () => void;
  onDismiss?: () => void;
  onBack?: () => void;
  emptyText?: string;
  footer?: Element;
  primaryAction?: {
    label: string;
    accessibilityLabel: string;
    onPress: () => void;
  };
};

function elements(root: Element): Element[] {
  if (typeof root.type === "function")
    return elements((root.type as (props: unknown) => Element)(root.props));
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

function content(root: Element): string {
  return [root.props.children]
    .flat(Infinity)
    .map((child) =>
      typeof child === "string" || typeof child === "number"
        ? String(child)
        : child && typeof child === "object" && "props" in child
          ? content(child)
          : "",
    )
    .join("");
}

/** Run the actual browser; only native components and device APIs are replaced. */
function setup(
  overrides: Partial<Props> = {},
  platform: "ios" | "android" = "ios",
) {
  let started = 0;
  let closed = 0;
  let dismissed = 0;
  let dismissals = 0;
  const props: Props = {
    title: "HR Admin Bot",
    scopeKey: "chats:hr-admin",
    description: "Search this agent’s chat history.",
    items: [
      { id: "first", title: "First conversation" },
      { id: "second", title: "Second conversation" },
    ],
    searchText: (item) => item.title,
    renderItem: (item) => ({ type: "Text", props: { children: item.title } }),
    onClose: () => {
      closed++;
    },
    onDismiss: () => {
      dismissed++;
    },
    emptyText: "No chats yet.",
    primaryAction: {
      label: "New chat",
      accessibilityLabel: "Start a chat with HR Admin Bot",
      onPress: () => {
        started++;
      },
    },
    ...overrides,
  };
  const harness = hookHarness<Element>({
    "../../lib/accessibility/useReducedMotion": {
      useReducedMotion: () => false,
    },
    "react/jsx-runtime": {
      jsx: (type: unknown, elementProps: unknown) => ({
        type,
        props: elementProps,
      }),
      jsxs: (type: unknown, elementProps: unknown) => ({
        type,
        props: elementProps,
      }),
    },
    "react-native": {
      ...Object.fromEntries(
        [
          "FlatList",
          "KeyboardAvoidingView",
          "Modal",
          "Pressable",
          "Text",
          "TextInput",
          "View",
        ].map((name) => [name, name]),
      ),
      Keyboard: {
        dismiss: () => {
          dismissals++;
        },
      },
      Platform: { OS: platform },
      StyleSheet: { create: (styles: unknown) => styles },
    },
    "lucide-react-native": Object.fromEntries(
      ["ChevronLeft", "Search", "X", "Plus", "MessageSquare"].map((name) => [
        name,
        name,
      ]),
    ),
    "react-native-safe-area-context": {
      SafeAreaProvider: "SafeAreaProvider",
      SafeAreaView: "SafeAreaView",
    },
    "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
    "../../theme/designTokens": { fonts: {} },
    "./workspaceChatListModel": { searchDrawerItems },
  });
  const { WorkspaceChatBrowser } = harness.load(
    fileURLToPath(new URL("./WorkspaceChatBrowser.tsx", import.meta.url)),
  ) as { WorkspaceChatBrowser: (props: Props) => Element };
  harness.mount(() => WorkspaceChatBrowser(props));
  const all = () => elements(harness.flush());
  const one = (predicate: (row: Element) => boolean) => {
    const row = all().find(predicate);
    assert.ok(row, "Expected browser control");
    return row;
  };
  const search = () => one((row) => row.type === "TextInput");
  const list = () => one((row) => row.type === "FlatList");
  const button = (label: string) =>
    one((row) => row.props.accessibilityLabel === label);
  return {
    harness,
    all,
    list,
    search,
    button,
    modal: () => one((row) => row.type === "Modal").props,
    counts: () => ({ started, closed, dismissed, dismissals }),
    query: (value: string) => {
      search().props.onChangeText(value);
      harness.flush();
    },
    update: (next: Partial<Props>) => {
      Object.assign(props, next);
      harness.render();
    },
  };
}

test("the primary action is rendered above saved history, outside its list footer", () => {
  const env = setup({
    footer: { type: "Text", props: { children: "Load older chats" } },
  });
  const rows = env.all();
  const primaryIndex = rows.findIndex(
    (row) => row.props.accessibilityLabel === "Start a chat with HR Admin Bot",
  );
  const listIndex = rows.findIndex((row) => row.type === "FlatList");
  assert.ok(primaryIndex >= 0 && primaryIndex < listIndex);
  assert.equal(content(rows[primaryIndex]!), "New chat");
  assert.equal(
    elements(env.list().props.ListFooterComponent).some(
      (row) =>
        row.props.accessibilityLabel === "Start a chat with HR Admin Bot",
    ),
    false,
  );
  env.button("Start a chat with HR Admin Bot").props.onPress();
  assert.equal(env.counts().started, 1);
  env.harness.unmount();
});

test("an empty history keeps its primary action while hiding unnecessary search and counts", () => {
  const env = setup({ items: [] });
  assert.equal(
    env.all().some((row) => row.type === "TextInput"),
    false,
  );
  assert.equal(
    env.all().some((row) => row.props.accessibilityLiveRegion === "polite"),
    false,
  );
  assert.match(content(env.list().props.ListEmptyComponent), /No chats yet/);
  env.button("Start a chat with HR Admin Bot").props.onPress();
  assert.equal(env.counts().started, 1);
  env.harness.unmount();
});

test("an unsuccessful search retains New chat and offers a clear-search recovery", () => {
  const env = setup();
  env.query("No matching conversation");
  assert.equal(env.search().props.value, "No matching conversation");
  assert.equal(env.list().props.data.length, 0);
  assert.match(content(env.list().props.ListEmptyComponent), /No matches/);
  env.button("Start a chat with HR Admin Bot").props.onPress();
  assert.equal(env.counts().started, 1);
  env.button("Clear HR Admin Bot search").props.onPress();
  env.harness.flush();
  assert.equal(env.search().props.value, "");
  assert.equal(env.list().props.data.length, 2);
  env.harness.unmount();
});

test("an active search stays visible if a refresh removes all saved history", () => {
  const env = setup();
  env.query("First");
  env.update({ items: [] });
  assert.equal(env.search().props.value, "First");
  assert.equal(env.list().props.data.length, 0);
  assert.match(content(env.list().props.ListEmptyComponent), /No matches/);
  env.button("Clear HR Admin Bot search").props.onPress();
  env.harness.flush();
  assert.equal(
    env.all().some((row) => row.type === "TextInput"),
    false,
  );
  env.harness.unmount();
});

test("a past-agent empty state exposes Choose another agent as its primary destination", () => {
  let choices = 0;
  const env = setup({
    items: [],
    description: "Past agent · saved conversations are read only.",
    emptyText: "This past agent has no saved chats.",
    primaryAction: {
      label: "Choose another agent",
      accessibilityLabel: "Choose another agent",
      onPress: () => {
        choices++;
      },
    },
  });
  assert.equal(
    env
      .all()
      .some(
        (row) =>
          row.props.accessibilityLabel === "Start a chat with HR Admin Bot",
      ),
    false,
  );
  const rows = env.all();
  assert.ok(
    rows.findIndex(
      (row) => row.props.accessibilityLabel === "Choose another agent",
    ) < rows.findIndex((row) => row.type === "FlatList"),
  );
  env.button("Choose another agent").props.onPress();
  assert.equal(choices, 1);
  env.harness.unmount();
});

test("switching scopes clears the previous agent search without losing the primary action", () => {
  const env = setup();
  env.query("First");
  env.update({ scopeKey: "chats:another-agent" });
  assert.equal(env.search().props.value, "");
  assert.equal(env.list().props.data.length, 2);
  assert.ok(env.button("Start a chat with HR Admin Bot"));
  env.harness.unmount();
});

test("iOS keeps the child modal mounted until its native dismissal and notifies only once", () => {
  const env = setup();
  assert.equal(env.modal().visible, true);
  env.modal().onDismiss();
  assert.equal(env.counts().dismissed, 0);
  env.update({ visible: false });
  assert.equal(env.modal().visible, false);
  assert.equal(env.counts().dismissed, 0);
  env.modal().onDismiss();
  env.harness.flush();
  assert.equal(env.counts().dismissed, 1);
  env.modal().onDismiss();
  env.harness.flush();
  assert.equal(env.counts().dismissed, 1);
  env.harness.unmount();
});

test("an obsolete native dismissal cannot complete a reopened browser or an unmounted one", () => {
  const env = setup();
  env.update({ visible: false });
  const previousDismissal = env.modal().onDismiss;
  env.update({ visible: true });
  previousDismissal();
  env.harness.flush();
  assert.equal(env.counts().dismissed, 0);
  env.update({ visible: false });
  const finalDismissal = env.modal().onDismiss;
  env.harness.unmount();
  finalDismissal();
  assert.equal(env.counts().dismissed, 0);
});

test("a browser that starts hidden does not report a dismissal that never occurred", () => {
  for (const platform of ["ios", "android"] as const) {
    const env = setup({ visible: false }, platform);
    env.modal().onDismiss?.();
    env.harness.flush();
    assert.equal(env.counts().dismissed, 0);
    env.harness.unmount();
  }
});

test("Android completes child dismissal after the hidden state commits without relying on native onDismiss", () => {
  const env = setup({}, "android");
  assert.equal(env.counts().dismissed, 0);
  env.update({ visible: false });
  assert.equal(env.modal().visible, false);
  assert.equal(env.counts().dismissed, 1);
  env.harness.render();
  assert.equal(env.counts().dismissed, 1);
  env.harness.unmount();
});
