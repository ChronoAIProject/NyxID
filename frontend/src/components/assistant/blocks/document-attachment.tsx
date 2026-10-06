import { ApiError } from "@/lib/api-client";
import { useState } from "react";
import { Download, FileText } from "lucide-react";
import { assistantHttp } from "@/lib/assistant/assistant-http";
import type { ChatImage } from "@/lib/assistant/chat-types";

/** Documents download only on demand; transcript rendering never loads every file. */
export function DocumentAttachment({
  attachment,
}: {
  readonly attachment: ChatImage;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(false);
  const [expired, setExpired] = useState(false);
  async function download() {
    if (busy) return;
    setBusy(true);
    setError(false);
    try {
      const response = await assistantHttp(attachment.endpoint);
      const url = URL.createObjectURL(await response.blob());
      const link = document.createElement("a");
      link.href = url;
      link.download = attachment.label;
      link.rel = "noopener noreferrer";
      link.click();
      setTimeout(() => URL.revokeObjectURL(url), 1000);
    } catch (error) {
      setError(true);
      setExpired(error instanceof ApiError && error.errorCode === 12101);
    } finally {
      setBusy(false);
    }
  }
  return (
    <div className="min-w-0 space-y-1">
      <button
        type="button"
        onClick={() => void download()}
        disabled={busy}
        className="flex max-w-full items-center gap-2 rounded-lg border border-hairline bg-card px-3 py-2 text-xs disabled:opacity-50"
      >
        <FileText className="h-4 w-4 shrink-0" />
        <span className="truncate">{attachment.label}</span>
        <Download
          className="h-3 w-3 shrink-0 text-muted-foreground"
          aria-label={busy ? "Downloading" : "Download"}
        />
      </button>
      {error && (
        <p role="status" className="text-xs text-muted-foreground">
          {expired ? "Attachment expired per retention policy. Upload it again to continue." : "Attachment unavailable or expired. Upload it again."}
        </p>
      )}
    </div>
  );
}
