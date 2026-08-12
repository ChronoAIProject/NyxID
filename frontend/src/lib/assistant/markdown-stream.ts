/**
 * Splitting streamed Markdown into a settled prefix and a live tail.
 *
 * A streaming text block re-parses its ENTIRE accumulated text on every paint,
 * and that parse is linear in length — so a long answer writes itself more and
 * more slowly, which is what reads as choppiness. Everything above the last
 * completed block, though, can no longer be reinterpreted by characters that
 * have not arrived yet: it can be parsed once and memoized by string identity,
 * leaving only a short tail to re-parse per frame.
 *
 * "Can no longer be reinterpreted" is the entire difficulty, and this module is
 * deliberately conservative about it. A split is only offered where the
 * boundary is provably inert; anywhere it cannot prove that, it declines and
 * returns the whole text as the tail — which is exactly the un-split behaviour,
 * so declining costs speed and never correctness.
 */

export interface StableMarkdownSplit {
  /** Parsed once and memoized. Empty when no safe boundary exists. */
  readonly prefix: string;
  /** Re-parsed per frame. The whole text when there is no safe boundary. */
  readonly tail: string;
}

/**
 * A line that the construct above it could still absorb. Splitting immediately
 * before one of these would re-parse it as a NEW construct: an ordered list
 * would restart its numbering at 1, a table row would lose its header, and an
 * indented line would lose its nesting. Blank-line separation is not enough to
 * rule this out, because loose lists carry blank lines between their items.
 */
const ABSORBABLE_LINE = /^(?:[ \t]|[-*+>|]|\d+[.)])/;

/**
 * Constructs whose meaning depends on text that arrives LATER: footnote
 * references and link reference definitions both resolve against definitions
 * collected from the whole document. A prefix frozen before its definitions
 * stream in would render them as literal text permanently, so a text carrying
 * either never splits. Both patterns are matched loosely on purpose — a false
 * positive only forgoes the optimisation.
 */
const RESOLVES_LATER = /\[\^|^ {0,3}\[[^\]\n]+\]:/m;

const FENCE = /^ {0,3}(`{3,}|~{3,})/;

/**
 * Below this there is nothing to win: the whole-text parse is already cheap and
 * splitting would just pay two parser start-ups instead of one.
 */
const MIN_SPLIT_CHARS = 600;

/** A prefix smaller than this saves less than the second parse costs. */
const MIN_PREFIX_CHARS = 200;

interface OpenFence {
  readonly char: string;
  readonly length: number;
}

/** CommonMark: same character, at least as long, and no trailing info string. */
function closesFence(line: string, open: OpenFence): boolean {
  const match = FENCE.exec(line);
  const run = match?.[1];
  if (match === null || run === undefined) return false;
  return (
    run.startsWith(open.char) &&
    run.length >= open.length &&
    line.slice(match[0].length).trim() === ""
  );
}

/**
 * The last offset in `text` that provably begins a new top-level block, or -1.
 *
 * Single pass, because this runs on every frame of a live stream: fence state
 * and boundary tracking advance together rather than rescanning the text once
 * per candidate.
 *
 * An unterminated trailing fence does NOT invalidate earlier boundaries. Every
 * boundary is recorded while outside a fence, and the fence's own opening line
 * is itself a boundary — splitting there leaves the incomplete fence in the
 * tail, where it parses exactly as it would have unsplit.
 */
function lastSettledBoundary(text: string): number {
  let openFence: OpenFence | null = null;
  let previousBlank = false;
  let offset = 0;
  let boundary = -1;

  for (const line of text.split("\n")) {
    const start = offset;
    offset += line.length + 1;

    if (openFence) {
      if (closesFence(line, openFence)) openFence = null;
      previousBlank = false;
      continue;
    }

    if (line.trim() === "") {
      previousBlank = true;
      continue;
    }

    if (previousBlank && !ABSORBABLE_LINE.test(line)) boundary = start;
    previousBlank = false;

    const run = FENCE.exec(line)?.[1];
    if (run !== undefined) {
      openFence = { char: run.slice(0, 1), length: run.length };
    }
  }

  return boundary;
}

export function splitStableMarkdown(text: string): StableMarkdownSplit {
  if (text.length < MIN_SPLIT_CHARS || RESOLVES_LATER.test(text)) {
    return { prefix: "", tail: text };
  }
  const boundary = lastSettledBoundary(text);
  if (boundary < MIN_PREFIX_CHARS || boundary >= text.length) {
    return { prefix: "", tail: text };
  }
  return { prefix: text.slice(0, boundary), tail: text.slice(boundary) };
}
