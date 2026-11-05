import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname } from "node:path";
import test from "node:test";
import { URL as NodeURL } from "node:url";
import { runInNewContext } from "node:vm";
import ts from "typescript";
import { createLegalMarkdownStyles } from "./legalMarkdownStyles";
import { darkColors, lightColors, darkSheetColors, lightSheetColors } from "../../theme/mobileTheme";

type Element = { type: string; props: Record<string, any> };
const require = createRequire(import.meta.url);
const markdownRoot = dirname(require.resolve("react-native-markdown-display"));
const flatten = (value: any): Record<string, any> => Array.isArray(value)
  ? Object.assign({}, ...value.map(flatten)) : value ?? {};
const jsx = (type: string, props: Record<string, any>) => ({ type, props });
const cache = new Map<string, any>();

// Run the installed library's parser, style merger, AST renderer and rules.
// Replace only React/native primitives, so inheritance is exercised as shipped.
function load(filename: string): any {
  if (cache.has(filename)) return cache.get(filename);
  const exports = {};
  cache.set(filename, exports);
  const localRequire = createRequire(filename);
  const code = ts.transpileModule(readFileSync(filename, "utf8"), {
    fileName: `${filename}.tsx`,
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.ReactJSX, esModuleInterop: true },
  }).outputText;
  runInNewContext(code, {
    exports, console,
    require: (name: string) => {
      if (name === "react") return { memo: (fn: unknown) => fn, useMemo: (fn: () => unknown) => fn() };
      if (name === "react/jsx-runtime") return { jsx, jsxs: jsx };
      if (name === "react-native") return {
        Text: "Text", View: "View", TouchableWithoutFeedback: "TouchableWithoutFeedback",
        StyleSheet: { create: (styles: unknown) => styles, flatten },
        Platform: { select: (options: Record<string, unknown>) => options.ios ?? options.default },
      };
      if (name === "react-native-fit-image") return "Image";
      const path = localRequire.resolve(name);
      return path.startsWith(markdownRoot) ? load(path) : localRequire(name);
    },
  }, { filename });
  return exports;
}
const markdown = load(require.resolve("react-native-markdown-display"));

function luminance(hex: string) {
  assert.match(hex, /^#[\da-f]{6}$/i);
  return [1, 3, 5].map((offset) => parseInt(hex.slice(offset, offset + 2), 16) / 255)
    .map((c) => c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4)
    .reduce((sum, c, index) => sum + c * [0.2126, 0.7152, 0.0722][index]!, 0);
}
function contrast(a: string, b: string) {
  const values = [luminance(a), luminance(b)].sort((x, y) => x - y);
  return (values[1]! + 0.05) / (values[0]! + 0.05);
}
const documents = ["privacy", "terms"].map((key) => readFileSync(
  new NodeURL(`../../../../frontend/public/legal/${key}.md`, import.meta.url), "utf8",
).replace(/^---\n[\s\S]*?\n---\n*/, ""));
const allElements = `
# Heading 1
## Heading 2
### Heading 3
#### Heading 4
##### Heading 5
###### Heading 6

Paragraph with **strong**, *emphasis*, ~~deleted~~, [link](https://nyxid.ai), and \`inline code\`.
Two trailing spaces make a hard break.${"  "}
A soft
break.

- Bullet
  - Nested bullet with [link](https://nyxid.ai)

1. Ordered
2. Ordered **strong**

> Quote with a [link](https://nyxid.ai) and **strong**.
>
> - Quoted bullet

    Indented code

\`\`\`text
Fenced code
\`\`\`

| Header | Header |
| --- | --- |
| Cell | [table link](https://nyxid.ai) |

---

![Image](https://nyxid.ai/icon.png)
[![Image link](https://nyxid.ai/icon.png)](https://nyxid.ai)
`;

for (const [theme, colors] of Object.entries({ dark: darkColors, light: lightColors, darkSheet: darkSheetColors, lightSheet: lightSheetColors })) {
  test(`published policies and every supported markdown element render at AA (${theme})`, () => {
    const styles = createLegalMarkdownStyles(colors);
    let texts = 0;
    let links = 0;
    const visit = (node: Element | string | number | null, background: string, inherited: Record<string, any> = {}) => {
      if (node == null || typeof node === "boolean") return;
      if (typeof node === "string" || typeof node === "number") {
        if (!String(node).trim()) return;
        assert.equal(typeof inherited.color, "string", `Missing rendered text color: ${node}`);
        assert.ok(contrast(inherited.color, background) >= 4.5,
          `${theme}: ${String(node).slice(0, 40)} (${inherited.color} on ${background})`);
        texts++;
        return;
      }
      const style = flatten(node.props.style);
      const nextBackground = style.backgroundColor ?? background;
      const textStyle = node.type === "Text" ? { ...inherited, ...style } : {};
      if (node.type === "Text" && node.props.onPress) {
        assert.equal(style.color, colors.primary);
        assert.equal(style.textDecorationLine, "underline");
        links++;
      }
      for (const child of [node.props.children].flat(Infinity)) visit(child, nextBackground, textStyle);
    };
    for (const content of [...documents, allElements]) {
      visit(markdown.default({ children: content, style: styles }), colors.card);
    }
    assert.ok(texts > 100);
    assert.ok(links > 5);
  });
}
