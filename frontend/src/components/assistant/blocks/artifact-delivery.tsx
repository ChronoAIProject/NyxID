import { useState } from "react";
import { assistantHttp } from "@/lib/assistant/assistant-http";
import type { ChatMessage } from "@/lib/assistant/chat-types";
import { attachmentFailure } from "@/lib/assistant/attachment-errors";

export function ArtifactDelivery({
  delivery,
}: {
  readonly delivery: NonNullable<ChatMessage["artifactDelivery"]>;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  if (delivery.state === "materialized" && !delivery.rejected?.length)
    return null;
  const copy =
    delivery.state === "source_verification_failed"
      ? "The generated result could not be verified. No files were attached and no saved-result recovery is available."
      : delivery.state === "artifact_too_large"
        ? "The generated result exceeded the supported response limit (13 MiB and eight files). No files were attached."
        : delivery.state === "materialized"
          ? `${delivery.count} verified file${delivery.count === 1 ? "" : "s"} attached. Some generated files could not be delivered.`
          : delivery.state === "result_expired"
            ? "The generated result expired before delivery. Ask for a new execution to generate it again."
            : delivery.state === "cancelled" || delivery.state === "discarded"
              ? "File delivery was cancelled."
              : delivery.state === "delivery_pending"
                ? "The result is verified. Attachment delivery is pending."
                : delivery.state === "delivery_capacity_exhausted"
                  ? "The result is verified. This reply reached its attachment limit; the files have not been delivered."
                  : "The result is verified, but attachment delivery failed.";
  async function recover() {
    if (busy) return;
    setBusy(true);
    setError(undefined);
    try {
      await assistantHttp(delivery.endpoint, { method: "POST" });
      window.location.reload();
    } catch (failure) {
      setError(attachmentFailure(failure));
      setBusy(false);
    }
  }
  return (
    <div
      role="status"
      className="mb-2 space-y-1 rounded-lg border border-hairline p-3 text-xs text-muted-foreground"
    >
      <p>{copy}</p>
      {!!delivery.rejected?.length && (
        <ul>
          {delivery.rejected.map((file) => (
            <li key={file.index}>
              File {file.index + 1} ({file.filename}): generated but not
              delivered —{" "}
              {file.reason === "too_large"
                ? "exceeds the supported file or accepted-total size limit"
                : file.reason === "unavailable"
                  ? "the producer could not retrieve this file"
                  : file.reason === "missing"
                    ? "the saved file is no longer available"
                    : "the returned file failed validation"}
              .
            </li>
          ))}
        </ul>
      )}
      {delivery.recoveryAvailable && (
        <button
          type="button"
          disabled={busy}
          onClick={() => void recover()}
          className="underline disabled:opacity-50"
        >
          {busy ? "Recovering attachments…" : "Retry attachment delivery"}
        </button>
      )}
      {error && <p>{error}</p>}
    </div>
  );
}
