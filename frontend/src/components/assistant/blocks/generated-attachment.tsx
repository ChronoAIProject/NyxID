import { useEffect, useRef, useState } from "react";
import { Download } from "lucide-react";
import { attachmentFailure } from "@/lib/assistant/attachment-errors";
import { acquireGeneratedAttachment } from "@/lib/assistant/generated-attachment-cache";
import { Button } from "@/components/ui/button";
import type { ChatImage } from "@/lib/assistant/chat-types";

/** Durable Sandbox files share one authenticated fetch for preview and download. */
export function GeneratedAttachment({
  attachment,
}: {
  readonly attachment: ChatImage;
}) {
  const container = useRef<HTMLDivElement>(null);
  const downloaded = useRef(false);
  const [visible, setVisible] = useState(
    () => typeof IntersectionObserver === "undefined",
  );
  const [height, setHeight] = useState<number>();
  const [url, setUrl] = useState<string>();
  useEffect(() => {
    if (!container.current || typeof IntersectionObserver === "undefined") return;
    const observer = new IntersectionObserver(([entry]) => {
      const entered = entry?.isIntersecting ?? false;
      setVisible(entered);
      if (!entered) {
        const height = entry?.boundingClientRect.height ?? 0;
        if (height > 0) setHeight((previous) => Math.max(previous ?? 0, height));
        setUrl(undefined);
      }
    });
    observer.observe(container.current);
    return () => observer.disconnect();
  }, []);
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
  const preview =
    attachment.origin === "generated_artifact" &&
    ((attachment.previewKind === "image" &&
      ["image/png", "image/jpeg", "image/gif", "image/webp"].includes(
        attachment.contentType,
      )) ||
      (attachment.previewKind === "video" &&
        attachment.contentType === "video/mp4"))
      ? attachment.previewKind
      : "download";
  const inline = preview === "image" || preview === "video";
  useEffect(() => {
    if (!visible || expired || (!inline && !downloadRequested)) return;
    const lease = acquireGeneratedAttachment(
      attachment.endpoint,
      attachment.expiresAt,
      attachment.size,
    );
    let active = true;
    lease.ready
      .then((objectUrl) => {
        if (!active) return;
        setUrl(objectUrl);
        if (!inline && !downloaded.current) {
          downloaded.current = true;
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
      lease.release();
    };
  }, [
    attachment.endpoint,
    attachment.size,
    visible,
    expired,
    attempt,
    inline,
    downloadRequested,
    name,
    attachment.expiresAt,
  ]);
  return (
    <div
      ref={container}
      style={{ minHeight: height }}
      className="min-w-0 max-w-full space-y-2 rounded-lg border border-hairline bg-card p-3"
    >
      <p className="break-words type-group-label">{name}</p>
      <p className="text-12 text-muted-foreground">
        {attachment.contentType} ·{" "}
        {attachment.size?.toLocaleString() ?? "Unknown"} bytes
        {attachment.resultVersion
          ? ` · Revision ${attachment.resultVersion}`
          : ""}
      </p>
      {expired ? (
        <p role="status" className="text-12 text-muted-foreground">
          This generated file has expired. Ask for a new result to generate it
          again.
        </p>
      ) : error ? (
        <div role="status" className="space-y-1 text-12 text-muted-foreground">
          <p>{error}</p>
          <Button
            variant="link"
            size="sm"
            type="button"
            onClick={() => {
              setError(undefined);
              setAttempt((n) => n + 1);
            }}
          >
            Load attachment again
          </Button>
        </div>
      ) : !inline && !downloadRequested ? (
        <>
          <p role="status" className="text-12 text-muted-foreground">
            Preview unavailable for this type. Download the file to open it.
          </p>
          <Button
            variant="link"
            size="sm"
            type="button"
            onClick={() => setDownloadRequested(true)}
          >
            <Download className="h-3.5 w-3.5" aria-hidden="true" />
            Download {name}
          </Button>
        </>
      ) : !url ? (
        <p role="status" className="text-12 text-muted-foreground">
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
            <p role="status" className="text-12 text-muted-foreground">
              {decodeFailed
                ? "Your browser could not decode this preview. You can still download the verified file."
                : "Preview unavailable for this type. Download the file to open it."}
            </p>
          )}
          <Button asChild variant="link" size="sm">
            <a href={url} download={name}>
              <Download className="h-3.5 w-3.5" aria-hidden="true" />
              Download {name}
            </a>
          </Button>
        </>
      )}
      {attachment.expiresAt && (
        <p className="text-12 text-muted-foreground">
          Available until {new Date(attachment.expiresAt).toLocaleString()}
        </p>
      )}
    </div>
  );
}
