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

/**
 * True when `*` at `at` is a list bullet rather than emphasis: first thing on
 * its line and followed by a space. Holding a cut back to a bullet would stall
 * the reveal at the start of every list item.
 */
function isBullet(text: string, at: number, length: number): boolean {
  if (length !== 1 || text[at + 1] !== " ") return false;
  for (let index = at - 1; index >= 0; index -= 1) {
    const character = text[index];
    if (character === "\n") return true;
    if (character !== " " && character !== "\t") return false;
  }
  return true;
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
      if (character === "*" && isBullet(text, index, length)) {
        index += length;
        continue;
      }
      // GFM strikethrough uses pairs. Treating a prose tilde as a delimiter
      // would unnecessarily hold ordinary approximations such as "~10 ms".
      if (character === "~" && length !== 2) {
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
      if (matching >= 0) {
        emphasis.splice(matching);
      } else if (canOpen) {
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
  if (capped - word > MAX_HOLDBACK) return grapheme;
  // A construct-aware cut may itself land mid-cluster.
  return word === grapheme ? grapheme : graphemeSafeEnd(text, word);
}
