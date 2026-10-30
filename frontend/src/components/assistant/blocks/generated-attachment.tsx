import { useEffect, useState } from "react";
import { Download } from "lucide-react";
import { attachmentFailure } from "@/lib/assistant/attachment-errors";
import { assistantHttp } from "@/lib/assistant/assistant-http";
import type { ChatImage } from "@/lib/assistant/chat-types";

/** Durable Sandbox files share one authenticated fetch for preview and download. */
export function GeneratedAttachment({
  attachment,
}: {
  readonly attachment: ChatImage;
}) {
  const [url, setUrl] = useState<string>();
  const [error, setError] = useState<string>();
  const [decodeFailed, setDecodeFailed] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const [downloadRequested, setDownloadRequested] = useState(false);
  const [locallyExpired, setLocallyExpired] = useState(() =>
    Boolean(
      attachment.expiresAt && Date.parse(attachment.expiresAt) <= Date.now(),
    ),
  );
  const expired = attachment.expired || locallyExpired;
  useEffect(() => {
    const deadline = Date.parse(attachment.expiresAt ?? "");
    let timeout: ReturnType<typeof setTimeout> | undefined;
    const checkExpiry = () => {
      if (!Number.isFinite(deadline)) {
        setLocallyExpired(false);
        return;
      }
      const remaining = deadline - Date.now();
      setLocallyExpired(remaining <= 0);
      if (remaining > 0) {
        timeout = setTimeout(checkExpiry, Math.min(remaining, 2_147_483_647));
      }
    };
    timeout = setTimeout(checkExpiry, 0);
    return () => clearTimeout(timeout);
  }, [attachment.expiresAt]);
  const name = attachment.filename ?? "Generated file";
  const preview = attachment.previewKind;
  const inline = preview === "image" || preview === "video";
  useEffect(() => {
    if (expired || (!inline && !downloadRequested)) return;
    const controller = new AbortController();
    let objectUrl: string | undefined;
    let active = true;
    assistantHttp(attachment.endpoint, { signal: controller.signal })
      .then((response) => response.blob())
      .then((blob) => {
        if (!active) return;
        objectUrl = URL.createObjectURL(blob);
        setUrl(objectUrl);
        if (!inline) {
          const link = document.createElement("a");
          link.href = objectUrl;
          link.download = name;
          link.rel = "noopener noreferrer";
          link.click();
        }
      })
      .catch((failure: unknown) => {
        if (active) setError(attachmentFailure(failure));
      });
    return () => {
      active = false;
      controller.abort();
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [attachment.endpoint, expired, attempt, inline, downloadRequested, name]);
  return (
    <div className="min-w-0 max-w-full space-y-2 rounded-lg border border-hairline bg-card p-3">
      <p className="break-words text-xs font-medium">{name}</p>
      <p className="text-11 text-muted-foreground">
        {attachment.contentType} ·{" "}
        {attachment.size?.toLocaleString() ?? "Unknown"} bytes
        {attachment.resultVersion
          ? ` · Revision ${attachment.resultVersion}`
          : ""}
      </p>
      {expired ? (
        <p role="status" className="text-xs text-muted-foreground">
          This generated file has expired. Ask for a new result to generate it
          again.
        </p>
      ) : error ? (
        <div role="status" className="space-y-1 text-xs text-muted-foreground">
          <p>{error}</p>
          <button
            type="button"
            onClick={() => {
              setError(undefined);
              setAttempt((n) => n + 1);
            }}
            className="underline"
          >
            Load attachment again
          </button>
        </div>
      ) : !inline && !downloadRequested ? (
        <>
          <p role="status" className="text-xs text-muted-foreground">
            Preview unavailable for this type. Download the file to open it.
          </p>
          <button
            type="button"
            onClick={() => setDownloadRequested(true)}
            className="inline-flex items-center gap-1.5 text-xs underline"
          >
            <Download className="h-3.5 w-3.5" aria-hidden="true" />
            Download {name}
          </button>
        </>
      ) : !url ? (
        <p role="status" className="text-xs text-muted-foreground">
          Loading generated file…
        </p>
      ) : (
        <>
          {preview === "video" && !decodeFailed && (
            <video
              controls
              preload="metadata"
              aria-label={`Preview ${name}`}
              src={url}
              onError={() => setDecodeFailed(true)}
              className="max-h-80 max-w-full rounded-lg"
            />
          )}
          {preview === "image" && !decodeFailed && (
            <a
              href={url}
              target="_blank"
              rel="noopener noreferrer"
              aria-label={`Open full preview of ${name}`}
            >
              <img
                src={url}
                alt={name}
                onError={() => setDecodeFailed(true)}
                className="max-h-80 max-w-full rounded-lg object-contain"
              />
            </a>
          )}
          {(!inline || decodeFailed) && (
            <p role="status" className="text-xs text-muted-foreground">
              {decodeFailed
                ? "Your browser could not decode this preview. You can still download the verified file."
                : "Preview unavailable for this type. Download the file to open it."}
            </p>
          )}
          <a
            href={url}
            download={name}
            className="inline-flex items-center gap-1.5 text-xs underline"
          >
            <Download className="h-3.5 w-3.5" aria-hidden="true" />
            Download {name}
          </a>
        </>
      )}
      {attachment.expiresAt && (
        <p className="text-11 text-muted-foreground">
          Available until {new Date(attachment.expiresAt).toLocaleString()}
        </p>
      )}
    </div>
  );
}
