/** Composer handles are local conveniences, never a new server routing field. */
export type AttachmentKind = "image" | "file" | "pasted-text";

export interface MentionOption {
  /** Without the `@`. */
  readonly handle: string;
  readonly label: string;
}

export interface AttachmentMention extends MentionOption {
  readonly kind: AttachmentKind;
}

/** React Native selection offsets, like string offsets, use UTF-16 units. */
export interface NativeTextSelection {
  readonly start: number;
  readonly end: number;
}

export interface ActiveMention {
  readonly start: number;
  /** Includes any remaining handle after the caret. */
  readonly end: number;
  readonly query: string;
}

const NAME_CHAR = /[A-Za-z0-9-]/;
const IMAGE_TYPES = new Set([
  "image/png",
  "image/jpeg",
  "image/gif",
  "image/webp",
]);

export function attachmentKind(file: {
  readonly type: string;
}): AttachmentKind {
  return IMAGE_TYPES.has(file.type) ? "image" : "file";
}

/** Match Web completion while leaving selection replacement and IME text alone. */
export function activeMention(
  text: string,
  selection: NativeTextSelection,
): ActiveMention | null {
  const caret = selection.start;
  if (
    selection.end !== caret ||
    !Number.isInteger(caret) ||
    caret < 0 ||
    caret > text.length
  )
    return null;
  let index = caret;
  while (index > 0 && NAME_CHAR.test(text[index - 1] ?? "")) index--;
  if (index === 0 || text[index - 1] !== "@") return null;
  const start = index - 1;
  if (start > 0 && !/\s/.test(text[start - 1] ?? "")) return null;
  let end = caret;
  while (end < text.length && NAME_CHAR.test(text[end] ?? "")) end++;
  return { start, end, query: text.slice(index, caret) };
}

/** Handle and friendly label are searchable, with stable prefix-first order. */
export function mentionCandidates<T extends MentionOption>(
  options: readonly T[],
  query: string,
): T[] {
  const needle = query.toLowerCase();
  const prefix: T[] = [];
  const inside: T[] = [];
  for (const option of options) {
    const keys = [option.handle.toLowerCase(), option.label.toLowerCase()];
    if (keys.some((key) => key.startsWith(needle))) prefix.push(option);
    else if (keys.some((key) => key.includes(needle))) inside.push(option);
  }
  return [...prefix, ...inside];
}

export function insertMention(
  text: string,
  range: ActiveMention,
  handle: string,
): { text: string; selection: NativeTextSelection } {
  const inserted = `@${handle} `;
  const caret = range.start + inserted.length;
  return {
    text: `${text.slice(0, range.start)}${inserted}${text.slice(range.end).replace(/^ /, "")}`,
    selection: { start: caret, end: caret },
  };
}

function escapeHandle(handle: string): string {
  return handle.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function describe(mention: AttachmentMention): string {
  const noun =
    mention.kind === "image"
      ? "image"
      : mention.kind === "pasted-text"
        ? "pasted text"
        : "file";
  // Labels cannot introduce agent routing when this draft belongs to a group.
  return `${noun} "${mention.label.replace(/@/g, "")}"`;
}

/**
 * Same attachment replacement as Web: known handles only, in a single pass.
 * The server receives descriptions plus ordinary attachment UUIDs; text added
 * by replacement is never scanned again. Handles remain stable when a chip is
 * removed, so file-1 must never accidentally consume file-10.
 */
export function resolveAttachmentMentions(
  text: string,
  mentions: readonly AttachmentMention[],
): string {
  const byHandle = new Map(
    mentions.map((mention) => [mention.handle, mention]),
  );
  if (!byHandle.size) return text;
  const handles = [...byHandle.keys()]
    .sort((a, b) => b.length - a.length)
    .map(escapeHandle);
  const pattern = new RegExp(
    `(^|[^A-Za-z0-9_.-])@(${handles.join("|")})(?![A-Za-z0-9_-])`,
    "g",
  );
  return text.replace(
    pattern,
    (_match: string, before: string, handle: string) =>
      `${before}${describe(byHandle.get(handle)!)}`,
  );
}

/** Remove only the deleted chip's references and one following space, as Web. */
export function stripAttachmentMention(text: string, handle: string): string {
  return text.replace(
    new RegExp(
      `(^|[^A-Za-z0-9_.-])@${escapeHandle(handle)}(?![A-Za-z0-9_-]) ?`,
      "g",
    ),
    (_match: string, before: string) => before,
  );
}
