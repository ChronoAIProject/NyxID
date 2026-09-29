/**
 * `@name` mentions in group chats. A mention is `@` at the start of the text
 * or after whitespace, followed by an agent name; matching is
 * case-insensitive, like the server's routing.
 */

const NAME_CHAR = /[A-Za-z0-9-]/;

export interface MentionQuery {
  /** Index of the `@`. */
  readonly start: number;
  /** What was typed after the `@` (may be empty). */
  readonly query: string;
}

/** The mention being typed at `caret`, if the caret sits inside one. */
export function activeMention(text: string, caret: number): MentionQuery | undefined {
  let index = caret;
  while (index > 0 && NAME_CHAR.test(text[index - 1] ?? "")) index -= 1;
  if (index === 0 || text[index - 1] !== "@") return undefined;
  const start = index - 1;
  if (start > 0 && !/\s/.test(text[start - 1] ?? "")) return undefined;
  return { start, query: text.slice(index, caret) };
}

/**
 * Candidates for a partial mention, prefix matches first. The handle is what
 * gets inserted, but a display name ("Luna") finds its agent too.
 */
export function mentionCandidates<
  T extends { readonly name: string; readonly display_name?: string | null },
>(members: readonly T[], query: string): T[] {
  const needle = query.toLowerCase();
  const keys = (member: T) =>
    [member.name, member.display_name ?? ""].map((key) => key.toLowerCase()).filter(Boolean);
  const prefix = members.filter((member) => keys(member).some((key) => key.startsWith(needle)));
  const inside = members.filter(
    (member) => !prefix.includes(member) && keys(member).some((key) => key.includes(needle)),
  );
  return [...prefix, ...inside];
}

/** Replace the mention being typed with `@name ` and return the new caret. */
export function insertMention(
  text: string,
  mention: MentionQuery,
  caret: number,
  name: string,
): { text: string; caret: number } {
  let end = caret;
  while (end < text.length && NAME_CHAR.test(text[end] ?? "")) end += 1;
  const rest = text.slice(end).replace(/^ /, "");
  const inserted = `@${name} `;
  return {
    text: `${text.slice(0, mention.start)}${inserted}${rest}`,
    caret: mention.start + inserted.length,
  };
}

/** Names mentioned in `text` that belong to `members` (case-insensitive). */
export function mentionedNames(text: string, names: readonly string[]): string[] {
  const byLower = new Map(names.map((name) => [name.toLowerCase(), name]));
  const found: string[] = [];
  for (const match of text.matchAll(/(^|\s)@([A-Za-z0-9][A-Za-z0-9-]*)/g)) {
    const name = byLower.get((match[2] ?? "").toLowerCase());
    if (name && !found.includes(name)) found.push(name);
  }
  return found;
}

/** Split text into plain runs and `@member` runs, for highlighting. */
export function mentionSegments(
  text: string,
  names: readonly string[],
): { text: string; mention: boolean }[] {
  const known = new Set(names.map((name) => name.toLowerCase()));
  const segments: { text: string; mention: boolean }[] = [];
  let last = 0;
  for (const match of text.matchAll(/(^|\s)(@([A-Za-z0-9][A-Za-z0-9-]*))/g)) {
    if (!known.has((match[3] ?? "").toLowerCase())) continue;
    const start = (match.index ?? 0) + (match[1] ?? "").length;
    if (start > last) segments.push({ text: text.slice(last, start), mention: false });
    segments.push({ text: match[2] ?? "", mention: true });
    last = start + (match[2] ?? "").length;
  }
  if (last < text.length) segments.push({ text: text.slice(last), mention: false });
  return segments;
}
