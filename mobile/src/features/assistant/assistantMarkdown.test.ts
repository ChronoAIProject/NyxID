import assert from "node:assert/strict";
import { createRequire } from "node:module";
import test from "node:test";
import {
  assistantMarkdownOptions,
  configureAssistantMarkdown,
  safeAssistantLinkUrl,
} from "./assistantMarkdown";

// Exercise the exact parser version used by the native Markdown renderer.
const require = createRequire(import.meta.url);
const rendererRequire = createRequire(
  require.resolve("react-native-markdown-display"),
);
const MarkdownIt = rendererRequire("markdown-it");
const parser = configureAssistantMarkdown(MarkdownIt(assistantMarkdownOptions));

type Token = {
  type: string;
  attrGet: (name: string) => string | null;
  children?: Token[] | null;
  content: string;
};

function tokens(markdown: string): Token[] {
  return parser
    .parse(markdown, {})
    .flatMap((token: Token) => [token, ...(token.children ?? [])]);
}

function links(markdown: string): (string | null)[] {
  return tokens(markdown)
    .filter((token) => token.type === "link_open")
    .map((token) => token.attrGet("href"));
}

test("a bare setup URL in a Chinese response becomes a link with its full query", () => {
  const url =
    "https://example.com/assistant/machines/new?setup=rn-link-qa-example&mode=browser#connect";
  assert.deepEqual(
    links(`接入链接已准备好：\n${url}\n\n打开后按页面提示继续。`),
    [url],
  );
});

test("normal URL punctuation is outside the link and balanced path parentheses remain", () => {
  assert.deepEqual(
    links(
      "Visit https://example.com/help. Or (https://example.com/topic_(guide)).",
    ),
    ["https://example.com/help", "https://example.com/topic_(guide)"],
  );
});

test("multiple bare links retain encoded query parameters", () => {
  assert.deepEqual(
    links(
      "https://example.com/?q=%E4%BD%A0%E5%A5%BD&next=%2Fsettings\nhttps://docs.example.com/guide",
    ),
    [
      "https://example.com/?q=%E4%BD%A0%E5%A5%BD&next=%2Fsettings",
      "https://docs.example.com/guide",
    ],
  );
});

test("existing labeled and angle-bracket Markdown links still parse once", () => {
  assert.deepEqual(
    links(
      "[打开设置](https://example.com/settings) <https://example.com/help>",
    ),
    ["https://example.com/settings", "https://example.com/help"],
  );
});

test("inline and fenced code URLs remain code rather than tappable links", () => {
  const markdown =
    "`https://example.com/inline`\n\n```text\nhttps://example.com/fenced\n```";
  assert.deepEqual(links(markdown), []);
  assert.ok(tokens(markdown).some((token) => token.type === "code_inline"));
  assert.ok(tokens(markdown).some((token) => token.type === "fence"));
});

test("unsupported and credential-bearing destinations never become links", () => {
  for (const url of [
    "http://example.com/",
    "ftp://example.com/",
    "javascript:alert(1)",
    "data:text/html,example",
    "file:///tmp/example",
    "/settings",
    "https://user:password@example.com/",
    "https://user@example.com/",
  ]) {
    assert.equal(safeAssistantLinkUrl(url), null, url);
    assert.deepEqual(links(`[destination](${url})`), [], url);
    assert.deepEqual(links(url), [], url);
  }
});

test("GFM mail and local footnote links retain their destinations", () => {
  assert.deepEqual(links("[Email](mailto:review@example.test) [Note](#fn1)"), [
    "mailto:review@example.test",
    "#fn1",
  ]);
});

test("GFM task states are native read-only tokens, nested lists and code stay intact", () => {
  const parsed = tokens(
    "- [x] Done\n- [ ] Waiting\n  - [X] Nested\n\n`- [x] code`\n\n[ ] outside a list",
  );
  assert.deepEqual(
    parsed
      .filter((t) => t.type === "list_item_open")
      .map((t) => t.attrGet("data-task-checked")),
    ["true", "false", "true"],
  );
  assert.ok(
    parsed.some((t) => t.type === "code_inline" && t.content === "- [x] code"),
  );
  assert.ok(
    parsed.some((t) => t.type === "text" && t.content === "[ ] outside a list"),
  );
  assert.equal(
    parsed.some((t) => t.type.startsWith("html_")),
    false,
  );
});

test("escaped task markers remain ordinary list text", () => {
  const parsed = tokens("- \\[x] escaped\n- `[x]` code");
  assert.deepEqual(
    parsed
      .filter((t) => t.type === "list_item_open")
      .map((t) => t.attrGet("data-task-checked")),
    [null, null],
  );
});

test("footnote references, repeated back links and multiline definitions survive the native parser", () => {
  const parsed = tokens(
    "Read[^source] and again[^source].\n\n[^source]: First **detail**.\n\n    Second paragraph.",
  );
  assert.deepEqual(
    parsed
      .filter((t) => t.type === "footnote_ref")
      .map((t) => [t.attrGet("href"), t.attrGet("id")]),
    [
      ["#fn1", "fnref1"],
      ["#fn1", "fnref1:1"],
    ],
  );
  assert.ok(parsed.some((t) => t.type === "footnote_block_open"));
  assert.ok(
    parsed.some((t) => t.type === "footnote_open" && t.attrGet("id") === "fn1"),
  );
  assert.ok(
    parsed.some((t) => t.type === "text" && t.content === "Second paragraph."),
  );
  assert.deepEqual(
    parsed
      .filter((t) => t.type === "footnote_anchor")
      .map((t) => t.attrGet("href")),
    ["#fnref1", "#fnref1:1"],
  );
});

test("GFM tables keep alignment and strikethrough without consuming raw HTML", () => {
  const parsed = tokens(
    "| Left | Right |\n| :-- | --: |\n| ~~Old~~ | **New** |\n\n<script>unsafe()</script>",
  );
  assert.ok(parsed.some((t) => t.type === "table_open"));
  assert.deepEqual(
    parsed.filter((t) => t.type === "th_open").map((t) => t.attrGet("style")),
    ["text-align:left", "text-align:right"],
  );
  assert.ok(parsed.some((t) => t.type === "s_open"));
  assert.equal(
    parsed.some((t) => t.type.startsWith("html_")),
    false,
  );
});

test("model images nested inside links flatten to their label; unsafe images remain literal labels", () => {
  const parsed = tokens(
    "[![Preview](https://tracker.test/pixel)](https://docs.test)\n\n![Private](file:///secret)\n\n![Public](https://tracker.test/remote)",
  );
  assert.deepEqual(
    parsed.filter((t) => t.type === "image").map((t) => t.attrGet("src")),
    ["file:///secret", "https://tracker.test/remote"],
  );
  assert.ok(parsed.some((t) => t.type === "text" && t.content === "Preview"));
  assert.deepEqual(
    links("[![Preview](https://tracker.test/pixel)](https://docs.test)"),
    ["https://docs.test"],
  );
});

test("HTML tags stay literal rather than parsed as HTML", () => {
  const parsed = tokens('<a href="https://example.com/">open</a>');
  assert.equal(
    parsed.some((token) => token.type.startsWith("html_")),
    false,
  );
});

test("the opener preserves safe HTTPS query and fragment, including uppercase schemes", () => {
  assert.equal(
    safeAssistantLinkUrl(
      "HTTPS://EXAMPLE.COM/path?setup=rn-link-qa-example#next",
    ),
    "https://example.com/path?setup=rn-link-qa-example#next",
  );
  assert.equal(safeAssistantLinkUrl("not a URL"), null);
  assert.equal(safeAssistantLinkUrl("https://"), null);
});
