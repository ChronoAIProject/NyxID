import type { z } from "zod";
import type { connectLinkPreviewSchema } from "./connectLinkContracts";

type Preview = z.infer<typeof connectLinkPreviewSchema>;

export type ConnectButtonState = {
  label: string;
  /** The status line under the button. */
  detail?: string;
  /** Used, expired or cancelled links can no longer be opened. */
  disabled: boolean;
  tone: "ready" | "done" | "closed";
};

/**
 * What a hosted connect button says, from the link's live preview: the
 * service by name, whether it can still be opened, and how long it has left.
 * Without a preview yet, the reply's own wording stands.
 */
export function connectButtonState(
  fallbackLabel: string,
  preview: Preview | undefined,
  now = Date.now(),
): ConnectButtonState {
  if (!preview) return { label: fallbackLabel, disabled: false, tone: "ready" };
  const name = preview.service_name;
  const expired =
    preview.status === "expired" ||
    (preview.status === "pending" && Date.parse(preview.expires_at) <= now);
  if (preview.status === "completed")
    return { label: `${name} connected`, detail: "Connected", disabled: true, tone: "done" };
  if (preview.status === "cancelled")
    return {
      label: `Connect ${name}`,
      detail: "This link was cancelled. Ask NyxBot for a new one.",
      disabled: true,
      tone: "closed",
    };
  if (expired)
    return {
      label: `Connect ${name}`,
      detail: "This link expired. Ask NyxBot for a new one.",
      disabled: true,
      tone: "closed",
    };
  const minutes = Math.max(1, Math.ceil((Date.parse(preview.expires_at) - now) / 60_000));
  return {
    label: `Connect ${name}`,
    detail: `Waiting for you · link expires in ${minutes} min`,
    disabled: false,
    tone: "ready",
  };
}
