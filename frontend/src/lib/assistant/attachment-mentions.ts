/**
 * Composer attachments get short handles (`@image-1`, `@file-1`,
 * `@pasted-text-1`) the writer can mention. Handles are a composer-only
 * convenience: before a message is sent each one becomes a plain description of
 * the file, so the server (which routes `@name` to agents in group chats) never
 * sees them.
 */
export type AttachmentKind = "image" | "file" | "pasted-text";

export interface AttachmentMention {
  /** Without the `@`, e.g. `image-1`. */
  readonly handle: string;
  /** The file name the assistant receives. */
  readonly label: string;
  readonly kind: AttachmentKind;
}

const IMAGE_TYPES = new Set(["image/png", "image/jpeg", "image/gif", "image/webp"]);

export function attachmentKind(file: Pick<File, "type">): AttachmentKind {
  return IMAGE_TYPES.has(file.type) ? "image" : "file";
}

/** Pasted text longer than this is attached as a file instead of filling the composer. */
export const LONG_PASTE_LINES = 6;
export const LONG_PASTE_CHARS = 1_500;

export function isLongPaste(text: string): boolean {
  return text.split(/\r\n|\r|\n/).length > LONG_PASTE_LINES || text.length > LONG_PASTE_CHARS;
}

const MARKDOWN = /^(#{1,6} |\s*[-*+] |\s*\d+[.)] |>|```|~~~|\|.*\|)/m;

/** A pasted wall of text as a Markdown or plain-text file. */
export function pastedTextFile(text: string, index: number): File {
  const markdown = MARKDOWN.test(text);
  return new File([text], `pasted-text-${String(index)}.${markdown ? "md" : "txt"}`, {
    type: markdown ? "text/markdown" : "text/plain",
  });
}

function describe(mention: AttachmentMention): string {
  const noun = mention.kind === "image" ? "image" : mention.kind === "pasted-text" ? "pasted text" : "file";
  // A label can never add an `@mention` (group chats route on them). Server
  // labels already drop `@`; this keeps the guarantee local.
  return `${noun} "${mention.label.replace(/@/g, "")}"`;
}

function escapeHandle(handle: string): string {
  return handle.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/**
 * One pass over `text`: each `@handle` of a known attachment (same word rule as
 * agent mentions) becomes `replacement(mention)`. Inserted text is never
 * rescanned, so a pasted document that itself contains `@pasted-text-2` stays
 * verbatim. Output stops at `limit` UTF-16 units without building what it drops.
 */
function replaceHandles<T extends AttachmentMention>(
  text: string,
  mentions: readonly T[],
  replacement: (mention: T) => string,
  limit = Number.POSITIVE_INFINITY,
): string {
  const byHandle = new Map(mentions.map((mention) => [mention.handle, mention]));
  let out = "";
  const append = (part: string) => {
    if (out.length < limit) out += part.length > limit - out.length ? part.slice(0, limit - out.length) : part;
  };
  let last = 0;
  if (byHandle.size) {
    const handles = [...byHandle.keys()].sort((a, b) => b.length - a.length).map(escapeHandle);
    const pattern = new RegExp(`(^|[^A-Za-z0-9_.-])@(${handles.join("|")})(?![A-Za-z0-9_-])`, "g");
    for (const match of text.matchAll(pattern)) {
      if (out.length >= limit) break;
      const at = match.index + (match[1] ?? "").length;
      const handle = match[2] ?? "";
      append(text.slice(last, at));
      append(replacement(byHandle.get(handle)!));
      last = at + 1 + handle.length;
    }
  }
  append(text.slice(last));
  // Never end on half of a surrogate pair.
  const tail = out.charCodeAt(out.length - 1);
  return out.length >= limit && tail >= 0xd800 && tail <= 0xdbff ? out.slice(0, -1) : out;
}

/** Replace every `@handle` with a description of its file. */
export function resolveAttachmentMentions(
  text: string,
  mentions: readonly AttachmentMention[],
): string {
  return replaceHandles(text, mentions, describe);
}

/** The textbox's own limit. */
const MAX_DRAFT_CHARS = 32_768;

/**
 * What a saved draft keeps. Attachments do not survive the composer, so a
 * restored draft must not point at one: pasted text returns inline, other files
 * become their description. Bounded like the textbox.
 */
export function draftWithoutHandles(
  text: string,
  mentions: readonly (AttachmentMention & { readonly text?: string })[],
): string {
  return replaceHandles(
    text,
    mentions,
    (mention) => (mention.kind === "pasted-text" && mention.text !== undefined ? mention.text : describe(mention)),
    MAX_DRAFT_CHARS,
  );
}

/** Remove a removed attachment's `@handle`, with the space that followed it. */
export function stripAttachmentMention(text: string, handle: string): string {
  return text.replace(
    new RegExp(`(^|[^A-Za-z0-9_.-])@${escapeHandle(handle)}(?![A-Za-z0-9_-]) ?`, "g"),
    (_, before: string) => before,
  );
}
