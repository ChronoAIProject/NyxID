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
  readonly conversationId?: string;
}
export const UPLOAD_ACCEPT =
  ".png,.jpg,.jpeg,.gif,.webp,.pdf,.docx,.txt,.md,.markdown,.csv,.json";

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
