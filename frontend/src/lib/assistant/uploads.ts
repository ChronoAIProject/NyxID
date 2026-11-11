import { apiUrl, buildFetchConfig } from "@/lib/api-client";
import { assistantJson } from "@/lib/assistant/assistant-http";
import { captureNyxbotError, httpErrorCode, responseCorrelation } from "./error-telemetry";
import { getAssistantIdentityRevision } from "./identity";
import type { NyxbotErrorCode } from "@/lib/telemetry-schema";
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
  const identityRevision = getAssistantIdentityRevision();
  const startedAt = Date.now();
  if (!file.size || file.size > 20 * 1024 * 1024)
    return Promise.reject(new Error("Choose a file between 1 byte and 20 MB."));
  // The preview fixture accepts raw files without sending them to a backend.
  // Vite removes this branch from production builds.
  if ((import.meta.env.DEV || import.meta.env.MODE === "test") && globalThis.__nyxidAssistantHttpMock) {
    const mock = globalThis.__nyxidAssistantHttpMock;
    return (async () => {
      if (signal.aborted) throw new Error("Upload cancelled.");
      const response = await mock({
        endpoint: attachmentPath(scope),
        init: { method: "POST", body: file, signal, headers: { "x-attachment-name": encodeURIComponent(file.name) } },
      });
      if (signal.aborted) throw new Error("Upload cancelled.");
      if (!response) throw new Error("Preview upload route not found.");
      const body: unknown = await response.json();
      if (!response.ok) throw new Error((body as { message?: string }).message ?? "Upload failed.");
      progress(100);
      return nyxAgentAttachmentSchema.parse(body);
    })();
  }
  return new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest();
    const endpoint = attachmentPath(scope);
    let settled = false;
    const rejectFailure = (error: unknown, code: NyxbotErrorCode) => {
      if (settled) return;
      settled = true;
      if (!signal.aborted && getAssistantIdentityRevision() === identityRevision) {
        try {
          const headers = new Headers();
          for (const name of ["x-request-id", "x-correlation-id", "X-NyxID-Trace-Id", "x-trace-id"]) {
            const value = xhr.getResponseHeader(name);
            if (value) headers.set(name, value);
          }
          captureNyxbotError({
            code, engine: "nyxagent", stage: "request", endpoint,
            status: xhr.status || undefined,
            ...responseCorrelation({ headers }),
            durationMs: Date.now() - startedAt,
          }, error);
        } catch { /* Diagnostics cannot prevent the upload error from settling. */ }
      }
      reject(error);
    };
    xhr.open("POST", apiUrl(endpoint));
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
      rejectFailure(new Error("Upload failed. Check your connection and try again."), "NYXBOT_NETWORK_ERROR");
    xhr.ontimeout = () =>
      rejectFailure(new Error("Upload timed out. Try a smaller file."), "NYXBOT_START_TIMEOUT");
    xhr.onabort = () => {
      settled = true;
      reject(new Error("Upload cancelled."));
    };
    xhr.onload = () => {
      if (settled) return;
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
        const attachment = nyxAgentAttachmentSchema.parse(body);
        settled = true;
        resolve(attachment);
      } catch (error) {
        rejectFailure(error, xhr.status >= 200 && xhr.status < 300
          ? "NYXBOT_PROTOCOL_ERROR" : httpErrorCode(xhr.status));
      }
    };
    if (signal.aborted) {
      signal.removeEventListener("abort", abort);
      settled = true;
      reject(new Error("Upload cancelled."));
      return;
    }
    try {
      xhr.send(file);
    } catch (error) {
      signal.removeEventListener("abort", abort);
      rejectFailure(error, "NYXBOT_NETWORK_ERROR");
    }
  });
}
export function removeUpload(scope: UploadScope, id: string) {
  return assistantJson(attachmentPath(scope, id), { method: "DELETE" });
}
