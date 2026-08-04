/**
 * Shared reading of `UserApiKey.status` for every surface that shows a
 * credential's health — the Studio key-detail page and the assistant's
 * connection modal.
 *
 * Both used to keep private copies of the same `statusVariant` switch and
 * `RECONNECTABLE_STATUSES` set, with a comment in one asking the reader to
 * keep it in sync with the other. That drifted: the modal rendered a bare
 * destructive badge for `failed` with no explanation and no way out, while
 * the detail page showed the row's `error_message` and a Reconnect button
 * for the exact same state.
 *
 * Display copy (label / meaning) lives in `STATUS_REGISTRY.credential`;
 * this module owns the behavioural questions that aren't display concerns:
 * can the user re-authorize out of this state, and what should the button
 * say.
 */
import { getStatusMeta, type StatusMeta } from "@/lib/status-contract";

/**
 * Statuses a fresh authorization can recover from. `revoked` is absent on
 * purpose — that credential is gone and the user adds a new connection
 * rather than repairing this one. `expired` is absent because the refresh
 * path, not the user, is what renews it; once refresh gives up the row
 * moves to `refresh_failed` and becomes reconnectable here.
 */
export const RECONNECTABLE_STATUSES: ReadonlySet<string> = new Set([
  "pending_auth",
  "refresh_failed",
  "failed",
]);

export function isReconnectableStatus(status: string): boolean {
  return RECONNECTABLE_STATUSES.has(status);
}

/**
 * `pending_auth` is a flow the user already started, so "Reconnect" would
 * misdescribe it — they're resuming, not repairing.
 */
export function reconnectLabel(status: string): string {
  return status === "pending_auth" ? "Continue authentication" : "Reconnect";
}

/** True when the status warrants an inline explanation rather than silence. */
export function isProblemStatus(status: string): boolean {
  return status !== "" && status !== "active";
}

/**
 * Registry lookup with a neutral fallback, so an unrecognised status added
 * by a newer backend renders as a readable badge instead of crashing or
 * showing an empty pill.
 */
export function credentialStatusMeta(status: string): StatusMeta {
  return (
    getStatusMeta("credential", status) ?? {
      label: status.replaceAll("_", " ") || "Unknown",
      variant: "secondary",
      tooltip: "NyxID does not recognise this credential status.",
    }
  );
}
