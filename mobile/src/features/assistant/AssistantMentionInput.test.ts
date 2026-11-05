import assert from "node:assert/strict";
import { URL, fileURLToPath } from "node:url";
import test from "node:test";
import { hookHarness } from "./assistantHookTestHarness";
import type { AssistantMentionOption } from "./AssistantMentionInput";

type Element = {
  type: unknown;
  key?: unknown;
  props: Record<string, any>;
};

const options: AssistantMentionOption[] = [
  {
    id: "report",
    handle: "file-1",
    label: "Quarterly report.pdf",
    kind: "file",
  },
  { id: "photo", handle: "image-1", label: "Office photo", kind: "image" },
  {
    id: "research",
    handle: "research",
    label: "Research helper",
    kind: "agent",
  },
];

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
  initial: Record<string, unknown> = {},
  platform: "android" | "ios" = "android",
) {
  let maxHeight = 120;
  const writes: string[] = [];
  const props: Record<string, any> = {
    value: "",
    onChangeText: (text: string) => {
      writes.push(text);
      props.value = text;
    },
    mentions: options,
    editable: true,
    placeholder: "Message",
    accessibilityLabel: "Message",
    generation: 0,
    ...initial,
  };
  const harness = hookHarness<Element>({
    "../display/ChatLayoutContext": {
      useComposerHeight: () => maxHeight,
      useChatColumn: () => ({
        width: "100%",
        maxWidth: 758,
        alignSelf: "center",
      }),
    },
    "../../components/workspace/ScrollEdgeFades": {
      ScrollEdgeFades: "ScrollEdgeFades",
      useScrollEdges: () => ({
        edges: { start: false, end: false },
        layout() {},
        content() {},
        scroll() {},
      }),
    },
    "../../components/AgentAvatar": { AgentAvatar: "AgentAvatar", AgentAvatarStack: "AgentAvatarStack" },
    "react/jsx-runtime": {
      jsx: (type: unknown, props: unknown, key?: unknown) => ({
        type,
        props,
        key,
      }),
      jsxs: (type: unknown, props: unknown, key?: unknown) => ({
        type,
        props,
        key,
      }),
    },
    "react-native": {
      Platform: { OS: platform },
      Pressable: "Pressable",
      ScrollView: "ScrollView",
      Text: "Text",
      TextInput: "TextInput",
      View: "View",
      StyleSheet: { create: (styles: unknown) => styles },
    },
    "lucide-react-native": {
      Bot: "Bot",
      Clipboard: "Clipboard",
      FileText: "FileText",
      Image: "Image",
    },
    "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
    "../../theme/designTokens": { fonts: {} },
  });
  const component = harness.load(
    fileURLToPath(new URL("./AssistantMentionInput.tsx", import.meta.url)),
  ).AssistantMentionInput as (props: any) => Element;
  harness.mount(() => component(props));
  const all = () => elements(harness.flush());
  const input = () => {
    const result = all().find((element) => element.type === "TextInput");
    assert.ok(result);
    return result;
  };
  const button = () => {
    const result = all().find(
      (element) =>
        element.props.accessibilityLabel === "Mention an attachment or agent",
    );
    assert.ok(result);
    return result;
  };
  const candidates = () =>
    all().filter(
      (element) =>
        element.props.accessibilityLabel?.startsWith("Mention ") &&
        element.props.accessibilityLabel !== "Mention an attachment or agent",
    );
  const select = (start: number, end = start) => {
    input().props.onSelectionChange({
      nativeEvent: { selection: { start, end } },
    });
    harness.flush();
  };
  const focus = () => {
    input().props.onFocus();
    harness.flush();
  };
  const choose = (handle: string) => {
    const result = candidates().find((element) =>
      element.props.accessibilityLabel.endsWith(`@${handle}`),
    );
    assert.ok(result, `Expected @${handle} candidate`);
    result.props.onPress();
    harness.render();
  };
  return {
    setMaxHeight: (height: number) => {
      maxHeight = height;
      harness.render();
    },
    props,
    writes,
    harness,
    input,
    button,
    candidates,
    select,
    focus,
    choose,
  };
}

test("typing a mention searches options and replaces its full handle at the native caret", () => {
  const view = setup({ value: "Please @fi-le later" });
  view.focus();
  view.select(10);
  assert.deepEqual(
    view.candidates().map((element) => element.props.accessibilityLabel),
    ["Mention Quarterly report.pdf, @file-1", "Mention Office photo, @image-1"],
  );
  view.choose("file-1");
  assert.equal(view.props.value, "Please @file-1 later");
  assert.equal(view.input().props.selection.start, "Please @file-1 ".length);
  assert.equal(view.input().props.selection.end, "Please @file-1 ".length);
  assert.equal(view.candidates().length, 0);
});

test("the @ button preserves text on both sides of a caret inside a word", () => {
  const view = setup({ value: "helloworld" });
  view.focus();
  view.select(5);
  view.button().props.onPress();
  view.harness.render();
  assert.equal(view.props.value, "hello @ world");
  view.choose("file-1");
  assert.equal(view.props.value, "hello @file-1 world");
});

test("selected text suppresses autocomplete and the @ button preserves that selection", () => {
  const view = setup({ value: "@file-1 tail" });
  view.focus();
  view.select(0, 7);
  assert.equal(view.candidates().length, 0);
  view.button().props.onPress();
  view.harness.render();
  assert.equal(view.props.value, "@file-1 @ tail");
  view.choose("image-1");
  assert.equal(view.props.value, "@file-1 @image-1 tail");
});

test("native typing and IME updates do not force a controlled cursor or remount the input", () => {
  const view = setup();
  const key = view.input().key;
  for (const text of ["n", "ni", "你", "你好", ""]) {
    view.input().props.onChangeText(text);
    view.harness.render();
    assert.equal(view.input().key, key);
    assert.equal(view.input().props.selection, undefined);
  }
  assert.deepEqual(view.writes, ["n", "ni", "你", "你好", ""]);
});

test("a private composer has no mention action or suggestions and keeps typed @ text intact", () => {
  const view = setup({ mentions: undefined });
  view.focus();
  view.input().props.onChangeText("Please check @file-1");
  view.harness.render();
  view.select("Please check @file-1".length);
  assert.equal(view.props.value, "Please check @file-1");
  assert.equal(view.candidates().length, 0);
  assert.equal(
    elements(view.harness.value()).some(
      (element) => element.type === "Pressable",
    ),
    false,
  );
});

test("generation changes fence old text, selection and suggestion handlers", () => {
  const view = setup({ value: "@fi" });
  view.focus();
  view.select(3);
  const oldInput = view.input();
  const oldSuggestion = view.candidates()[0];
  assert.ok(oldSuggestion);
  view.props.generation = 1;
  view.props.value = "";
  view.harness.render();
  assert.notEqual(view.input().key, oldInput.key);
  oldInput.props.onChangeText("old composing text");
  oldInput.props.onSelectionChange({
    nativeEvent: { selection: { start: 3, end: 3 } },
  });
  oldSuggestion.props.onPress();
  view.harness.flush();
  assert.equal(view.props.value, "");
  assert.deepEqual(view.writes, []);
  view.input().props.onChangeText("新");
  view.harness.render();
  assert.equal(view.props.value, "新");
});

test("an external send/clear fences delayed native edits even without a generation change", () => {
  const view = setup({ value: "sent message" });
  const oldInput = view.input();
  view.props.value = "";
  view.harness.render();
  assert.notEqual(view.input().key, oldInput.key);
  oldInput.props.onChangeText("sent message restored by stale IME event");
  view.harness.flush();
  assert.equal(view.props.value, "");
  assert.deepEqual(view.writes, []);
});

test("disabled and unmounted composers ignore retained native callbacks", () => {
  const view = setup({ value: "@fi" });
  view.focus();
  view.select(3);
  const prior = view.input();
  const suggestion = view.candidates()[0];
  view.props.editable = false;
  view.harness.render();
  assert.equal(view.button().props.disabled, true);
  assert.equal(view.candidates().length, 0);
  prior.props.onChangeText("blocked");
  suggestion!.props.onPress();
  assert.deepEqual(view.writes, []);
  view.props.editable = true;
  view.harness.render();
  const current = view.input();
  view.harness.unmount();
  current.props.onChangeText("after unmount");
  current.props.onBlur();
  assert.deepEqual(view.writes, []);
});

const inputStyle = (element: Element) =>
  Object.assign({}, ...element.props.style.flat(Infinity));

test("native measured content grows the shared composer to its preference/keyboard cap and keeps scrolling", () => {
  const view = setup();
  view.setMaxHeight(140);
  const key = view.input().key;
  assert.equal(inputStyle(view.input()).height, 44);
  view.input().props.onChangeText("1234567890".repeat(20));
  view.harness.render();
  view
    .input()
    .props.onContentSizeChange({
      nativeEvent: { contentSize: { width: 256, height: 96.4 } },
    });
  view.harness.flush();
  assert.equal(inputStyle(view.input()).height, 97);
  view
    .input()
    .props.onContentSizeChange({
      nativeEvent: { contentSize: { width: 256, height: 275 } },
    });
  view.harness.flush();
  assert.equal(inputStyle(view.input()).height, 140);
  assert.equal(view.input().props.multiline, true);
  assert.equal(view.input().props.scrollEnabled, true);
  assert.equal(
    view.input().key,
    key,
    "growing does not remount the focused native input",
  );
  view.setMaxHeight(80);
  assert.equal(
    inputStyle(view.input()).height,
    80,
    "keyboard space can reduce the active cap",
  );
  view.setMaxHeight(240);
  assert.equal(
    inputStyle(view.input()).height,
    240,
    "increasing the cap reuses measured overflow",
  );
  view
    .input()
    .props.onContentSizeChange({
      nativeEvent: { contentSize: { width: 300, height: 58 } },
    });
  view.harness.flush();
  assert.equal(
    inputStyle(view.input()).height,
    58,
    "rewrapping or deleting text shrinks the composer",
  );
  view.harness.unmount();
});

test("iOS sizes the multiline composer to its text under the shared cap", () => {
  const view = setup({}, "ios");
  view.setMaxHeight(140);
  view.input().props.onChangeText("1234567890".repeat(20));
  view.harness.render();
  view.input().props.onContentSizeChange({
    nativeEvent: { contentSize: { width: 256, height: 96.4 } },
  });
  view.harness.flush();
  const style = inputStyle(view.input());
  // A measured height pinned the iOS input at one line; native sizing grows it.
  assert.equal(style.height, undefined);
  assert.equal(style.maxHeight, 140);
  assert.equal(style.minHeight, 44);
  view.harness.unmount();
});

test("draft replacement resets measured height and rejects stale size events while readonly drafts still measure", () => {
  const view = setup({ value: "old draft" });
  const oldInput = view.input();
  oldInput.props.onContentSizeChange({
    nativeEvent: { contentSize: { height: 200 } },
  });
  view.harness.flush();
  assert.equal(inputStyle(view.input()).height, 120);
  view.props.value = "";
  view.harness.render();
  assert.equal(inputStyle(view.input()).height, 44);
  oldInput.props.onContentSizeChange({
    nativeEvent: { contentSize: { height: 300 } },
  });
  view.harness.flush();
  assert.equal(inputStyle(view.input()).height, 44);
  view.props.editable = false;
  view.harness.render();
  view
    .input()
    .props.onContentSizeChange({
      nativeEvent: { contentSize: { height: 70 } },
    });
  view.harness.flush();
  assert.equal(inputStyle(view.input()).height, 70);
  view
    .input()
    .props.onContentSizeChange({
      nativeEvent: { contentSize: { height: NaN } },
    });
  view.harness.flush();
  assert.equal(inputStyle(view.input()).height, 70);
  view.harness.unmount();
});

test("the full-width composer keeps mention and action controls below its live input while typing and growing", () => {
  const view = setup({
    renderActions: (mention: Element) => ({
      type: "Toolbar",
      props: { children: [mention, { type: "Send", props: {} }] },
    }),
  });
  const root = view.harness.value();
  const toolbar = elements(root).find((node) => node.type === "Toolbar");
  assert.ok(toolbar);
  assert.ok(
    elements(toolbar).some(
      (node) =>
        node.props.accessibilityLabel === "Mention an attachment or agent",
    ),
  );
  assert.equal(
    elements(toolbar).some((node) => node.type === "TextInput"),
    false,
  );
  const key = view.input().key;
  view
    .input()
    .props.onChangeText("A draft that uses the width above the controls");
  view.harness.render();
  view
    .input()
    .props.onContentSizeChange({
      nativeEvent: { contentSize: { height: 96 } },
    });
  view.harness.flush();
  assert.equal(inputStyle(view.input()).height, 96);
  assert.equal(view.input().key, key);
  view.focus();
  view.button().props.onPress();
  view.harness.render();
  assert.equal(
    view.props.value,
    "A draft that uses the width above the controls @ ",
  );
  view.choose("research");
  assert.equal(
    view.props.value,
    "A draft that uses the width above the controls @research ",
  );
  assert.equal(
    view.input().key,
    key,
    "toolbar actions preserve the native input and selection owner",
  );
  view.harness.unmount();
});
