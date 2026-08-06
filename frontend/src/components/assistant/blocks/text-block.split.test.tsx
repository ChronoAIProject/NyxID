import { cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { splitStableMarkdown } from "@/lib/assistant/markdown-stream";

/**
 * The streaming optimisation is only allowed to be faster, never different.
 * These render each document twice — once through the real splitter, once with
 * splitting stubbed out — and require the resulting DOM to be identical.
 *
 * A whole-text parse is the reference implementation, so any divergence here is
 * the optimisation changing meaning: an ordered list renumbering, a table
 * losing its header, a fence swallowing prose.
 */
async function renderTextBlock(
  text: string,
  { split }: { readonly split: boolean },
): Promise<string> {
  vi.resetModules();
  if (split) {
    vi.doUnmock("@/lib/assistant/markdown-stream");
  } else {
    vi.doMock("@/lib/assistant/markdown-stream", () => ({
      splitStableMarkdown: (value: string) => ({ prefix: "", tail: value }),
    }));
  }
  const { TextBlock } = await import("@/components/assistant/blocks/text-block");
  const { container } = render(<TextBlock text={text} />);
  const html = container.innerHTML;
  cleanup();
  return html;
}

/** One paragraph, long enough on its own to make each document splittable. */
const INTRO =
  "NyxID brokered the call with a scoped credential and never handed the raw " +
  "token to the agent. The node injected it at the edge, so the transcript " +
  "below records only metadata: which service, which scope, and how long the " +
  "hop took end to end. Nothing in this paragraph is a list marker, a table " +
  "row, a blockquote or an indented line, so it is exactly the shape the " +
  "splitter is allowed to freeze once what follows has settled into its own " +
  "block, which makes it a useful preamble for every case below here. It " +
  "runs on for a few more clauses purely so that the fixture clears the " +
  "splitter's minimum length without any document needing to pad it.";

const DOCUMENTS: ReadonlyArray<readonly [string, string]> = [
  ["headings and prose", `${INTRO}\n\n## Result\n\nThe call succeeded.`],
  ["an ordered list", `${INTRO}\n\n1. First item\n\n2. Second item\n\n3. Third`],
  ["a loose unordered list", `${INTRO}\n\n- alpha\n\n- beta\n\n- gamma`],
  ["a task list", `${INTRO}\n\n- [x] Brokered\n\n- [ ] Pending`],
  [
    "a table",
    `${INTRO}\n\n| Step | Status |\n| --- | --- |\n| resolve | ok |\n\nAfter the table.`,
  ],
  [
    "a closed code fence",
    `${INTRO}\n\n\`\`\`ts\nconst a = 1;\n\nconst b = 2;\n\`\`\`\n\nDone.`,
  ],
  ["an open code fence", `${INTRO}\n\n\`\`\`ts\nconst a = 1;\n\nconst b = 2;`],
  ["a blockquote", `${INTRO}\n\n> Quoted\n\n> Still quoted\n\nOut again.`],
  ["nested lists", `${INTRO}\n\n1. outer\n\n   - inner\n\n2. outer again`],
  [
    "a footnote",
    `${INTRO}\n\nSee the note.[^1]\n\nMore prose here.\n\n[^1]: The note body.`,
  ],
  [
    "a link reference definition",
    `${INTRO}\n\nSee [the docs][d].\n\nMore prose.\n\n[d]: https://example.com`,
  ],
  ["a horizontal rule", `${INTRO}\n\n---\n\nBelow the rule.`],
  ["inline emphasis across the seam", `${INTRO}\n\n**Bold** and \`code\`.`],
];

afterEach(() => {
  vi.doUnmock("@/lib/assistant/markdown-stream");
});

describe("TextBlock splitting", () => {
  it.each(DOCUMENTS)("renders %s identically split and unsplit", async (
    _case,
    text,
  ) => {
    // Sequential, never concurrent: each render swaps the module registry.
    const split = await renderTextBlock(text, { split: true });
    const whole = await renderTextBlock(text, { split: false });
    expect(split).toBe(whole);
  });

  it("actually exercises the split path across the corpus", () => {
    const splittable = DOCUMENTS.filter(
      ([, text]) => splitStableMarkdown(text).prefix !== "",
    );
    // Without this the equivalence above passes vacuously: two identical
    // whole-text parses always match. The declined cases (lists, footnotes,
    // reference links) are deliberately still in the corpus — they assert the
    // splitter's refusals, and their equivalence IS trivially true.
    expect(splittable.map(([name]) => name)).toContain("headings and prose");
    expect(splittable.length).toBeGreaterThanOrEqual(4);
  });

  it("renders every prefix of a streaming answer identically to a whole parse", async () => {
    const full = `${INTRO}\n\n## Result\n\nThe call succeeded and the token never left the node.\n\n- one\n\n- two`;
    for (let length = 620; length <= full.length; length += 37) {
      const text = full.slice(0, length);
      const split = await renderTextBlock(text, { split: true });
      const whole = await renderTextBlock(text, { split: false });
      expect(split, `diverged at ${String(length)} chars`).toBe(whole);
    }
  });
});
