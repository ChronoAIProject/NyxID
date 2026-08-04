// Shared helpers for the pairing "action reservation" server-side
// latch. Both DisplayOnce panels (panels.tsx) and the ai-key flows
// (ai-key-panel.tsx, ai-key-auth-flows.tsx) call these — previously
// each file had its own copy, which risks divergence when the
// error-handling contract evolves. Centralized here.

import { ApiError, api } from "@/lib/api-client";

/**
 * Mark the destructive action as "about to run" on the server before
 * the caller invokes the mint/rotate/create API. The reservation
 * latch is the only server-visible signal that the destructive step
 * has started — if we don't record it, a subsequent refresh can
 * replay the step. So this call MUST succeed; any failure aborts
 * the destructive flow and surfaces a user-facing error. Treat it
 * as "fail closed" rather than "log and proceed".
 *
 * Semantics:
 *   - 2xx            → caller may proceed
 *   - 409 / 404      → stale-tab-specific message (pairing already
 *                      completed / started elsewhere / not found)
 *   - Other ApiError → generic "try again" message; the reservation
 *                      latch didn't land, so we can't safely run
 *                      the destructive step
 *   - Network error  → same as above — bail, let the user retry
 */
export async function reservePairingAction(pairingId: string): Promise<void> {
  try {
    await api.post(
      `/cli-pairings/${encodeURIComponent(pairingId)}/reserve-action`,
      {},
    );
  } catch (e) {
    if (e instanceof ApiError && (e.status === 409 || e.status === 404)) {
      throw new Error(
        "This pairing was already completed or started in another tab. Close this tab and check your CLI — if the CLI didn't finish the flow, run the command again for a fresh pairing.",
      );
    }
    // ANY other failure (5xx, network, timeout) aborts the flow.
    // Letting the destructive call run anyway would bypass the
    // replay guard because `action_started_at` wouldn't be set.
    const detail = e instanceof Error ? e.message : String(e);
    throw new Error(
      `Couldn't reserve this pairing with NyxID (${detail}). Try again; if the problem persists, cancel and re-run the CLI command.`,
    );
  }
}

/**
 * Undo a prior `reserve-action` so the user can retry a cancelled
 * OAuth / device-code sub-flow on the same pairing. The backend
 * service refuses to rewind Completed pairings, so this call is
 * safe even if it happens to race with a concurrent `/complete`.
 * Best-effort: any failure leaves the pairing locked and the user
 * re-runs the CLI (no data-integrity implication).
 */
export async function rewindPairingAction(pairingId: string): Promise<void> {
  try {
    await api.post(
      `/cli-pairings/${encodeURIComponent(pairingId)}/rewind-action`,
      {},
    );
  } catch {
    // No-op: rewind is purely a UX convenience.
  }
}

/**
 * Run a destructive API call that was gated by `reservePairingAction`
 * and rewind the reservation ONLY when the failure proves no side
 * effect committed. That means 4xx `ApiError` — validation errors,
 * not-found, conflicts — where the server rejected the request
 * before mutating state.
 *
 * CRITICAL: 5xx and network errors (timeouts, connection resets)
 * are AMBIGUOUS. The server may have committed the mutation and
 * then failed to return a response. Rewinding in that case would
 * let the user retry and create a duplicate resource while the
 * original sits orphaned from the CLI ack. So on those errors we
 * leave the reservation latched and let the user re-run the CLI
 * for a fresh pairing — the conservative choice that can't
 * double-mint.
 *
 * The inner call is re-thrown on failure so the caller's `catch`
 * can still surface the error to the user.
 */
export async function withRewindOnError<T>(
  pairingId: string,
  run: () => Promise<T>,
): Promise<T> {
  try {
    return await run();
  } catch (e) {
    // Only 4xx ApiErrors are safely retriable — they mean the
    // server rejected the input BEFORE committing. 5xx / network
    // errors are ambiguous so we keep the latch to prevent
    // duplicate-mint races.
    if (e instanceof ApiError && e.status >= 400 && e.status < 500) {
      await rewindPairingAction(pairingId);
    }
    throw e;
  }
}
