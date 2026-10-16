import assert from "node:assert/strict";
import { createRequire } from "node:module";
import test from "node:test";
import {
  assistantMarkdownOptions,
  safeAssistantLinkUrl,
} from "./assistantMarkdown";

// Exercise the exact parser version used by the native Markdown renderer.
const require = createRequire(import.meta.url);
const rendererRequire = createRequire(
  require.resolve("react-native-markdown-display"),
);
const MarkdownIt = rendererRequire("markdown-it");
const parser = MarkdownIt(assistantMarkdownOptions);
parser.validateLink = (url: string) => safeAssistantLinkUrl(url) !== null;

type Token = {
  type: string;
  attrGet: (name: string) => string | null;
  children?: Token[] | null;
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
    "mailto:person@example.com",
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
