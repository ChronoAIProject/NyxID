/** Matches the owner upload contract used by the Web composer. */
export const MAX_ASSISTANT_ATTACHMENT_BYTES = 20 * 1024 * 1024;
export const MAX_ASSISTANT_ATTACHMENTS = 10;
export const ASSISTANT_UPLOAD_TIMEOUT_MS = 60_000;
export const UNSUPPORTED_ATTACHMENT_MESSAGE =
  "Format not supported. Please try again.";
export const ATTACHMENT_SIZE_MESSAGE =
  "Choose a file between 1 byte and 20 MB.";

export interface AssistantUploadScope {
  readonly kind: "conversations" | "groups";
  readonly id?: string;
  readonly agentId?: string;
}

/** Picker metadata is a hint; the API checks the actual byte length as well. */
export interface AssistantUploadFile {
  readonly name: string;
  readonly mimeType?: string | null;
  readonly size?: number | null;
}

const DOCX =
  "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
const SNIFFED_TYPES = new Set([
  "image/png",
  "image/jpeg",
  "image/gif",
  "image/webp",
  "application/pdf",
  DOCX,
]);
const SNIFFED_EXTENSIONS = new Set([
  "png",
  "jpg",
  "jpeg",
  "gif",
  "webp",
  "pdf",
  "docx",
]);
const TEXT_EXTENSIONS = new Set([
  "",
  "txt",
  "text",
  "md",
  "markdown",
  "csv",
  "json",
]);
const UNKNOWN_TYPES = new Set(["", "application/octet-stream"]);
const CSV_TYPES = new Set(["application/csv", "application/vnd.ms-excel"]);

/** Mirror `assistant_upload_service::safe_name`, including Unicode limits. */
export function safeAttachmentName(name: string): string {
  const leaf = name.split(/[/\\]/).pop() ?? "";
  const kept = [...leaf.replace(/[^\p{L}\p{N}._\- ()]/gu, "")]
    .slice(0, 120)
    .join("");
  return (
    kept
      .trim()
      .replace(/^\.+|\.+$/g, "")
      .trim() || "attachment"
  );
}

function extension(name: string): string {
  const clean = safeAttachmentName(name);
  const dot = clean.lastIndexOf(".");
  return dot < 0 ? "" : clean.slice(dot + 1).toLowerCase();
}

/** MIME and suffix only guide the picker; the server still checks the bytes. */
export function isSupportedAttachment(file: AssistantUploadFile): boolean {
  const type = (file.mimeType?.split(";")[0] ?? "").trim().toLowerCase();
  const suffix = extension(file.name);
  if (SNIFFED_TYPES.has(type)) return true;
  if (UNKNOWN_TYPES.has(type))
    return SNIFFED_EXTENSIONS.has(suffix) || TEXT_EXTENSIONS.has(suffix);
  if (!TEXT_EXTENSIONS.has(suffix)) return false;
  return (
    type.startsWith("text/") ||
    type === "application/json" ||
    (suffix === "csv" && CSV_TYPES.has(type))
  );
}

export function attachmentFileError(
  file: AssistantUploadFile,
  actualByteSize: number | null | undefined = file.size,
): string | null {
  if (!isSupportedAttachment(file)) return UNSUPPORTED_ATTACHMENT_MESSAGE;
  if (
    actualByteSize !== undefined &&
    actualByteSize !== null &&
    (!Number.isSafeInteger(actualByteSize) ||
      actualByteSize < 1 ||
      actualByteSize > MAX_ASSISTANT_ATTACHMENT_BYTES)
  )
    return ATTACHMENT_SIZE_MESSAGE;
  return null;
}

export function assistantAttachmentPath(
  scope: AssistantUploadScope,
  attachmentId?: string,
): string {
  if (!scope.id?.trim()) throw new Error("Choose a conversation first.");
  if (attachmentId !== undefined && !attachmentId.trim())
    throw new Error("Choose an attachment first.");
  return `/assistant/nyxagent/${scope.kind}/${encodeURIComponent(scope.id)}/attachments${
    attachmentId === undefined ? "" : `/${encodeURIComponent(attachmentId)}`
  }`;
}

/** Raw bytes, never multipart; preserve the dialect suffix under the header cap. */
export function attachmentUploadHeaders(name: string): Record<string, string> {
  let safeName = safeAttachmentName(name);
  while (encodeURIComponent(safeName).length > 1024) {
    const dot = safeName.lastIndexOf(".");
    const stem = [...(dot > 0 ? safeName.slice(0, dot) : safeName)];
    const suffix = dot > 0 ? safeName.slice(dot) : "";
    if (stem.length > 1) safeName = stem.slice(0, -1).join("") + suffix;
    else safeName = [...safeName].slice(0, -1).join("");
  }
  return {
    "Content-Type": "application/octet-stream",
    "X-Attachment-Name": encodeURIComponent(safeName),
  };
}

export function attachmentUploadError(error: unknown): string {
  if (!(error instanceof Error)) return "Upload failed. Please try again.";
  if (/malformed or unsupported attachment/i.test(error.message))
    return UNSUPPORTED_ATTACHMENT_MESSAGE;
  if (error.name === "AbortError") return "Upload cancelled.";
  return error.message || "Upload failed. Please try again.";
}

/** Cancels transport work and stops a stuck upload without replaying its POST. */
export async function withAttachmentUploadDeadline<T>(
  upload: (signal: AbortSignal) => Promise<T>,
  signal?: AbortSignal,
  milliseconds = ASSISTANT_UPLOAD_TIMEOUT_MS,
): Promise<T> {
  const cancelled = () =>
    Object.assign(new Error("Upload cancelled."), { name: "AbortError" });
  if (signal?.aborted) throw cancelled();
  const controller = new AbortController();
  let timer: ReturnType<typeof setTimeout> | undefined;
  let rejectDeadline: (error: Error) => void = () => undefined;
  const deadline = new Promise<never>((_, reject) => {
    rejectDeadline = reject;
    timer = setTimeout(() => {
      reject(new Error("Upload timed out. Try a smaller file."));
      controller.abort();
    }, milliseconds);
  });
  const abort = () => {
    rejectDeadline(cancelled());
    controller.abort();
  };
  signal?.addEventListener("abort", abort, { once: true });
  try {
    return await Promise.race([upload(controller.signal), deadline]);
  } finally {
    clearTimeout(timer);
    signal?.removeEventListener("abort", abort);
  }
}
