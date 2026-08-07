/**
 * Choosing WHERE to cut a partially-revealed answer.
 *
 * Pacing decides how much of an arrived answer is on screen; this decides where
 * that prefix is allowed to end. The two are independent on purpose — the cut
 * is applied to the controller's output, never fed back into it, so nothing
 * here can change the reveal rate or strand content.
 *
 * A cut at an arbitrary offset is wrong three ways, all of them visible:
 *
 *   - It splits a grapheme. `slice` counts UTF-16 code units, so cutting inside
 *     a surrogate pair, a ZWJ sequence, a skin-tone modifier or a combining
 *     mark paints a replacement glyph or a decomposed sequence for a frame.
 *   - It splits inline Markdown. `**bo` has no closing run yet, so it renders
 *     as literal asterisks and then re-renders as bold once they arrive — a
 *     flicker on every emphasis, and for links the whole raw URL is shown while
 *     it streams.
 *   - It splits a word, which reflows the line when the rest lands.
 *
 * Each rule only ever moves the cut EARLIER, and the total move is bounded by
 * `MAX_HOLDBACK`. Past that bound the text is revealed as-is: withholding
 * output indefinitely to protect an opener the model may never close is the
 * worse failure, so the bound is the explicit trade rather than an oversight.
 */

/**
 * The most already-arrived text any rule may withhold. Sized well above a
 * typical emphasis run or link so ordinary constructs always resolve, and far
 * below the point where a reader would notice the answer sitting still.
 */
const MAX_HOLDBACK = 96;

/** Word snapping alone gets a tighter bound; a long token reveals as it comes. */
const MAX_WORD_HOLDBACK = 24;

/** How far back an unclosed inline opener is looked for. */
const INLINE_WINDOW = 256;

/** A fenced block opens on a run of at least this many backticks at line start. */
const MIN_FENCE_LENGTH = 3;

/** CommonMark allows up to this much indent before a fence still counts. */
const MAX_FENCE_INDENT = 3;

/** Guards the backwards walks against pathological input. */
const MAX_STEPS = 64;

function isLowSurrogate(code: number): boolean {
  return code >= 0xdc00 && code <= 0xdfff;
}

function isHighSurrogate(code: number): boolean {
  return code >= 0xd800 && code <= 0xdbff;
}

const COMBINING_MARK = /\p{M}/u;

/**
 * True when the code point at `at` attaches to whatever precedes it: combining
 * marks, ZWJ, variation selectors, skin-tone modifiers and the keycap
 * combiner. A cut immediately before one of these separates it from its base.
 * (Explicit code-point checks rather than one character class: lone combining
 * code points in a class trip `no-misleading-character-class`, whose warning
 * is exactly the behaviour wanted here.)
 */
function attachesLeft(text: string, at: number): boolean {
  const code = text.codePointAt(at);
  if (code === undefined) return false;
  if (code === 0x200d || code === 0xfe0e || code === 0xfe0f || code === 0x20e3) {
    return true;
  }
  if (code >= 0x1f3fb && code <= 0x1f3ff) return true;
  return COMBINING_MARK.test(String.fromCodePoint(code));
}

/** Scripts written without inter-word spaces, where snapping to a "word" would
 *  either withhold a whole clause or do nothing useful. */
const UNSPACED_SCRIPT =
  /[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}\p{Script=Hangul}\p{Script=Thai}\p{Script=Lao}\p{Script=Khmer}\p{Script=Myanmar}]/u;

const WORD_CHARACTER = /[\p{L}\p{N}]/u;

let cachedSegmenter: Intl.Segmenter | null | undefined;

function graphemeSegmenter(): Intl.Segmenter | null {
  if (cachedSegmenter === undefined) {
    cachedSegmenter =
      typeof Intl !== "undefined" && typeof Intl.Segmenter === "function"
        ? new Intl.Segmenter(undefined, { granularity: "grapheme" })
        : null;
  }
  return cachedSegmenter;
}

/**
 * The largest grapheme boundary at or before `end`.
 *
 * `Segments.containing` keeps the full Unicode context. A fixed look-behind
 * window is unsound because combining sequences and regional-indicator context
 * have no fixed maximum length.
 */
function graphemeSafeEnd(text: string, end: number): number {
  // The end of the string is always a boundary, but the segmenter only ever
  // reports segment STARTS — without this the final grapheme of a fully
  // arrived text is never creditable and the last word is withheld forever.
  if (end >= text.length) return text.length;
  const segmenter = graphemeSegmenter();
  if (segmenter === null) return graphemeSafeEndFallback(text, end);

  return segmenter.segment(text).containing(end)?.index ?? end;
}

/** Environments without `Intl.Segmenter` still must not split a pair. */
function graphemeSafeEndFallback(text: string, end: number): number {
  let cut = end;
  for (let step = 0; step < MAX_STEPS && cut > 0; step += 1) {
    if (isLowSurrogate(text.charCodeAt(cut))) {
      cut -= 1;
      continue;
    }
    const previous = text.charCodeAt(cut - 1);
    const attaches = attachesLeft(text, cut) || previous === 0x200d;
    if (!attaches) break;
    cut -= isHighSurrogate(text.charCodeAt(cut - 2)) ? 2 : 1;
  }
  return Math.max(cut, 0);
}

function runLength(
  text: string,
  from: number,
  character: string,
  end = text.length,
): number {
  let length = 0;
  while (from + length < end && text[from + length] === character) length += 1;
  return length;
}

/** Leading whitespace on the line containing `at`, or -1 when `at` is not at
 *  the start of its line (ignoring indent). */
function lineIndent(text: string, at: number): number {
  let indent = 0;
  for (let index = at - 1; index >= 0; index -= 1) {
    const character = text[index];
    if (character === "\n") return indent;
    if (character !== " " && character !== "\t") return -1;
    indent += 1;
  }
  return indent;
}

/**
 * True when the backtick run at `at` opens a FENCED BLOCK rather than a code
 * span. The distinction matters: a fence's content is literal code, so there is
 * nothing inline to protect inside it — treating ``` as a span opener parks the
 * reveal at the top of every code block until the holdback bound expires and
 * then dumps the whole thing at once, which is exactly the stutter this module
 * exists to remove.
 */
function isFence(text: string, at: number, length: number): boolean {
  if (length < MIN_FENCE_LENGTH) return false;
  const indent = lineIndent(text, at);
  return indent >= 0 && indent <= MAX_FENCE_INDENT;
}

/**
 * The start of the earliest inline construct that is still open at `end`, or
 * `end` when every construct in range is closed.
 *
 * One left-to-right pass over a bounded window. Code spans win over everything
 * — inside them no other marker means anything — which is also what keeps a
 * stray asterisk in a code sample from stalling the reveal.
 */
interface EmphasisDelimiter {
  readonly at: number;
  readonly character: string;
  readonly length: number;
}

const WHITESPACE = /\s/u;
const PUNCTUATION_OR_SYMBOL = /[\p{P}\p{S}]/u;

function codePointBefore(text: string, at: number): string {
  return Array.from(text.slice(Math.max(0, at - 2), at)).at(-1) ?? "";
}

function codePointAfter(text: string, at: number): string {
  return Array.from(text.slice(at, at + 2))[0] ?? "";
}

function delimiterFlanking(
  text: string,
  at: number,
  length: number,
  character: string,
): { readonly canOpen: boolean; readonly canClose: boolean } {
  const before = codePointBefore(text, at);
  const after = codePointAfter(text, at + length);
  const beforeWhitespace = before === "" || WHITESPACE.test(before);
  const afterWhitespace = after === "" || WHITESPACE.test(after);
  const beforePunctuation = PUNCTUATION_OR_SYMBOL.test(before);
  const afterPunctuation = PUNCTUATION_OR_SYMBOL.test(after);
  const leftFlanking =
    !afterWhitespace &&
    (!afterPunctuation || beforeWhitespace || beforePunctuation);
  const rightFlanking =
    !beforeWhitespace &&
    (!beforePunctuation || afterWhitespace || afterPunctuation);

  if (character !== "_") {
    return { canOpen: leftFlanking, canClose: rightFlanking };
  }
  return {
    canOpen: leftFlanking && (!rightFlanking || beforePunctuation),
    canClose: rightFlanking && (!leftFlanking || afterPunctuation),
  };
}

function matchingDelimiterIndex(
  emphasis: readonly EmphasisDelimiter[],
  character: string,
  length: number,
): number {
  for (let index = emphasis.length - 1; index >= 0; index -= 1) {
    const delimiter = emphasis[index];
    if (delimiter?.character === character && delimiter.length === length) {
      return index;
    }
  }
  return -1;
}

function linkDestinationEnd(
  text: string,
  openParen: number,
  end: number,
): number | null {
  let depth = 1;
  let index = openParen + 1;
  while (index < end) {
    const character = text[index];
    if (character === "\\") {
      index += 2;
      continue;
    }
    if (character === "(") depth += 1;
    if (character === ")") {
      depth -= 1;
      if (depth === 0) return index + 1;
    }
    index += 1;
  }
  return null;
}

/** Offset just past the closing fence of the block opened at `openAt`, or null
 *  when the block is still open at `end`. */
function fenceCloseEnd(
  text: string,
  openAt: number,
  openLength: number,
  end: number,
): number | null {
  let index = text.indexOf("\n", openAt + openLength);
  while (index >= 0 && index < end) {
    const lineStart = index + 1;
    let cursor = lineStart;
    while (cursor < end && (text[cursor] === " " || text[cursor] === "\t")) {
      cursor += 1;
    }
    const length = runLength(text, cursor, text[openAt] ?? "`", end);
    if (
      length >= openLength &&
      cursor - lineStart <= MAX_FENCE_INDENT &&
      length > 0
    ) {
      return cursor + length;
    }
    index = text.indexOf("\n", lineStart);
  }
  return null;
}

function inlineSafeEnd(text: string, end: number): number {
  const start = Math.max(0, end - INLINE_WINDOW);
  const emphasis: EmphasisDelimiter[] = [];
  let codeSpanAt: number | null = null;
  let codeSpanLength = 0;
  const brackets: number[] = [];
  let index = start;

  while (index < end) {
    const character = text[index];

    if (character === "\\") {
      index += 2;
      continue;
    }

    if (character === "`") {
      const length = runLength(text, index, "`", end);
      if (codeSpanAt === null && isFence(text, index, length)) {
        // Inside an open fence nothing is inline; a closing fence ends the
        // block and also ends any paragraph-level construct before it.
        const closing = fenceCloseEnd(text, index, length, end);
        if (closing === null) return end;
        emphasis.length = 0;
        brackets.length = 0;
        index = closing;
        continue;
      }
      if (codeSpanAt === null) {
        codeSpanAt = index;
        codeSpanLength = length;
      } else if (length === codeSpanLength) {
        codeSpanAt = null;
      }
      index += length;
      continue;
    }

    if (codeSpanAt !== null) {
      index += 1;
      continue;
    }

    if (character === "*" || character === "_" || character === "~") {
      const length = runLength(text, index, character, end);
      // A cut INSIDE a delimiter run is never right — it paints `*` for a `**`
      // that has already arrived. Looking past the head is safe here because it
      // can only hold more, never release an unclosed run early.
      if (
        index + length >= end &&
        runLength(text, index, character, text.length) > length
      ) {
        emphasis.push({ at: index, character, length });
        break;
      }
      // Single-marker runs are NOT held. A lone `*` or `_` is far more often
      // literal prose — a glob (`*.ts`), a multiplication (`5*3`), a list
      // bullet, `SELECT *` — than an italic opener that will close, and the
      // cost of guessing wrong is asymmetric: a false positive freezes the
      // reveal for up to MAX_HOLDBACK characters, while a false negative
      // flickers one asterisk. `**`, `~~`, code spans and links, where a bad
      // cut shows `**bo` or a whole raw URL, are still protected.
      // GFM strikethrough is likewise only a pair, so "~10 ms" never holds.
      if (length < 2 || (character === "~" && length !== 2)) {
        index += length;
        continue;
      }
      const { canOpen, canClose } = delimiterFlanking(
        text,
        index,
        length,
        character,
      );
      const matching = canClose
        ? matchingDelimiterIndex(emphasis, character, length)
        : -1;
      // A run touching the head has no right-hand context yet, so it cannot be
      // ruled out as an opener. Closing is decidable (it depends only on what
      // precedes), so a genuine closer still closes; anything else is held for
      // one arrival — without this, a chunk that ends exactly at `**` paints
      // the literal markers and the display clamp then pins them forever.
      const atHead = index + length >= end;
      if (matching >= 0) {
        emphasis.splice(matching);
      } else if (canOpen || atHead) {
        emphasis.push({ at: index, character, length });
      }
      index += length;
      continue;
    }

    if (character === "[") {
      brackets.push(index);
      index += 1;
      continue;
    }

    if (character === "]" && brackets.length > 0) {
      // The next unit disambiguates a shortcut reference from an inline link.
      // At the current head, keep the bracket provisional for one arrival.
      if (index + 1 >= end) break;
      if (text[index + 1] !== "(") {
        brackets.pop();
        index += 1;
        continue;
      }
      const destinationEnd = linkDestinationEnd(text, index + 1, end);
      if (destinationEnd === null) break;
      brackets.pop();
      index = destinationEnd;
      continue;
    }

    index += 1;
  }

  const openings = [
    codeSpanAt,
    emphasis[0]?.at ?? null,
    brackets[0] ?? null,
  ].filter(
    (position): position is number => position !== null,
  );
  return openings.length === 0 ? end : Math.min(...openings);
}

/** Pulls a cut that lands inside a word back to the word's start. */
function wordSafeEnd(text: string, end: number): number {
  const before = text.slice(Math.max(0, end - 2), end);
  const after = text.slice(end, end + 2);
  if (!WORD_CHARACTER.test(before.slice(-1)) || !WORD_CHARACTER.test(after)) {
    return end;
  }
  // Per-character reveal is the correct granularity where words are not
  // space-delimited, and snapping there would withhold whole clauses.
  if (UNSPACED_SCRIPT.test(before) || UNSPACED_SCRIPT.test(after)) return end;

  let cut = end;
  const limit = Math.max(0, end - MAX_WORD_HOLDBACK);
  while (cut > limit && WORD_CHARACTER.test(text[cut - 1] ?? "")) cut -= 1;
  return cut <= limit ? end : cut;
}

/**
 * Where a reveal of `desired` characters may actually end.
 *
 * Grapheme safety is applied unconditionally — it costs at most a cluster and
 * a broken glyph is never acceptable. Inline and word safety are applied on
 * top and then abandoned together if they would exceed `MAX_HOLDBACK`, so a
 * never-closed opener degrades to today's behaviour instead of a stalled answer.
 */
export function safeRevealEnd(text: string, desired: number): number {
  if (desired <= 0) return 0;
  const capped = Math.min(desired, text.length);

  const grapheme = graphemeSafeEnd(text, capped);
  if (grapheme <= 0) return 0;

  const inline = Math.min(grapheme, inlineSafeEnd(text, grapheme));
  const word = Math.min(inline, wordSafeEnd(text, inline));
  // Past the bound the hold is CLAMPED, not abandoned. Abandoning it returns
  // `grapheme`, which paints the withheld run in a single frame — a
  // ~MAX_HOLDBACK-character jump after a ~MAX_HOLDBACK-character freeze, which
  // is a worse stutter than the flicker the hold was protecting against. Riding
  // a constant bounded distance behind keeps the text moving instead.
  const bounded = Math.max(word, capped - MAX_HOLDBACK);
  // A construct-aware cut may itself land mid-cluster.
  return bounded === grapheme ? grapheme : graphemeSafeEnd(text, bounded);
}
