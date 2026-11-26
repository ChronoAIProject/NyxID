import { composerInputMinHeight } from "./composerGeometry";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
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
  fontScale = 1,
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
    "./composerGeometry": { composerInputMinHeight },
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
    "../../components/AgentAvatar": {
      AgentAvatar: "AgentAvatar",
      AgentAvatarStack: "AgentAvatarStack",
    },
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
      useWindowDimensions: () => ({ fontScale }),
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
  view.input().props.onContentSizeChange({
    nativeEvent: { contentSize: { width: 256, height: 96.4 } },
  });
  view.harness.flush();
  assert.equal(inputStyle(view.input()).height, 97);
  view.input().props.onContentSizeChange({
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
  view.input().props.onContentSizeChange({
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
  view.input().props.onContentSizeChange({
    nativeEvent: { contentSize: { height: 70 } },
  });
  view.harness.flush();
  assert.equal(inputStyle(view.input()).height, 70);
  view.input().props.onContentSizeChange({
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
  view.input().props.onContentSizeChange({
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

test("the empty iOS placeholder stays one visual line and cannot enlarge the normal composer", () => {
  const f = setup({ value: "", placeholder: "Message NyxBot" }, "ios");
  assert.equal(f.input().props.placeholder, "");
  assert.equal(f.input().props.accessibilityHint, "Message NyxBot");
  assert.equal(inputStyle(f.input()).height, 44);
  f.input().props.onContentSizeChange({
    nativeEvent: { contentSize: { height: 200 } },
  });
  f.harness.flush();
  assert.equal(inputStyle(f.input()).height, 44);
  const placeholder = elements(f.harness.flush()).find(
    (n) => n.type === "Text" && n.props.children === "Message NyxBot",
  )!;
  assert.equal(placeholder.props.numberOfLines, 1);
  assert.equal(placeholder.props.pointerEvents, "none");
  f.harness.unmount();
});

test("the composer field and its placeholder line scale together at every Dynamic Type size", () => {
  // 3.12 is iOS's largest accessibility size for body text.
  for (const fontScale of [1, 1.35, 2, 3.12]) {
    const empty = setup(
      { value: "", placeholder: "Message NyxBot" },
      "ios",
      fontScale,
    );
    const field = inputStyle(empty.input());
    const placeholder = elements(empty.harness.flush()).find(
      (n) => n.type === "Text" && n.props.children === "Message NyxBot",
    )!;
    const line = Object.assign({}, ...[placeholder.props.style].flat(Infinity));
    // RN scales lineHeight by the same multiplier as fontSize.
    assert.equal(field.height, composerInputMinHeight(fontScale));
    assert.ok(
      field.height >=
        line.top + line.lineHeight * fontScale + field.paddingBottom,
      `placeholder fits the empty field at ${fontScale}x`,
    );
    assert.equal(line.lineHeight, field.lineHeight);
    assert.equal(line.fontSize, field.fontSize);
    empty.harness.unmount();
    const typed = setup({ value: "Draft reply" }, "ios", fontScale);
    const typedField = inputStyle(typed.input());
    assert.equal(typedField.height, undefined, "typed text sizes the field");
    assert.equal(typedField.minHeight, composerInputMinHeight(fontScale));
    typed.harness.unmount();
  }
  // P9: retain the base native paragraph identity outside the Duo.
  const source = readFileSync(
    new URL("./AssistantMentionInput.tsx", import.meta.url),
    "utf8",
  );
  assert.doesNotMatch(source, /DynamicTypeText as Text/);
  assert.match(source, /import \{[^}]*\bText\b[^}]*\} from "react-native"/);
});

test("a whitespace-only draft collapses once unfocused, keeping its whitespace, cursor and one native input", () => {
  const layouts: any[] = [];
  const view = setup(
    {
      value: "",
      placeholder: "Message NyxBot",
      compactWhenIdle: true,
      renderActions: (mention: Element, layout: any) => {
        layouts.push(layout);
        return { type: "Toolbar", props: { children: [mention] } };
      },
    },
    "ios",
  );
  const compact = () => layouts.at(-1).compact;
  const prompt = () =>
    elements(view.harness.flush()).some(
      (n) => n.type === "Text" && n.props.children === "Message NyxBot",
    );
  const key = view.input().key;
  view.focus();
  view.input().props.onChangeText("  \n\n ");
  view.input().props.onSelectionChange({
    nativeEvent: { selection: { start: 3, end: 3 } },
  });
  view.harness.render();
  assert.equal(compact(), false, "focused: two rows");
  view.input().props.onBlur();
  view.harness.flush();
  assert.equal(compact(), true, "unfocused with only whitespace: one row");
  assert.equal(view.input().props.value, "  \n\n ", "the draft is not trimmed");
  assert.equal(
    view.writes.at(-1),
    "  \n\n ",
    "no write discards the whitespace",
  );
  // One line: the field is pinned and the prompt shows over it.
  assert.equal(inputStyle(view.input()).height, composerInputMinHeight(1));
  assert.ok(prompt());
  view.focus();
  assert.equal(compact(), false, "focus reopens two rows");
  assert.equal(view.input().key, key, "the same native input keeps focus");
  assert.equal(view.input().props.value, "  \n\n ");
  assert.equal(prompt(), false);
  // Any non-whitespace text keeps two rows when unfocused.
  view.input().props.onChangeText("  x ");
  view.input().props.onBlur();
  view.harness.flush();
  assert.equal(compact(), false);
  view.harness.unmount();
  // Attachments, uploads and dictation reach the input as compactWhenIdle
  // false: two rows even for an empty or whitespace draft.
  for (const value of ["", " \n"]) {
    const busy = setup({
      value,
      compactWhenIdle: false,
      renderActions: (mention: Element, layout: any) => {
        layouts.push(layout);
        return { type: "Toolbar", props: { children: [mention] } };
      },
    });
    assert.equal(layouts.at(-1).compact, false);
    busy.harness.unmount();
  }
});

test("the collapse rule reads trimmed text only while unfocused and idle", async () => {
  const { composerCompact } = await import("./composerCompact");
  assert.equal(composerCompact(true, false, ""), true);
  assert.equal(composerCompact(true, false, " \n\t "), true);
  assert.equal(composerCompact(true, true, " "), false, "focused");
  assert.equal(composerCompact(true, false, " a "), false, "text");
  assert.equal(
    composerCompact(false, false, " "),
    false,
    "attachments, uploads or dictation",
  );
});

test("an idle phone composer is one row until it is focused or holds a draft, keeping one native input", () => {
  const layouts: any[] = [];
  const view = setup(
    {
      value: "",
      placeholder: "Message NyxBot",
      compactWhenIdle: true,
      renderActions: (mention: Element, layout: any) => {
        layouts.push(layout);
        return { type: "Toolbar", props: { children: [mention] } };
      },
    },
    "ios",
  );
  const compact = () => layouts.at(-1).compact;
  const frame = () =>
    Object.assign(
      {},
      ...[
        elements(view.harness.flush()).find(
          (n) => n.props.testID === "chat-composer-input-frame",
        )!.props.style,
      ]
        .flat(Infinity)
        .filter(Boolean),
    );
  const placeholder = () =>
    Object.assign(
      {},
      ...[
        elements(view.harness.flush()).find(
          (n) => n.type === "Text" && n.props.children === "Message NyxBot",
        )!.props.style,
      ]
        .flat(Infinity)
        .filter(Boolean),
    );
  const key = view.input().key;
  assert.equal(compact(), true, "unfocused and empty");
  // Measured toolbar ends clear the text: [+] … [mic][send].
  layouts.at(-1).onLeadingWidth(44);
  layouts.at(-1).onTrailingWidth(132);
  view.harness.flush();
  assert.equal(frame().paddingLeft, 48);
  assert.equal(frame().paddingRight, 136);
  assert.equal(placeholder().left, 48);
  assert.equal(placeholder().right, 136);
  view.focus();
  assert.equal(compact(), false, "focus opens the two-row layout");
  assert.equal(frame().paddingLeft, undefined);
  assert.equal(frame().paddingHorizontal, 12);
  view.input().props.onBlur();
  view.harness.flush();
  assert.equal(compact(), true);
  view.input().props.onChangeText("Draft");
  view.harness.render();
  assert.equal(compact(), false, "a draft is never collapsed");
  view.input().props.onChangeText("");
  view.harness.render();
  assert.equal(compact(), true);
  assert.equal(view.input().key, key, "focus, cursor and text keep one input");
  view.harness.unmount();
  const optedOut = setup({
    value: "",
    compactWhenIdle: false,
    renderActions: (mention: Element, layout: any) => {
      layouts.push(layout);
      return { type: "Toolbar", props: { children: [mention] } };
    },
  });
  assert.equal(
    layouts.at(-1).compact,
    false,
    "an explicit active-layout override can keep two rows",
  );
  optedOut.harness.unmount();
});

for (const platform of ["ios", "android"] as const) {
  test(`${platform}: idle row expands on focus and preserves meaningful drafts and native input`, () => {
    let compact = false;
    const view = setup(
      {
        value: " ",
        compactWhenIdle: true,
        renderActions: (_mention: Element, layout: { compact: boolean }) => {
          compact = layout.compact;
          return { type: "Toolbar", props: {} };
        },
      },
      platform,
    );
    const key = view.input().key;
    assert.equal(compact, true);
    view.focus();
    assert.equal(compact, false);
    view.input().props.onChangeText("meaningful draft");
    view.input().props.onBlur();
    view.harness.render();
    assert.equal(compact, false);
    assert.equal(view.props.value, "meaningful draft");
    assert.equal(view.input().key, key);
    view.harness.unmount();
  });
}
