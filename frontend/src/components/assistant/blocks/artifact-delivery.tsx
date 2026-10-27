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
  if (delivery.state === "materialized") return null;
  const copy =
    delivery.state === "result_expired"
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
