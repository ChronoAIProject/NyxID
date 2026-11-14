// Shared bare-URL boundaries; vectors live in autolink-punctuation.json.
const PUNCTUATION =
  /[。，、！？；：「」『』（）【】《》〈〉“”‘’…・\uFF01-\uFF0F\uFF1A-\uFF20\uFF3B-\uFF40\uFF5B-\uFF65\u3000-\u303F]/u;
export function bareAutolinkBoundary(value: string): number {
  const index = value.search(PUNCTUATION);
  return index < 0 ? value.length : index;
}

/** Split every HTTP(S) literal in a run without interpreting the prose as Markdown. */
export function splitBareAutolinks(
  value: string,
): { text: string; href?: string }[] {
  const parts: { text: string; href?: string }[] = [];
  const starts = /https?:\/\//gi;
  let cursor = 0;
  let match: RegExpExecArray | null;
  while ((match = starts.exec(value))) {
    const start = match.index;
    if (start > cursor) parts.push({ text: value.slice(cursor, start) });
    const rest = value.slice(start);
    const whitespace = rest.search(/[\s<>]/u);
    let length = Math.min(
      bareAutolinkBoundary(rest),
      whitespace < 0 ? rest.length : whitespace,
    );
    // Match literal autolinks' ASCII sentence endings for later URLs too.
    let href = rest.slice(0, length).replace(/[?!.,:;]+$/u, "");
    while (
      href.endsWith(")") &&
      (href.match(/\)/g)?.length ?? 0) > (href.match(/\(/g)?.length ?? 0)
    ) {
      href = href.slice(0, -1).replace(/[?!.,:;]+$/u, "");
    }
    length = href.length;
    parts.push({ text: href, href });
    cursor = start + length;
    starts.lastIndex = cursor;
  }
  if (cursor < value.length) parts.push({ text: value.slice(cursor) });
  return parts;
}
