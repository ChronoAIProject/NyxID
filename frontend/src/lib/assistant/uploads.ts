import { apiUrl, buildFetchConfig } from "@/lib/api-client";
import { assistantJson } from "@/lib/assistant/assistant-http";
import {
  nyxAgentAttachmentSchema,
  type NyxAgentAttachment,
} from "@/schemas/assistant-nyxagent";

export interface UploadScope {
  readonly kind: "conversations" | "groups";
  readonly id?: string;
  readonly agentId?: string;
}
export interface UploadedMessage {
  readonly attachmentIds: string[];
  readonly attachments?: readonly NyxAgentAttachment[];
  readonly conversationId?: string;
}

export const UNSUPPORTED_FORMAT_MESSAGE = "Format not supported. Please try again.";

const DOCX = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
/** The server identifies these from their bytes, whatever the file is called. */
const SNIFFED_TYPES = new Set(["image/png", "image/jpeg", "image/gif", "image/webp", "application/pdf", DOCX]);
const SNIFFED_EXTENSIONS = new Set(["png", "jpg", "jpeg", "gif", "webp", "pdf", "docx"]);
/** UTF-8 text dialects the server accepts by name; "" is an extensionless file. */
const TEXT_EXTENSIONS = new Set(["", "txt", "text", "md", "markdown", "csv", "json"]);
const UNKNOWN_TYPES = new Set(["", "application/octet-stream"]);
// Windows reports CSV files with Excel's MIME type.
const CSV_TYPES = new Set(["application/csv", "application/vnd.ms-excel"]);

/** The extension the server derives from the sanitized upload name (`safe_name`). */
function serverExtension(name: string): string {
  const leaf = name.split(/[/\\]/).pop() ?? "";
  const kept = [...leaf.replace(/[^\p{L}\p{N}._\- ()]/gu, "")].slice(0, 120).join("");
  const clean = kept.trim().replace(/^\.+|\.+$/g, "").trim();
  const dot = clean.lastIndexOf(".");
  return dot < 0 ? "" : clean.slice(dot + 1).toLowerCase();
}

/**
 * Mirrors `attachment_extraction::parse`: PNG, JPEG, GIF, WebP, PDF and DOCX
 * (content-sniffed), and UTF-8 text, Markdown, CSV and JSON chosen by
 * extension. Anything else is refused before its bytes are uploaded.
 */
export function isSupportedAttachment(file: Pick<File, "name" | "type">): boolean {
  const type = (file.type.split(";")[0] ?? "").trim().toLowerCase();
  const extension = serverExtension(file.name);
  if (SNIFFED_TYPES.has(type)) return true;
  if (UNKNOWN_TYPES.has(type)) {
    return SNIFFED_EXTENSIONS.has(extension) || TEXT_EXTENSIONS.has(extension);
  }
  if (!TEXT_EXTENSIONS.has(extension)) return false;
  return (
    type.startsWith("text/") ||
    type === "application/json" ||
    (extension === "csv" && CSV_TYPES.has(type))
  );
}

export async function createUploadDraft(agentId?: string): Promise<string> {
  const draft = await assistantJson<{ id: string }>(
    "/assistant/nyxagent/drafts",
    {
      method: "POST",
      body: agentId ? { agent_id: agentId } : {},
    },
  );
  return draft.id;
}
export function attachmentPath(scope: UploadScope, id?: string): string {
  if (!scope.id) throw new Error("Choose a conversation first.");
  return `/assistant/nyxagent/${scope.kind}/${encodeURIComponent(scope.id)}/attachments${id ? `/${encodeURIComponent(id)}` : ""}`;
}
/** Raw, bounded upload: no wire-log capture, no filename/content telemetry. */
export function uploadFile(
  scope: UploadScope,
  file: File,
  signal: AbortSignal,
  progress: (value: number) => void,
): Promise<NyxAgentAttachment> {
  if (!file.size || file.size > 20 * 1024 * 1024)
    return Promise.reject(new Error("Choose a file between 1 byte and 20 MB."));
  return new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest();
    xhr.open("POST", apiUrl(attachmentPath(scope)));
    xhr.withCredentials = true;
    xhr.timeout = 60_000;
    const config = buildFetchConfig({ method: "POST" });
    new Headers(config.headers).forEach((value, key) => {
      if (key.toLowerCase() !== "content-type")
        xhr.setRequestHeader(key, value);
    });
    xhr.setRequestHeader("content-type", "application/octet-stream");
    xhr.setRequestHeader("x-attachment-name", encodeURIComponent(file.name));
    xhr.upload.onprogress = (event) => {
      if (event.lengthComputable)
        progress(Math.min(99, Math.round((event.loaded / event.total) * 100)));
    };
    const abort = () => xhr.abort();
    signal.addEventListener("abort", abort, { once: true });
    xhr.onloadend = () => signal.removeEventListener("abort", abort);
    xhr.onerror = () =>
      reject(new Error("Upload failed. Check your connection and try again."));
    xhr.ontimeout = () =>
      reject(new Error("Upload timed out. Try a smaller file."));
    xhr.onabort = () => reject(new Error("Upload cancelled."));
    xhr.onload = () => {
      try {
        const body: unknown = JSON.parse(xhr.responseText);
        if (xhr.status < 200 || xhr.status >= 300) {
          const error = body as { message?: unknown };
          if (typeof error.message === "string" && /malformed or unsupported attachment/i.test(error.message)) {
            throw new Error(UNSUPPORTED_FORMAT_MESSAGE);
          }
          throw new Error(
            typeof error.message === "string"
              ? error.message
              : "Upload failed.",
          );
        }
        resolve(nyxAgentAttachmentSchema.parse(body));
      } catch (error) {
        reject(error);
      }
    };
    if (signal.aborted) {
      reject(new Error("Upload cancelled."));
      return;
    }
    xhr.send(file);
  });
}
export function removeUpload(scope: UploadScope, id: string) {
  return assistantJson(attachmentPath(scope, id), { method: "DELETE" });
}
