import { ApiError } from "../../lib/api/ApiError";
import type { AssistantAttachment } from "./assistantTypes";
import {
  MAX_ASSISTANT_ATTACHMENT_BYTES,
  safeAttachmentName,
} from "./assistantUploads";

export const ATTACHMENT_EXPIRED_MESSAGE =
  "Attachment expired per retention policy. Upload it again to continue.";
export const IMAGE_INPUT_UNAVAILABLE_MESSAGE =
  "Image received. This agent version cannot view it, or it exceeds its image limits. Try a smaller image.";
export const ATTACHMENT_UNAVAILABLE_MESSAGE =
  "Attachment unavailable. Please try again or upload it again.";

const IMAGE_TYPES = new Set([
  "image/png",
  "image/jpeg",
  "image/gif",
  "image/webp",
]);
const EXTENSIONS: Readonly<Record<string, string>> = {
  "image/png": "png",
  "image/jpeg": "jpg",
  "image/gif": "gif",
  "image/webp": "webp",
  "application/pdf": "pdf",
  "application/vnd.openxmlformats-officedocument.wordprocessingml.document":
    "docx",
  "text/plain": "txt",
  "text/markdown": "md",
  "text/csv": "csv",
  "application/json": "json",
};

export function isPreviewableAttachment(attachment: AssistantAttachment) {
  return IMAGE_TYPES.has(attachment.content_type);
}

/** Always a leaf filename inside the app's private cache, never a supplied path. */
export function attachmentCacheName(attachment: AssistantAttachment): string {
  const name = safeAttachmentName(attachment.label);
  const extension = EXTENSIONS[attachment.content_type];
  if (!extension) return name;
  // iOS sharing infers document types from the URL suffix, even when the
  // backend correctly sniffed a file originally named e.g. `report.bin`.
  const stem = name.replace(/\.[a-z0-9]+$/i, "") || "attachment";
  return `${stem}.${extension}`;
}

/** Validate metadata before fetching, and the actual bytes before caching. */
export function assertReadableAttachment(
  attachment: AssistantAttachment,
  actualSize?: number,
): void {
  if (attachment.expired) throw new Error(ATTACHMENT_EXPIRED_MESSAGE);
  if (
    !Number.isSafeInteger(attachment.size) ||
    attachment.size < 1 ||
    attachment.size > MAX_ASSISTANT_ATTACHMENT_BYTES ||
    (actualSize !== undefined && actualSize !== attachment.size)
  )
    throw new Error(ATTACHMENT_UNAVAILABLE_MESSAGE);
}

export function attachmentAccessError(cause: unknown): string {
  if (
    cause instanceof ApiError &&
    (cause.statusCode === 410 || cause.errorCode === 12101)
  )
    return ATTACHMENT_EXPIRED_MESSAGE;
  if (cause instanceof Error && cause.message === ATTACHMENT_EXPIRED_MESSAGE)
    return ATTACHMENT_EXPIRED_MESSAGE;
  // Filesystem/native exceptions may contain filenames or paths.
  return ATTACHMENT_UNAVAILABLE_MESSAGE;
}
