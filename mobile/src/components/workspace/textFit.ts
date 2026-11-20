// Pure text-fit math (no React), so tests load it unmocked.

/**
 * Manrope Regular advance widths in em for ASCII U+0020–U+007E, read from the
 * bundled Manrope_400Regular.ttf (hmtx / unitsPerEm 2000). Kerning only
 * tightens text, so sums are an upper bound before the safety margin.
 */
const MANROPE_REGULAR_ASCII_EM = [
  0.2, 0.314, 0.392, 0.92, 0.589, 0.901, 0.651, 0.213, 0.426, 0.426, 0.436, 0.577,
  0.266, 0.42, 0.254, 0.388, 0.61, 0.39, 0.57, 0.553, 0.59, 0.578, 0.621, 0.502,
  0.581, 0.621, 0.299, 0.305, 0.615, 0.75, 0.615, 0.523, 0.903, 0.633, 0.613, 0.716,
  0.671, 0.57, 0.503, 0.705, 0.684, 0.234, 0.464, 0.594, 0.51, 0.841, 0.684, 0.724,
  0.604, 0.724, 0.63, 0.617, 0.597, 0.702, 0.604, 0.935, 0.606, 0.556, 0.614, 0.407,
  0.388, 0.407, 0.669, 0.66, 0.489, 0.555, 0.592, 0.553, 0.593, 0.585, 0.354, 0.592,
  0.594, 0.234, 0.251, 0.496, 0.234, 0.848, 0.594, 0.592, 0.592, 0.593, 0.36, 0.525,
  0.401, 0.594, 0.508, 0.767, 0.527, 0.527, 0.529, 0.419, 0.254, 0.419, 0.662,
];
/** CJK and fullwidth forms take a full em in the system fallback font. */
const FULL_WIDTH = /[\u1100-\u11ff\u2e80-\u9fff\uac00-\ud7af\uf900-\ufaff\ufe30-\ufe4f\uff00-\uffef]/;
/** Other characters (accents, curly quotes, ellipsis): a generous average. */
const OTHER_EM = 0.65;
/** Headroom for rendering differences between this estimate and iOS. */
export const TEXT_WIDTH_SAFETY = 1.05;

/** Estimated rendered width of one line of Manrope Regular body text. */
export function estimateBodyTextWidth(text: string, fontSize: number) {
  let em = 0;
  for (const char of text) {
    const code = char.codePointAt(0)!;
    em +=
      code >= 0x20 && code <= 0x7e
        ? MANROPE_REGULAR_ASCII_EM[code - 0x20]!
        : FULL_WIDTH.test(char)
          ? 1
          : OTHER_EM;
  }
  return em * fontSize * TEXT_WIDTH_SAFETY;
}

/** Search field geometry: the styles and the fit budget share these. */
export const SEARCH_FIELD = {
  fontSize: 15,
  border: 1,
  paddingLeft: 12,
  iconSize: 18,
  gap: 8,
  /** Placeholder and typed text stop short of the border. */
  textPaddingRight: 12,
} as const;

/** Room for placeholder text in a search field of the given outer width. */
export function searchFieldTextWidth(fieldWidth: number) {
  const f = SEARCH_FIELD;
  return (
    fieldWidth - 2 * f.border - f.paddingLeft - f.iconSize - f.gap - f.textPaddingRight
  );
}

/**
 * The first placeholder (most descriptive first) that fits the measured text
 * width at the current text size; the last (shortest, e.g. "Search") when
 * none does or before the field has been measured. Each candidate is a
 * complete translated phrase, never assembled.
 */
export function fittingPlaceholder(
  candidates: readonly string[],
  textWidth: number | undefined,
  fontSize: number,
) {
  const shortest = candidates[candidates.length - 1]!;
  if (!textWidth) return shortest;
  return (
    candidates.find((text) => estimateBodyTextWidth(text, fontSize) <= textWidth) ??
    shortest
  );
}
