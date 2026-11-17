// Shared bare-URL boundaries; vectors live in autolink-punctuation.json.
const SENTENCE_PUNCTUATION =
  /[。，、！？；：…・\uFF01-\uFF0F\uFF1A-\uFF20\uFF3B-\uFF40\uFF5B-\uFF65\u3000-\u303F]/u;
const PAIRS: Record<string, string> = {
  ")": "(",
  "]": "[",
  "}": "{",
  "）": "（",
  "」": "「",
  "』": "『",
  "】": "【",
  "》": "《",
  "〉": "〈",
  "”": "“",
  "’": "‘",
};

/** Interior Unicode belongs to the URL; only sentence endings and unmatched closers leave it. */
export function bareAutolinkBoundary(value: string): number {
  // One-time NyxID paths have an ASCII token alphabet, so adjacent prose cannot extend them.
  const connect =
    /^(?:https?:\/\/|www\.)[^\s/]+\/connect\/(?:bot\/)?nyx_(?:bcl|clk)_[A-Za-z0-9_-]+/i.exec(
      value,
    );
  if (connect) return connect[0].length;
  let end = value.length;
  while (end > 0) {
    const ch = value[end - 1]!;
    const opener = PAIRS[ch];
    if (opener) {
      const part = value.slice(0, end);
      if (part.split(ch).length > part.split(opener).length) {
        end--;
        continue;
      }
      break;
    }
    if (SENTENCE_PUNCTUATION.test(ch) || /[?!.,:;]/u.test(ch)) {
      end--;
      continue;
    }
    break;
  }
  return end;
}

/** Renderer glue supplies provenance and retains its existing URL safety policy. */
export function splitBareAutolinks(
  value: string,
): { text: string; href?: string }[] {
  const parts: { text: string; href?: string }[] = [];
  const starts = /https?:\/\/|www\.|(?:[a-z0-9-]+\.)+[a-z]{2,}(?=[/:?#]|\b)/gi;
  let cursor = 0;
  let match: RegExpExecArray | null;
  while ((match = starts.exec(value))) {
    const start = match.index;
    if (start > cursor) parts.push({ text: value.slice(cursor, start) });
    const rest = value.slice(start);
    const delimiter = rest.search(/[\s<>]/u);
    let extent = delimiter < 0 ? rest.length : delimiter;
    // A second scheme is a new literal; punctuation before it remains text.
    const next = /https?:\/\//gi;
    next.lastIndex = match[0].length;
    let following: RegExpExecArray | null;
    while ((following = next.exec(rest))) {
      const before = rest[following.index - 1]!;
      if (
        SENTENCE_PUNCTUATION.test(before) ||
        /[,;!?)\]}]/u.test(before) ||
        (before.charCodeAt(0) > 127 &&
          SENTENCE_PUNCTUATION.test(
            rest.slice(match[0].length, following.index),
          ))
      ) {
        extent = Math.min(extent, following.index);
        break;
      }
    }
    const text = rest.slice(0, bareAutolinkBoundary(rest.slice(0, extent)));
    if (!text) {
      starts.lastIndex = start + match[0].length;
      continue;
    }
    parts.push({
      text,
      href: /^https?:\/\//i.test(text) ? text : `http://${text}`,
    });
    cursor = start + text.length;
    starts.lastIndex = start + extent;
  }
  if (cursor < value.length) parts.push({ text: value.slice(cursor) });
  return parts;
}
