import { expect, it } from "vitest";
import { unified } from "unified";
import remarkParse from "remark-parse";
import remarkGfm from "remark-gfm";
// @ts-expect-error Build-only module.
import { semanticMarkdown } from "../scripts/public-content.mjs";
it("retains table columns, code whitespace, links, images and ordered list meaning", () => {
  const md = semanticMarkdown(
    '<main><h1>Approved guide</h1><p>An <code>API key</code> example with a <a href="/docs">reference</a>.</p><table><tr><th>Field</th><th>Type</th></tr><tr><td>name</td><td>string</td></tr></table><pre><code>if (ready) {\n  run();\n}</code></pre><ol><li>Connect</li><li>Approve</li></ol><img alt="Credential boundary" src="/diagram.svg"><p hidden>private panel</p></main>',
    { path: "/docs/example", locale: "en", title: "Approved guide" },
    "revision",
    "https://nyx.chrono-ai.fun",
  );
  expect(md).toContain("| Field | Type |");
  expect(md).toContain("| name | string |");
  expect(md).toContain("  run();");
  expect(md).toContain("1. Connect");
  expect(md).toContain("2. Approve");
  expect(md).toContain("![Credential boundary]");
  expect(md).toContain("[reference](<https://nyx.chrono-ai.fun/docs>)");
  expect(md).not.toContain("private panel");
});

it("round-trips backslashes and pipes in table text and inline code", () => {
  const md = semanticMarkdown(
    String.raw`<main><h1>Table escapes</h1><table><tr><th>Plain</th><th>Code</th></tr><tr><td>C:\tools|work</td><td><code>a\b|c</code></td></tr></table></main>`,
    { path: "/docs/example", locale: "en", title: "Table escapes" }, "revision", "https://nyx.chrono-ai.fun",
  );
  const tree = unified().use(remarkParse).use(remarkGfm).parse(md);
  const table = tree.children.find(node => node.type === "table");
  if (table?.type !== "table") throw new Error("Missing semantic table");
  expect(table.children[1]!.children.map(cell => cell.children.map(node => "value" in node ? node.value : "").join("")))
    .toEqual([String.raw`C:\tools|work`, String.raw`a\b|c`]);
});
