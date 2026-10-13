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
  return `${noun} "${mention.label}"`;
}

/** Same word rule as agent mentions: `@` starts a word and the handle ends one. */
function mentionPattern(handle: string): RegExp {
  const escaped = handle.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return new RegExp(`(^|[^A-Za-z0-9_.-])@${escaped}(?![A-Za-z0-9_-])`, "g");
}

/** Replace every `@handle` with a description of its file. */
export function resolveAttachmentMentions(
  text: string,
  mentions: readonly AttachmentMention[],
): string {
  return mentions.reduce(
    (resolved, mention) =>
      resolved.replace(mentionPattern(mention.handle), (_, before: string) => `${before}${describe(mention)}`),
    text,
  );
}

/** Remove a removed attachment's `@handle`, with the space that followed it. */
export function stripAttachmentMention(text: string, handle: string): string {
  const escaped = handle.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return text.replace(
    new RegExp(`(^|[^A-Za-z0-9_.-])@${escaped}(?![A-Za-z0-9_-]) ?`, "g"),
    (_, before: string) => before,
  );
}
