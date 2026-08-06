import { describe, expect, it } from "vitest";
import { splitStableMarkdown } from "./markdown-stream";

/**
 * One paragraph — so it contributes no boundaries of its own — long enough to
 * clear the module's minimum-length and minimum-prefix guards on its own.
 */
const INTRO =
  "NyxID brokered the call with a scoped credential and never handed the raw " +
  "token to the agent. The node injected it at the edge, so the transcript " +
  "below records only metadata: which service, which scope, and how long the " +
  "hop took end to end. Nothing in this paragraph is a list marker, a table " +
  "row, a blockquote or an indented line, and it contains no footnote or link " +
  "reference, so it is exactly the shape the splitter is allowed to freeze " +
  "once the text that follows it has settled into its own block. It runs on " +
  "for a few more clauses purely so that the fixture clears the minimum " +
  "length on its own, without any test needing to pad it further.";

describe("splitStableMarkdown", () => {
  it("uses a fixture long enough to be eligible for splitting", () => {
    expect(INTRO.length).toBeGreaterThan(600);
  });

  it("splits before a line that provably starts a new top-level block", () => {
    const text = `${INTRO}\n\n## Result\n\nThe call succeeded.`;
    const { prefix, tail } = splitStableMarkdown(text);
    expect(prefix).toBe(`${INTRO}\n\n## Result\n\n`);
    expect(tail).toBe("The call succeeded.");
  });

  it.each([
    ["short text", "Too short to be worth splitting.\n\nSecond paragraph."],
    ["an ordered list", `${INTRO}\n\n1. First item\n\n2. Second item`],
    ["an unordered list", `${INTRO}\n\n- First item\n\n- Second item`],
    ["a blockquote", `${INTRO}\n\n> Quoted line\n\n> Continued`],
    ["a table row", `${INTRO}\n\n| a | b |\n| - | - |\n\n| 1 | 2 |`],
    ["an indented block", `${INTRO}\n\n    indented code\n\n    more code`],
    ["a footnote reference", `${INTRO}[^1]\n\nProse.\n\n[^1]: The note.`],
    [
      "a link reference definition",
      `${INTRO}\n\nSee [the docs][d].\n\n[d]: https://example.com`,
    ],
  ])("declines to split around %s", (_case, text) => {
    expect(splitStableMarkdown(text).prefix).toBe("");
  });

  it("never splits inside a fenced code block", () => {
    const text = `${INTRO}\n\n\`\`\`ts\nconst a = 1;\n\nconst b = 2;\n\`\`\`\n\nDone.`;
    const { prefix, tail } = splitStableMarkdown(text);
    // The blank line INSIDE the fence is not a boundary; the only safe split is
    // the settled text after the fence closed.
    expect(prefix).toBe(`${INTRO}\n\n\`\`\`ts\nconst a = 1;\n\nconst b = 2;\n\`\`\`\n\n`);
    expect(tail).toBe("Done.");
  });

  it("keeps an unterminated fence whole in the tail", () => {
    const text = `${INTRO}\n\n\`\`\`ts\nconst a = 1;\n\nconst b = 2;`;
    const { prefix, tail } = splitStableMarkdown(text);
    // Splitting AT the opening fence is safe — the tail still parses it as the
    // same incomplete fence — but nothing inside it may be frozen.
    expect(prefix).toBe(`${INTRO}\n\n`);
    expect(tail).toBe("```ts\nconst a = 1;\n\nconst b = 2;");
  });

  it("does not treat an indented tilde run inside a fence as a closer", () => {
    const text = `${INTRO}\n\n~~~\nplain\n\n\`\`\`\nnested-looking\n\n~~~\n\nAfter.`;
    const { prefix } = splitStableMarkdown(text);
    expect(prefix.endsWith("~~~\n\n")).toBe(true);
  });

  it("always partitions the input exactly", () => {
    const corpus = [
      "",
      INTRO,
      `${INTRO}\n\n## Result\n\nThe call succeeded.`,
      `${INTRO}\n\n1. one\n\n2. two`,
      `${INTRO}\n\n\`\`\`\ncode\n\`\`\`\n\nAfter.`,
      `${INTRO}\n\n\n\n\nExtra blank lines.`,
      `${INTRO}\n\nTrailing newline.\n`,
    ];
    for (const text of corpus) {
      const { prefix, tail } = splitStableMarkdown(text);
      expect(prefix + tail).toBe(text);
    }
  });

  it("advances the boundary monotonically as text streams in", () => {
    const full = `${INTRO}\n\n## Result\n\nThe call succeeded.\n\nAnd then some more prose arrived after it, which should move the boundary forward.`;
    let previous = 0;
    for (let length = 1; length <= full.length; length += 7) {
      const { prefix } = splitStableMarkdown(full.slice(0, length));
      expect(prefix.length).toBeGreaterThanOrEqual(previous);
      previous = prefix.length;
    }
  });
});
