import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { hookHarness } from "./assistantHookTestHarness";
import { createRequire } from "node:module";
import { darkColors } from "../../theme/mobileTheme";

const require = createRequire(import.meta.url);
const parserRequire = createRequire(
  require.resolve("react-native-markdown-display"),
);
const MarkdownIt = parserRequire("markdown-it");

test("private and crew message bodies are memoized and keep parser props stable across parent updates", () => {
  let colors = { textPrimary: "black", primaryOnTint: "purple" };
  const h = hookHarness<any>({
    "react/jsx-runtime": { jsx: (type: any, props: any) => ({ type, props }) },
    "react-native": { Text: "Text" },
    "react-native-markdown-display": {
      __esModule: true,
      default: "Markdown",
      MarkdownIt,
      renderRules: {},
    },
    "expo-web-browser": {},
    "../../theme/ThemeContext": { useTheme: () => ({ colors }) },
    "./assistantSelectableMarkdown": {
      assistantSelectableMarkdown: { textgroup: () => "selectable" },
    },
    "./AssistantConnectButton": {
      AssistantConnectButton: "AssistantConnectButton",
    },
  });
  const Body = h.load(
    fileURLToPath(new URL("./AssistantMarkdownBody.tsx", import.meta.url)),
  ).AssistantMarkdownBody as any;
  assert.equal(Body.$$typeof, Symbol.for("react.memo"));
  assert.equal(
    Body.compare,
    null,
    "React shallow comparison includes text, variant and the link handler",
  );
  for (const variant of ["private", "crew"]) {
    let text = "Saved reply";
    h.mount(() => Body.type({ text, variant }));
    const first = h.value();
    h.render();
    assert.equal(h.value().props.style, first.props.style);
    assert.equal(h.value().props.rules, first.props.rules);
    assert.equal(h.value().props.markdownit, first.props.markdownit);
    assert.equal(h.value().props.onLinkPress, first.props.onLinkPress);
    assert.equal(
      h.value().props.rules.textgroup({ key: "body" }, [], [], {}),
      "selectable",
    );
    text = "Updated stream";
    h.render();
    assert.equal(h.value().props.children, text);
    colors = { textPrimary: "white", primaryOnTint: "violet" };
    h.render();
    assert.equal(h.value().props.style.body.color, "white");
    assert.notEqual(h.value().props.style, first.props.style);
    h.unmount();
    colors = { textPrimary: "black", primaryOnTint: "purple" };
  }
});

test("private and crew GFM render accessible tasks, scrollable aligned tables, safe image links and local footnotes", async () => {
  const opened: string[] = [];
  const focused: number[] = [];
  const emails: string[] = [];
  const h = hookHarness<any>({
    "react/jsx-runtime": {
      jsx: (type: any, props: any) => ({ type, props }),
      jsxs: (type: any, props: any) => ({ type, props }),
    },
    "react-native": {
      Text: "Text",
      View: "View",
      ScrollView: "ScrollView",
      AccessibilityInfo: {
        setAccessibilityFocus: (handle: number) => focused.push(handle),
      },
      findNodeHandle: (target: { handle: number }) => target.handle,
      Linking: {
        openURL: async (url: string) => {
          emails.push(url);
        },
      },
    },
    "react-native-markdown-display": {
      __esModule: true,
      default: "Markdown",
      MarkdownIt,
      renderRules: { list_item: () => "ordinary list item" },
    },
    "expo-web-browser": {
      openBrowserAsync: async (url: string) => opened.push(url),
    },
    "../../theme/ThemeContext": { useTheme: () => ({ colors: darkColors }) },
    "./assistantSelectableMarkdown": {
      assistantSelectableMarkdown: { textgroup: () => "selectable" },
    },
    "./AssistantConnectButton": {
      AssistantConnectButton: "AssistantConnectButton",
    },
  });
  const Body = h.load(
    fileURLToPath(new URL("./AssistantMarkdownBody.tsx", import.meta.url)),
  ).AssistantMarkdownBody as any;
  const node = (
    attributes: Record<string, any> = {},
    children: any[] = [],
  ) => ({ key: "node", attributes, children });
  for (const variant of ["private", "crew"]) {
    h.mount(() => Body.type({ text: "GFM", variant }));
    const { rules } = h.value().props;
    for (const checked of [true, false]) {
      const task = rules.list_item(
        node({ "data-task-checked": String(checked) }),
        ["Task text"],
      );
      const checkbox = task.props.children[0];
      assert.equal(checkbox.props.accessibilityRole, "checkbox");
      assert.equal(checkbox.props.accessibilityState.checked, checked);
      assert.equal(checkbox.props.accessibilityState.disabled, true);
      assert.equal(checkbox.props.onPress, undefined);
    }
    assert.equal(rules.list_item(node(), []), "ordinary list item");
    const types = [
      "heading1",
      "paragraph",
      "fence",
      "table",
      "heading2",
      "paragraph",
      "blockquote",
      "bullet_list",
      "image",
    ];
    const body = rules.body(
      {
        ...node(),
        children: types.map((type, i) => ({ type, key: String(i) })),
      },
      types,
      [],
      { _VIEW_SAFE_body: { color: "white" } },
    );
    assert.deepEqual(
      body.props.children.map((child: any) => child.props.style.marginTop),
      [0, 8, 12, 12, 16, 8, 12, 12, 12],
    );
    assert.equal(
      body.props.children.at(-1).props.style.marginBottom,
      undefined,
    );
    assert.equal(h.value().props.style.paragraph.marginBottom, 0);
    const table = rules.table(
      node({}, [{ type: "thead", children: [{ children: [1, 2, 3] }] }]),
      ["cells"],
    );
    assert.equal(table.type, "ScrollView");
    assert.equal(table.props.horizontal, true);
    assert.equal(table.props.contentContainerStyle.paddingRight, 12);
    assert.equal(table.props.style.marginBottom, undefined);
    assert.equal(table.props.children.props.style.minWidth, 420);
    const cell = rules.textgroup(
      node(),
      ["right"],
      [{ type: "td", attributes: { style: "text-align:right" } }],
      { textgroup: {} },
    );
    assert.equal(cell.props.selectable, true);
    assert.equal(cell.props.style[1].textAlign, "right");
    const unsafeImage = rules.image(
      node({ src: "file:///secret", alt: "Private" }),
    );
    assert.equal(
      unsafeImage.type,
      "Text",
      "model image creates no native Image or request",
    );
    assert.equal(unsafeImage.props.children, "Private");
    assert.equal(unsafeImage.props.onPress, undefined);
    const image = rules.image(
      node({ src: "https://example.test/chart", alt: "Chart" }),
    );
    image.props.onPress();
    assert.equal(opened.at(-1), "https://example.test/chart");
    const mail = rules.link(node({ href: "mailto:review@example.test" }), [
      "Email",
    ]);
    mail.props.onPress();
    assert.equal(emails.at(-1), "mailto:review@example.test");
    const section = rules.footnote_block(node(), ["definitions"]);
    assert.equal(section.props.children[0].props.children, "Footnotes");
    const definition = rules.footnote(node({ id: "fn1", "data-number": "1" }), [
      "Note",
    ]);
    const heading = definition.props.children[0];
    heading.props.ref({ handle: 81 });
    const reference = rules.footnote_ref(
      node({ id: "fnref1", href: "#fn1", "data-number": "1" }),
    );
    reference.props.ref({ handle: 82 });
    reference.props.onPress();
    await new Promise((resolve) => setTimeout(resolve, 10));
    await h.settle();
    assert.equal(focused.at(-1), 81);
    const back = rules.footnote_anchor(
      node({ href: "#fnref1", "data-number": "1" }),
    );
    back.props.onPress();
    await new Promise((resolve) => setTimeout(resolve, 10));
    await h.settle();
    assert.equal(focused.at(-1), 82);
    assert.equal(
      opened.some((url) => url.startsWith("#")),
      false,
    );
    heading.props.ref(null);
    reference.props.ref(null);
    h.unmount();
  }
});

test("hosted connect links render one button under their block and keep plain words inline", () => {
  const origin = "https://app.example.test";
  const href = `${origin}/connect/nyx_clk_synthetic_review`;
  const opened: { url: string; trigger?: number }[] = [];
  const h = hookHarness<any>({
    "react/jsx-runtime": {
      jsx: (type: any, props: any) => ({ type, props }),
      jsxs: (type: any, props: any) => ({ type, props }),
    },
    "react-native": { Text: "Text", View: "View", ScrollView: "ScrollView" },
    "react-native-markdown-display": {
      __esModule: true,
      default: "Markdown",
      MarkdownIt,
      renderRules: {},
    },
    "expo-web-browser": {},
    "../../theme/ThemeContext": { useTheme: () => ({ colors: darkColors }) },
    "./assistantSelectableMarkdown": { assistantSelectableMarkdown: {} },
    "./AssistantConnectButton": {
      AssistantConnectButton: "AssistantConnectButton",
    },
    "./AssistantLinkContext": {
      AssistantLinkContext: {
        value: (url: string, trigger?: number) => {
          opened.push({ url, trigger });
          return true;
        },
      },
    },
    "./assistantMarkdown": {
      assistantMarkdownOptions: {},
      configureAssistantMarkdown: (parser: unknown) => parser,
      safeAssistantLinkUrl: (url: string) => url,
    },
    "../../lib/env": { firstPartyFrontendUrls: () => [origin] },
  });
  const Body = h.load(
    fileURLToPath(new URL("./AssistantMarkdownBody.tsx", import.meta.url)),
  ).AssistantMarkdownBody as any;
  h.mount(() => Body.type({ text: "connect", variant: "private" }));
  const { rules } = h.value().props;
  const link = (target: string, text: string) => ({
    type: "link",
    attributes: { href: target },
    children: [{ type: "text", content: text }],
  });
  const paragraph = (key: string, children: any[]) => ({
    type: "paragraph",
    key,
    children: [{ type: "textgroup", children }],
  });
  const ast = {
    key: "body",
    attributes: {},
    children: [
      paragraph("a", [
        { type: "text", content: "Your mailbox needs connecting. " },
        link(href, "Connect your email securely"),
      ]),
      paragraph("b", [link("https://example.test/docs", "docs")]),
      paragraph("c", [link(href, href)]),
      paragraph("d", [
        link(
          `${origin}/keys?tab=services&action=add-service&slug=api-github`,
          "Connect another GitHub account",
        ),
      ]),
    ],
  };
  const body = rules.body(ast, ["A", "B", "C", "D"], [], {
    _VIEW_SAFE_body: {},
  });
  const blocks = body.props.children;
  const buttons = (block: any) =>
    block.props.children[1].filter(
      (child: any) => child.type === "AssistantConnectButton",
    );
  assert.equal(blocks[0].props.children[0], "A");
  assert.equal(
    buttons(blocks[0])
      .map((button: any) => button.props.label)
      .join("|"),
    "Connect your email securely",
  );
  assert.equal(buttons(blocks[1]).length, 0, "ordinary links stay links");
  assert.equal(buttons(blocks[2]).length, 0, "one button per connect link");
  assert.equal(
    buttons(blocks[3])
      .map((button: any) => button.props.label)
      .join("|"),
    "Connect another GitHub account",
    "add-service links are connector buttons too",
  );
  assert.equal(buttons(blocks[0])[0].props.href, href, "the button follows its link's live state");
  buttons(blocks[0])[0].props.onPress(42);
  assert.equal(
    JSON.stringify(opened),
    JSON.stringify([{ url: href, trigger: 42 }]),
  );

  const inline = rules.link(
    { key: "l", attributes: { href }, children: [{ content: href }] },
    [href],
  );
  assert.equal(inline, null, "a bare connect URL would only repeat its button");
  const named = rules.link(
    { key: "n", attributes: { href }, children: [{ content: "GitHub" }] },
    ["GitHub"],
  );
  assert.equal(named.type, "Text");
  assert.equal(named.props.children, "Connect GitHub");
  assert.equal(named.props.onPress, undefined);
  assert.equal(named.props.accessibilityRole, undefined);
  const ordinary = rules.link(
    {
      key: "o",
      attributes: { href: "https://example.test/docs" },
      children: [{ content: "docs" }],
    },
    ["docs"],
  );
  assert.equal(ordinary.props.accessibilityRole, "link");
  ordinary.props.onPress({ nativeEvent: { target: 7 } });
  assert.equal(opened.at(-1)!.url, "https://example.test/docs");
  h.unmount();
});
