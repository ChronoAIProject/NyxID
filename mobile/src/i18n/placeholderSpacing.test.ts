import assert from "node:assert/strict";
import test from "node:test";
import cn from "./locales/zh-CN";
import tw from "./locales/zh-TW";

/*
 * English source phrases (the catalog keys) keep a space between a
 * placeholder and an adjacent word: "{{count}} memories" and "Tool: {{name}}",
 * never "{{count}}memories" or "Tool:{{name}}". Converting JSX text such as `{n} memories` to a phrase
 * once dropped those spaces. Only compact unit forms and values that carry
 * their own leading separator may touch a word.
 */
const COMPACT = new Set([
  "{{minutes}}m ago",
  "{{hours}}h ago",
  "{{days}}d ago",
  "{{days}}d left",
  "in {{minutes}}m",
  "in {{hours}}h",
  "in {{days}}d",
  "{{seconds}}s",
  "{{date}} ({{minutes}}s)",
  "{{amount}}pt",
  "{{min}}–{{max}}pt",
  "Use latest (v{{version}})",
  "{{width}} x {{height}} CSS px at {{scale}}x",
  // The value is "" or " plus 0.1 per start…", spacing included.
  "{{credits}} credits per second{{totalCost}}.",
]);

test("English phrases never glue a placeholder to a word", () => {
  for (const [name, catalog] of [
    ["zh-CN", cn],
    ["zh-TW", tw],
  ] as const) {
    const glued = Object.keys(catalog).filter(
      (key) =>
        /\}\}[A-Za-z]|[A-Za-z]:?\{\{/.test(key) && !COMPACT.has(key),
    );
    assert.deepEqual(glued, [], `${name} keys with a missing space`);
  }
});
