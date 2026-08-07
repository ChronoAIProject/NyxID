import { describe, expect, it } from "vitest";
import { safeRevealEnd } from "./reveal-boundary";

/** What the reader actually sees for a given reveal length. */
function revealed(text: string, desired: number): string {
  return text.slice(0, safeRevealEnd(text, desired));
}

describe("safeRevealEnd", () => {
  it("cuts exactly where asked when the boundary is already safe", () => {
    const text = "the quick brown fox";
    expect(revealed(text, 10)).toBe("the quick ");
  });

  it("never cuts past the end, and never before the start", () => {
    const text = "short";
    expect(safeRevealEnd(text, 99)).toBe(5);
    expect(safeRevealEnd(text, 0)).toBe(0);
    expect(safeRevealEnd(text, -3)).toBe(0);
  });

  it("only ever moves the cut earlier", () => {
    const text = "**bold** `code` [link](https://example.com) and 👩‍💻 text";
    for (let desired = 0; desired <= text.length; desired += 1) {
      const end = safeRevealEnd(text, desired);
      expect(end).toBeLessThanOrEqual(desired);
      expect(end).toBeGreaterThanOrEqual(0);
      // Whatever it returns is a real prefix of the answer.
      expect(text.startsWith(text.slice(0, end))).toBe(true);
    }
  });

  describe("grapheme safety", () => {
    it("does not split a surrogate pair", () => {
      const text = "ok 🎉 done";
      // Offset 4 is between the two code units of the emoji.
      expect(revealed(text, 4)).toBe("ok ");
    });

    it("does not split a ZWJ sequence", () => {
      const text = "Hi 👩‍💻!";
      // The whole emoji is one cluster spanning offsets 3..8.
      expect(revealed(text, 5)).toBe("Hi ");
      expect(revealed(text, 8)).toBe("Hi 👩‍💻");
    });

    it("does not separate a combining mark from its base", () => {
      const text = "café bar";
      expect(revealed(text, 4)).toBe("caf");
      expect(revealed(text, 5)).toBe("café");
    });

    it("keeps Unicode context beyond a fixed look-behind window", () => {
      const text = `a${"\u0301".repeat(100)} done`;
      expect(revealed(text, 80)).toBe("");
    });

    it("does not split a skin-tone modifier", () => {
      const text = "wave 👋🏽 now";
      expect(revealed(text, 7)).toBe("wave ");
    });
  });

  describe("inline Markdown safety", () => {
    it("withholds an emphasis run that has not closed yet", () => {
      const text = "make it **bol";
      expect(revealed(text, text.length)).toBe("make it ");
    });

    it("reveals emphasis once it closes", () => {
      const text = "make it **bold** now";
      expect(revealed(text, text.length)).toBe(text);
    });

    it("does not let one closer marker close a two-marker run", () => {
      const text = "make it **bold*";
      expect(revealed(text, text.length)).toBe("make it ");
    });

    it("does not inspect a delimiter run past the desired head", () => {
      const text = "make it **bold** now";
      const afterFirstCloser = "make it **bold*".length;
      expect(revealed(text, afterFirstCloser)).toBe("make it ");
    });

    it.each(["2 * 3", "file_name", "about ~10 ms"])(
      "does not hold non-delimiter prose: %s",
      (text) => {
        expect(revealed(text, text.length)).toBe(text);
      },
    );

    it("withholds an unterminated code span", () => {
      const text = "run `npm insta";
      expect(revealed(text, text.length)).toBe("run ");
    });

    it("withholds a link whose destination is still streaming", () => {
      const text = "see [the docs](https://ex";
      expect(revealed(text, text.length)).toBe("see ");
    });

    it("reveals a link once its destination closes", () => {
      const text = "see [the docs](https://e.com) now";
      expect(revealed(text, text.length)).toBe(text);
    });

    it("keeps a head-ending label provisional for a possible destination", () => {
      expect(revealed("see [the docs]", 14)).toBe("see ");
      expect(revealed("see [the docs] next", 19)).toBe("see [the docs] next");
    });

    it("does not close a destination on a nested parenthesis", () => {
      const partial = "see [docs](https://example.test/a_(b)";
      expect(revealed(partial, partial.length)).toBe("see ");
      const complete = `${partial}) now`;
      expect(revealed(complete, complete.length)).toBe(complete);
    });

    it("treats a list bullet as a bullet, not as emphasis", () => {
      const text = "* item one";
      expect(revealed(text, text.length)).toBe(text);
    });

    it("ignores markers inside a closed code span", () => {
      const text = "use `a * b` now";
      expect(revealed(text, text.length)).toBe(text);
    });

    it("ignores escaped markers", () => {
      const text = "5 \\* 3 equals";
      expect(revealed(text, text.length)).toBe(text);
    });

    it("rides a bounded distance behind an opener that never closes", () => {
      const text = `a**${"b".repeat(200)}`;
      // Past the holdback bound the hold is clamped rather than dropped: a
      // stalled answer is worse than a stray asterisk, but so is freezing for
      // 96 characters and then painting all 96 in one frame.
      expect(text.length - safeRevealEnd(text, text.length)).toBe(96);
      // ...and it keeps MOVING while it does so — one arrived unit in, one
      // revealed unit out, never a jump.
      for (let desired = 100; desired <= text.length; desired += 1) {
        expect(safeRevealEnd(text, desired) - safeRevealEnd(text, desired - 1)).toBe(1);
      }
    });

    it("never holds a single-marker run, which is usually literal prose", () => {
      for (const text of [
        "Use *.ts to match TypeScript files.",
        "The cost is 5*3 dollars in total.",
        "Run SELECT * FROM users to see them.",
        "Set snake_case names for the fields.",
      ]) {
        expect(revealed(text, text.length)).toBe(text);
      }
    });

    it("holds a two-marker run that ends exactly at the head", () => {
      // The common chunk boundary: `**` is a single model token, so an arrival
      // routinely ends right after it. Flanking is undecidable there, so the
      // run must stay provisional rather than paint as literal asterisks.
      expect(revealed("make it **", 10)).toBe("make it ");
      expect(revealed("a ~~", 4)).toBe("a ");
      // A genuine closer is still decidable from its left context and closes.
      expect(revealed("make it **bold**", 16)).toBe("make it **bold**");
    });
  });

  describe("fenced code blocks", () => {
    it("does not treat a fence opener as an unterminated code span", () => {
      const text = "Here is code:\n\n```ts\nconst x = 1;\nconst y = 2;\n";
      // Every offset inside the open fence reveals as it arrives: the content
      // is literal code, so there is no inline construct to protect.
      for (const desired of [21, 30, 40, text.length]) {
        expect(safeRevealEnd(text, desired)).toBe(desired);
      }
    });

    it("resumes inline safety after the fence closes", () => {
      const text = "```\ncode *here*\n```\nthen **bol";
      expect(revealed(text, text.length)).toBe("```\ncode *here*\n```\nthen ");
    });

    it("still holds a real inline code span", () => {
      const text = "run ``npm insta";
      expect(revealed(text, text.length)).toBe("run ");
    });
  });

  describe("word safety", () => {
    it("snaps a cut that lands inside a word back to its start", () => {
      const text = "the quick brown fox";
      expect(revealed(text, 13)).toBe("the quick ");
    });

    it("does not snap when the cut already follows a space", () => {
      const text = "the quick brown fox";
      expect(revealed(text, 4)).toBe("the ");
    });

    it("reveals a long token progressively rather than withholding it", () => {
      const text = `start ${"x".repeat(60)} end`;
      // Beyond the word bound the token streams as it arrives.
      expect(safeRevealEnd(text, 40)).toBe(40);
    });

    it("does not snap in scripts written without spaces", () => {
      const text = "这是一个测试";
      expect(safeRevealEnd(text, 3)).toBe(3);
    });
  });

  it("holds nothing back for plain prose", () => {
    const text = "A perfectly ordinary sentence with no markup at all.";
    for (let desired = 0; desired <= text.length; desired += 1) {
      // Only word snapping may apply, and never by more than a word.
      expect(desired - safeRevealEnd(text, desired)).toBeLessThan(24);
    }
  });
});
