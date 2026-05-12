// OAuth and device-code sub-flows for the ai-key pairing panel.
//
// Ported from cli/src/wizard/assets/wizard.js::runOauthFlow and
// runDeviceCodeFlow. Same contract, different transport: whereas the
// local-server wizard runs on 127.0.0.1 and proxies backend calls
// through its own axum process, the pair page runs on the regular
// NyxID origin and calls the backend directly via the cookie-auth
// `api` client.
//
// Both flows follow the same three-stage shape:
//   1. POST /keys → placeholder UserService with status=pending_auth
//   2. Initiate provider auth (popup for OAuth / user code + URL for
//      device) and wait for the provider side to grant access
//   3. GET /keys/{id} to read the now-active key, hand the summary
//      to `onSuccess`, and let the parent fire the pairing ack
//
// The parent passes `providerId`, `slug`, and `label`; these flows
// own the rest — placeholder creation, polling, and cleanup.

import { useEffect, useRef, useState } from "react";
import { Button, ButtonIcon } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { ApiError, api } from "@/lib/api-client";
import { Copy, ExternalLink, Loader2 } from "lucide-react";
import type { AiKeyPairingSuccess } from "./ai-key-confirm-panel";
import {
  reservePairingAction,
  rewindPairingAction,
} from "@/pages/cli-pair/reserve-action";
import { pollOAuthKeyUntilActive } from "./auth-flow-polling";

interface FlowProps {
  readonly providerId: string;
  readonly slug: string;
  readonly label: string;
  readonly nodeId?: string;
  readonly targetOrgId?: string | null;
  /**
   * User's `--endpoint-url` override from the CLI prefill (e.g.
   * self-hosted OpenClaw instance URL). Plumbed into the
   * placeholder-key creation so the final connected service points
   * at the user's host, not the catalog default.
   */
  readonly endpointUrl?: string;
  /**
   * Pairing id for the enclosing `/cli-pair` session. The flow calls
   * `reserve-action` on this record right before creating the
   * `pending_auth` placeholder so a concurrent re-claim knows the
   * destructive step has started.
   */
  readonly pairingId: string;
  /**
   * `system` / `user` / `both`. OAuth flows only — when `user` or
   * `both`, the flow runs a pre-step that PUTs the user's own
   * `client_id` / `client_secret` onto the provider before redirect.
   */
  readonly credentialMode?: string;
  /** Admin-provided docs URL rendered in the OAuth-credentials step. */
  readonly documentationUrl?: string;
  readonly onSuccess: (result: AiKeyPairingSuccess) => void;
  /**
   * Called when the user bails before success so the parent can reset
   * to the confirm form. Placeholder-key cleanup is fire-and-forget
   * — a pending_auth record expires on its own.
   */
  readonly onCancel: () => void;
}

interface UserCredentialsMetadata {
  readonly has_credentials: boolean;
}

/**
 * Returns true when this OAuth provider expects the end user to
 * register their own OAuth app and supply the resulting client
 * credentials before NyxID can redirect to the authorization URL.
 */
function needsUserOAuthCredentials(credentialMode: string | undefined): boolean {
  const m = (credentialMode ?? "system").toLowerCase();
  return m === "user" || m === "both";
}

interface PlaceholderKeyResponse {
  readonly id: string;
  readonly status: string;
  readonly slug: string;
  readonly label: string;
}

interface ActiveKeyResponse {
  readonly id: string;
  readonly slug: string;
  readonly label: string;
  readonly status: string;
}

interface InitiateOAuthResponse {
  readonly authorization_url: string;
}

interface DeviceCodeInitiateResponse {
  readonly user_code: string;
  readonly verification_uri: string;
  readonly state: string;
  readonly interval: number;
}

interface DeviceCodePollResponse {
  readonly status?: string;
  readonly interval?: number;
  readonly access_token?: string;
}

/**
 * Create a placeholder UserService that will be flipped to
 * `active` by a successful OAuth / device-code callback.
 *
 * Callers flip `placeholderCreateSentRef` to `true` BEFORE calling
 * this so `cancelAndCleanup` can't rewind the pairing while a
 * create is in flight. On a definitive 4xx (validation error —
 * e.g. bad endpoint URL, slug not in the user's catalog) we know
 * the server committed nothing, so we clear the flag back to
 * `false` before rethrowing. That lets `cancelAndCleanup` rewind
 * the reserve-action latch and the user can retry on the same
 * pairing code instead of waiting for the 15-min TTL. 5xx /
 * network errors keep the flag set because the request may have
 * landed and committed a placeholder we'll never observe.
 */
async function createPlaceholderKey(
  slug: string,
  label: string,
  createSentRef: { current: boolean },
  nodeId?: string,
  endpointUrl?: string,
  targetOrgId?: string | null,
): Promise<PlaceholderKeyResponse> {
  const body: Record<string, unknown> = {
    service_slug: slug,
    label,
  };
  if (nodeId) body.node_id = nodeId;
  if (targetOrgId) body.target_org_id = targetOrgId;
  // Preserve the user's `--endpoint-url` override. Without this,
  // OAuth/device-code pairings for self-hosted providers (e.g.
  // OpenClaw) would bind the final service to the catalog default
  // URL instead of the user's own instance. The terminal path
  // plumbs this through `run_oauth_add`; the pairing path needs
  // the same behavior.
  const trimmed = endpointUrl?.trim();
  if (trimmed) body.endpoint_url = trimmed;
  try {
    return await api.post<PlaceholderKeyResponse>("/keys", body);
  } catch (e) {
    // On a clean 4xx the server rejected the request before any
    // placeholder write. We can safely tell the caller "no side
    // effect" so rewind becomes possible. Anything else (5xx,
    // fetch failure, abort) stays ambiguous.
    if (e instanceof ApiError && e.status >= 400 && e.status < 500) {
      createSentRef.current = false;
    }
    throw e;
  }
}


/**
 * Best-effort: fire a `pending_auth` key cleanup when the page is
 * about to unload (refresh / navigate-away / tab close). Uses
 * `keepalive: true` so the request survives unload; most browsers
 * honor this for sub-64 KB bodies.
 *
 * The `only_if_pending=true` query param makes the server skip
 * the revoke if the provider callback flipped the key to `active`
 * before the DELETE landed. Without this guard, closing the tab
 * during the tail of an OAuth / device-code flow could revoke a
 * freshly-authorized service. The check is server-side (cheap and
 * race-free) precisely because the client has no time for a
 * round-trip GET during unload.
 */
function abandonPlaceholderKeyOnUnload(keyId: string): void {
  if (typeof window === "undefined") return;
  try {
    // Use fetch with keepalive rather than navigator.sendBeacon:
    // sendBeacon only supports POST, and we need DELETE. Same cookie
    // credentials the rest of the app uses.
    //
    // Only the placeholder DELETE fires here. We intentionally do
    // NOT rewind the pairing's `action_started_at` latch. The
    // keepalive DELETE can be dropped by the browser (most retain
    // unload requests, but it's best-effort), and pairing a
    // possibly-surviving placeholder with a succeeded rewind would
    // let the next claim create a duplicate placeholder while the
    // first remains live (or the provider callback later flips it
    // to `active`). The pairing's 15-min TTL reclaims the latch in
    // the worst case, and users can always rerun the CLI for a
    // fresh code — both are strictly safer than the duplicate-mint
    // risk.
    void fetch(
      `/api/v1/keys/${encodeURIComponent(keyId)}?only_if_pending=true`,
      {
        method: "DELETE",
        credentials: "include",
        keepalive: true,
      },
    );
  } catch {
    // No-op: nothing more we can do from a page that's about to unload.
  }
}

/**
 * Best-effort keepalive cancel for when the page unloads while
 * `createPlaceholderKey()` is still in flight (so `keyIdRef` is
 * still `null` but the server may be about to commit a
 * `pending_auth` placeholder). We can't target the placeholder —
 * we don't yet have its id — so instead we cancel the pairing
 * itself: the CLI's next poll returns Cancelled and the waiting
 * command exits promptly instead of latching until TTL. Any
 * placeholder that lands after the cancel is an orphan in the
 * user's key list, visible on the AI Services page for manual
 * cleanup. This is the least-bad option for this specific race;
 * firing a bare rewind would leave the pairing open to a replay
 * that creates a duplicate placeholder, and we have no way to
 * target the unseen-by-the-client placeholder from inside a
 * keepalive request.
 */
function cancelPairingOnUnload(pairingId: string): void {
  if (typeof window === "undefined") return;
  try {
    void fetch(
      `/api/v1/cli-pairings/${encodeURIComponent(pairingId)}/cancel`,
      {
        method: "POST",
        credentials: "include",
        keepalive: true,
        headers: { "Content-Type": "application/json" },
        body: "{}",
      },
    );
  } catch {
    // No-op: we're unloading anyway.
  }
}

/**
 * Best-effort companion to `abandonPlaceholderKeyOnUnload`: fires a
 * keepalive POST `/cli-pairings/{id}/complete` with the reconstructed
 * ai-key ack. Mutually exclusive with the DELETE in practice because
 * the server validates that the referenced service is `active`
 * before accepting (see `verify_ai_key_ack_service_active`):
 *
 *   - Placeholder still `pending_auth`: DELETE succeeds (revokes),
 *     /complete rejected by the server-side active check. Pairing
 *     stays `claimed` → CLI times out normally. No false success.
 *   - Placeholder flipped to `active` (OAuth callback beat
 *     `beforeunload`): DELETE skipped by `only_if_pending`,
 *     /complete accepted. Pairing transitions to Completed → CLI
 *     unblocks cleanly without waiting for the next poll tick.
 *
 * This closes the race where a user authorizes in the provider
 * popup and immediately refreshes / closes the pair tab before
 * `pollUntilActive` notices the new `active` state. Without this,
 * the pairing would stay `claimed && action_started` until TTL
 * even though the credential is already live.
 */
function tryCompleteAiKeyOnUnload(
  pairingId: string,
  keyId: string,
  slug: string,
  label: string,
): void {
  if (typeof window === "undefined") return;
  try {
    void fetch(
      `/api/v1/cli-pairings/${encodeURIComponent(pairingId)}/complete`,
      {
        method: "POST",
        credentials: "include",
        keepalive: true,
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          ack: {
            acknowledged: true,
            service_id: keyId,
            slug,
            label,
          },
        }),
      },
    );
  } catch {
    // Best-effort — if this fails, the CLI falls back to a normal
    // timeout and the user re-runs.
  }
}

/**
 * Discriminated outcome of `abandonPlaceholderKey`.
 *
 * - `deleted`: the placeholder was still `pending_auth` and the
 *   server revoked it. Safe to rewind the pairing.
 * - `active`: the provider callback won the race and flipped the
 *   placeholder to `active` before our DELETE landed. The server
 *   skipped the revoke; `key` is the freshly-authorized record the
 *   caller should complete the flow with instead of cancelling.
 * - `unknown`: key id was null, or the DELETE failed for a reason
 *   we can't classify (network, 5xx). Caller must NOT rewind.
 */
type AbandonOutcome =
  | { readonly kind: "deleted" }
  | { readonly kind: "active"; readonly key: ActiveKeyResponse }
  | { readonly kind: "unknown" };

/**
 * Delete a placeholder (pending_auth) UserService created during an
 * OAuth / device-code sub-flow that the user then abandoned.
 *
 * Race-free via the server-side `only_if_pending=true` guard: a
 * single DELETE either revokes the placeholder (`deleted: true`)
 * or skips it because the provider callback already flipped the
 * credential to `active` (`deleted: false`). No GET-then-DELETE
 * window where a newly authorized service can be killed. Matches
 * the local wizard's `pendingPlaceholderKeyId` cleanup semantics
 * (`cli/src/wizard/assets/wizard.js`).
 *
 * Callers MUST NOT rewind the pairing when the outcome is `active`
 * or `unknown`: if the key is active the pairing is effectively
 * Completed-equivalent and the caller should finish the flow
 * through `onSuccess`; if the state is unknown we can't prove no
 * side effect, and clearing `action_started_at` would let a
 * replay mint a duplicate service.
 */
async function abandonPlaceholderKey(
  keyId: string | null,
): Promise<AbandonOutcome> {
  if (!keyId) return { kind: "unknown" };
  try {
    const res = await api.delete<{ readonly deleted?: boolean }>(
      `/keys/${encodeURIComponent(keyId)}?only_if_pending=true`,
    );
    if (res.deleted === true) return { kind: "deleted" };
    // Server skipped because the key is no longer pending_auth.
    // That covers TWO cases:
    //   (a) Provider callback flipped the placeholder to
    //       `active` — we can complete the flow with this key.
    //   (b) Another tab (or a prior cleanup on this same tab)
    //       already revoked the placeholder. The GET below will
    //       return `status: "revoked"` and we must NOT treat
    //       that as success or the caller would try to
    //       `/complete` the pairing with a dead service.
    // So read the live record and only claim "active" when the
    // status actually matches. Any other status (revoked,
    // expired, refresh_failed, ...) falls through to
    // `unknown` → the caller runs the normal cancel path.
    try {
      const live = await api.get<ActiveKeyResponse>(
        `/keys/${encodeURIComponent(keyId)}`,
      );
      if (live.status === "active") {
        return { kind: "active", key: live };
      }
      return { kind: "unknown" };
    } catch {
      // Unable to confirm the active shape — treat as unknown
      // so the caller doesn't rewind on uncertain state.
      return { kind: "unknown" };
    }
  } catch {
    // Best-effort cleanup. Network / 5xx errors leave the key in
    // an unknown state — the 15-minute pairing TTL reclaims the
    // latch in the worst case.
    return { kind: "unknown" };
  }
}

/**
 * SPA-navigation unmount cleanup. Called from the effect cleanup
 * when the user leaves `/cli/pair` via router back or an in-app
 * link. Different from the `beforeunload` path: here we can
 * actually await fetches and inspect their responses, which lets
 * us make the right call in each race:
 *
 *   - Wait for in-flight `createPlaceholderKey` to settle before
 *     reading the create-sent / key-id refs. A clean 4xx inside
 *     that function resets `placeholderCreateSentRef` (no side
 *     effect); a late success sets `keyIdRef`. Snapshotting
 *     before the await would miss both of those transitions and
 *     strand a pairing that is actually cleanly recoverable.
 *
 *   - `DELETE /keys/{id}?only_if_pending=true` returns a
 *     structured `{deleted: bool}`. Three outcomes:
 *       `deleted: true`  → safe to rewind (if reserved).
 *       `deleted: false` → provider callback already flipped to
 *                          active. Re-read the key, POST
 *                          `/complete` with the ai-key ack so
 *                          the CLI unblocks. Leaving this
 *                          latched would let the CLI time out
 *                          despite the credential being live.
 *       network/5xx      → conservative: rewind only when we
 *                          can prove `createPlaceholderKey` never
 *                          went out.
 *
 *   - No placeholder (no keyId): rewind iff we held a
 *     reservation and no create was ever sent.
 *
 * Refs are passed as inputs (rather than snapshotted at call
 * time) so the post-`await` reads see the up-to-date values
 * after any in-flight request settles. This is safe because
 * refs outlive the component unmount.
 */
async function releaseServerStateOnUnmount(
  pairingId: string,
  keyIdRef: React.MutableRefObject<string | null>,
  reservedRef: React.MutableRefObject<boolean>,
  placeholderCreateSentRef: React.MutableRefObject<boolean>,
  placeholderCreateInFlightRef: React.MutableRefObject<Promise<unknown> | null>,
): Promise<void> {
  // Wait for any in-flight `createPlaceholderKey` to resolve
  // BEFORE reading the post-await ref values. This is the
  // difference between "correct cleanup" and "latched for 15
  // minutes": a late 4xx will reset `placeholderCreateSentRef`
  // (no side effect → safe to rewind), and a late success will
  // populate `keyIdRef` (so we can DELETE the actual placeholder
  // instead of just leaving it orphaned).
  const inFlight = placeholderCreateInFlightRef.current;
  if (inFlight) {
    try {
      await inFlight;
    } catch {
      // Error path already handled by `createPlaceholderKey`
      // (resets sent-ref on 4xx); other failures leave the ref
      // set and we fall through to the conservative branch.
    }
  }

  const keyId = keyIdRef.current;
  const reserved = reservedRef.current;
  const createWasSent = placeholderCreateSentRef.current;
  keyIdRef.current = null;
  reservedRef.current = false;

  let canRewind = reserved && !createWasSent;
  if (keyId) {
    try {
      const res = await api.delete<{ readonly deleted?: boolean }>(
        `/keys/${encodeURIComponent(keyId)}?only_if_pending=true`,
      );
      if (res.deleted === true && reserved) {
        canRewind = true;
      } else if (res.deleted === false) {
        // Provider callback won the race: the placeholder is now
        // an active credential. The user has a real service,
        // just on a different tab than where the pairing page
        // lives. Close the loop with the CLI by posting
        // `/complete` with the ai-key ack — otherwise the CLI
        // times out despite successful setup. Don't rewind; the
        // pairing is completing, not restarting.
        try {
          const active = await api.get<ActiveKeyResponse>(
            `/keys/${encodeURIComponent(keyId)}`,
          );
          await api.post(
            `/cli-pairings/${encodeURIComponent(pairingId)}/complete`,
            {
              ack: {
                acknowledged: true as const,
                service_id: active.id,
                slug: active.slug,
                label: active.label,
              },
            },
          );
        } catch {
          // Best-effort: if we can't read the active record or
          // post complete, the CLI's poll will eventually time
          // out. Either way no server-side state is harmed.
        }
        return;
      } else {
        canRewind = false;
      }
    } catch {
      // Network / 5xx. Preserve current `canRewind`: we only
      // rewind if nothing was ever sent to `POST /keys` (which
      // would mean no placeholder could be latched server-side
      // either).
    }
  }
  if (canRewind) {
    try {
      await rewindPairingAction(pairingId);
    } catch {
      // Best-effort — TTL reclaims the latch eventually.
    }
  }
}


// ── OAuth ────────────────────────────────────────────────────────────

export function OAuthFlow({
  providerId,
  slug,
  label,
  nodeId,
  targetOrgId,
  endpointUrl,
  pairingId,
  credentialMode,
  documentationUrl,
  onSuccess,
  onCancel,
}: FlowProps) {
  // Distinct phase for the user-OAuth-app credential sub-step. When
  // `credentialMode` allows user credentials we first GET the existing
  // metadata; if already set we skip straight to the OAuth redirect,
  // otherwise we gate on the user pasting client_id / client_secret.
  const [phase, setPhase] = useState<
    "checking-credentials"
    | "needs-credentials"
    | "saving-credentials"
    | "starting"
    | "waiting"
    | "done"
    | "error"
  >(needsUserOAuthCredentials(credentialMode) ? "checking-credentials" : "starting");
  const [error, setError] = useState<string | null>(null);
  const [authUrl, setAuthUrl] = useState<string | null>(null);
  const [clientId, setClientId] = useState("");
  const [clientSecret, setClientSecret] = useState("");
  const keyIdRef = useRef<string | null>(null);
  const cancelledRef = useRef(false);
  // Set to `true` once THIS tab has successfully latched
  // `reservePairingAction` on the server. Used to gate the rewind
  // call in `cancelAndCleanup` so a losing tab (409 on reserve)
  // can't clear a winning tab's latch and enable a replay. If
  // this tab never reserved, cancel just tears down local state.
  const reservedRef = useRef(false);
  // Set to `true` right BEFORE the fetch for `createPlaceholderKey`
  // leaves the client. Reset to `false` inside
  // `createPlaceholderKey` ONLY when the server returned a clean
  // 4xx (proves no side effect); any other failure mode keeps it
  // set so a cancel can't rewind atop a possibly-committed
  // placeholder.
  const placeholderCreateSentRef = useRef(false);
  // Tracks the in-flight `createPlaceholderKey` promise so
  // `releaseServerState` can wait for it to settle before reading
  // `placeholderCreateSentRef`. Without this, a user who hits
  // Cancel mid-request observes the ref still `true`, returns
  // `uncertain`, and a later 4xx response that would have reset
  // the ref arrives AFTER the rewind decision — leaving the
  // pairing latched as `action_started=true` until TTL even
  // though no placeholder was created. Cleared when the promise
  // settles (success or failure).
  const placeholderCreateInFlightRef = useRef<Promise<unknown> | null>(null);
  // Flipped `true` right before `onSuccess()` fires so the unmount
  // / beforeunload cleanup skips deleting an active credential and
  // doesn't rewind a completed reservation. Any path that
  // transitions the flow to "done" must set this first.
  const successRef = useRef(false);

  // Release server-side state on:
  //   - full page unload (tab close / refresh / cross-origin nav)
  //     via `beforeunload` — fetch with `keepalive: true`.
  //   - SPA navigation / component unmount (router Back, in-app
  //     link clicks, a parent re-render swapping flows) via the
  //     effect cleanup. This case is NOT covered by
  //     `beforeunload`; without it, leaving `/cli/pair` mid-OAuth
  //     leaves a `pending_auth` placeholder plus a latched
  //     `action_started` reservation until TTL, and the next
  //     claim for the same code hits the "already started" warning
  //     with nothing server-side to recover from.
  //
  // Both paths are skipped when `successRef.current` is true so a
  // successful completion isn't followed by a spurious delete of
  // the now-active credential.
  useEffect(() => {
    function handleBeforeUnload() {
      if (successRef.current) return;
      const stale = keyIdRef.current;
      if (stale) {
        // Placeholder has already resolved (keyId is populated).
        // Fire BOTH a conditional DELETE and a reconstructed
        // /complete as keepalive. The server's active-check on
        // /complete makes them mutually exclusive in effect:
        // one succeeds and the other is a no-op, based on
        // whether the placeholder is still pending or has
        // flipped to active.
        abandonPlaceholderKeyOnUnload(stale);
        tryCompleteAiKeyOnUnload(pairingId, stale, slug, label);
        return;
      }
      // In-flight create: `placeholderCreateSentRef` was set
      // before `POST /keys` left the client but the response
      // hasn't returned yet. We have no key id to DELETE, so
      // cancel the pairing instead — the CLI exits promptly and
      // any placeholder that commits post-unload is an orphan
      // visible on the AI Services page. Without this the
      // pairing would latch `action_started_at` until TTL even
      // though the user is clearly gone.
      if (placeholderCreateSentRef.current) {
        cancelPairingOnUnload(pairingId);
      }
    }
    window.addEventListener("beforeunload", handleBeforeUnload);
    return () => {
      window.removeEventListener("beforeunload", handleBeforeUnload);
      if (successRef.current) return;
      // SPA navigation cleanup. Pass refs (not snapshots) so the
      // helper can await any in-flight `createPlaceholderKey`
      // before reading keyId / create-sent — a late 4xx or a
      // late success each need the post-await values to
      // correctly decide between rewinding, revoking, or
      // completing.
      void releaseServerStateOnUnmount(
        pairingId,
        keyIdRef,
        reservedRef,
        placeholderCreateSentRef,
        placeholderCreateInFlightRef,
      );
    };
  }, [pairingId, slug, label]);

  /**
   * Release whatever server-side state this sub-flow may have
   * latched: the placeholder UserService (delete), the pairing's
   * `action_started` reservation (rewind), or — if the provider
   * callback already flipped the placeholder to active — promote
   * the flow to success.
   *
   * Used by both `cancelAndCleanup` (explicit user abort) and the
   * catch block in the main effect (setup failed before / after
   * placeholder creation). Without rewinding from the error path,
   * a reload from the error screen leaves the pairing latched
   * with `action_started=true` and nothing server-side to clean
   * up, so the next claim is blocked until TTL even though no
   * resource was created.
   *
   * Returns `"active"` when the flow was promoted to success
   * (caller must NOT fire `onCancel`), `"released"` when the
   * reservation was rewound, or `"uncertain"` when we couldn't
   * prove the server committed nothing (rewind skipped so a
   * replay can't mint a duplicate placeholder).
   */
  async function releaseServerState(): Promise<
    "active" | "released" | "uncertain"
  > {
    // Wait for any in-flight `createPlaceholderKey` to settle
    // FIRST — a clean 4xx will reset `placeholderCreateSentRef`
    // to `false`, which is the signal that rewind is safe. Doing
    // the check before this await would treat "fetch is in
    // flight" as "placeholder might exist" and skip rewind for a
    // request that will actually fail with no side effect.
    const inFlight = placeholderCreateInFlightRef.current;
    if (inFlight) {
      try {
        await inFlight;
      } catch {
        // Errors are fine — `createPlaceholderKey`'s own catch
        // already reset `placeholderCreateSentRef` on 4xx; other
        // failures leave it set and we'll fall through to
        // `uncertain`.
      }
    }
    const stale = keyIdRef.current;
    keyIdRef.current = null;
    const outcome = await abandonPlaceholderKey(stale);
    // Provider-callback race winner: the placeholder flipped to
    // `active` between us deciding to bail and our DELETE hitting
    // the server. Tearing the state machine down via `onCancel()`
    // would strand `/cli-pairings/{id}/complete`, so the CLI
    // would time out even though the service exists and retries
    // on the same pairing would 409 on the reserve-action latch
    // until TTL expiry. Finish the flow instead — the credential
    // the user just authorized is exactly what they asked for.
    if (outcome.kind === "active") {
      reservedRef.current = false;
      successRef.current = true;
      setPhase("done");
      onSuccess({
        kind: "ai-key",
        service_id: outcome.key.id,
        slug: outcome.key.slug,
        label: outcome.key.label,
      });
      return "active";
    }
    // Rewind when BOTH:
    //   1. this tab holds the reservation (`reservedRef`), AND
    //   2. we can prove no placeholder was committed server-side.
    //
    // "Proof of no placeholder" means either:
    //   (a) we deleted it successfully (`outcome.kind === "deleted"`),
    //       OR
    //   (b) the `createPlaceholderKey` fetch never left the
    //       client (`!placeholderCreateSentRef`). If the fetch
    //       IS in flight, the server may still be committing a
    //       placeholder we'll never see — rewinding there would
    //       reopen the pairing and let a second claim mint a
    //       duplicate placeholder. Better to leave the latch and
    //       let TTL expire the orphan.
    const noCreateSent = !placeholderCreateSentRef.current;
    const deleted = outcome.kind === "deleted";
    if (reservedRef.current && (deleted || noCreateSent)) {
      reservedRef.current = false;
      await rewindPairingAction(pairingId);
      return "released";
    }
    reservedRef.current = false;
    return "uncertain";
  }

  // Wrapper that runs the placeholder cleanup before surfacing cancel
  // to the parent. Without this, a user who aborts mid-OAuth leaves
  // a permanent pending_auth UserService in their AI Services list
  // (UserService / UserApiKey have no TTL). Matches the local wizard's
  // `pendingPlaceholderKeyId` cleanup path.
  async function cancelAndCleanup() {
    cancelledRef.current = true;
    const result = await releaseServerState();
    // `releaseServerState` already fired `onSuccess` when the
    // placeholder was found active — don't also tear the state
    // machine down.
    if (result === "active") return;
    onCancel();
  }

  // Credential-mode pre-check. Skipped entirely when the provider
  // uses `system` credentials.
  useEffect(() => {
    if (!needsUserOAuthCredentials(credentialMode)) return;
    let cancel = false;
    void (async () => {
      try {
        const metadata = await api.get<UserCredentialsMetadata>(
          `/providers/${encodeURIComponent(providerId)}/credentials`,
        );
        if (cancel) return;
        if (metadata.has_credentials) {
          setPhase("starting");
        } else {
          setPhase("needs-credentials");
        }
      } catch (e) {
        if (cancel) return;
        setPhase("error");
        setError(extractMessage(e));
      }
    })();
    return () => {
      cancel = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function saveUserCredentials() {
    if (!clientId.trim() || !clientSecret.trim()) return;
    setPhase("saving-credentials");
    setError(null);
    try {
      await api.put(
        `/providers/${encodeURIComponent(providerId)}/credentials`,
        {
          client_id: clientId.trim(),
          client_secret: clientSecret.trim(),
          label,
        },
      );
      setPhase("starting");
    } catch (e) {
      setPhase("needs-credentials");
      setError(extractMessage(e));
    }
  }

  // Placeholder + redirect + poll runs as soon as `phase === "starting"`.
  useEffect(() => {
    if (phase !== "starting") return;
    let cancel = false;
    cancelledRef.current = false;
    void (async () => {
      try {
        // Mark the destructive step as started BEFORE the placeholder
        // exists. Idempotent server-side, so re-runs on retry are a
        // no-op after the first call.
        if (!keyIdRef.current) {
          await reservePairingAction(pairingId);
          reservedRef.current = true;
        }
        // Reuse placeholder if we already minted one on a prior
        // retry (e.g. popup was blocked); avoids creating a second
        // pending_auth UserService on each mount. `slug` / `label`
        // on the reused case are carried in from the closure — we
        // never need to read them from the placeholder object in
        // the reuse branch because only `id` and `status` are used
        // below.
        let placeholder: { id: string; status: string };
        if (keyIdRef.current) {
          placeholder = { id: keyIdRef.current, status: "pending_auth" };
        } else {
          // Flag BEFORE sending the request, not after: once the
          // fetch has left the client the server may commit a
          // placeholder regardless of whether we ever see the
          // response. `cancelAndCleanup` reads this ref to decide
          // whether rewinding is safe.
          placeholderCreateSentRef.current = true;
          // Park the in-flight promise on a ref so
          // `releaseServerState` (called from Cancel or the error
          // catch) can await the request to settle before deciding
          // rewind safety. Without this, a cancel mid-flight would
          // read the still-`true` sent-ref and return `uncertain`,
          // even when the server later replies 4xx (no side effect).
          const createPromise = createPlaceholderKey(
            slug,
            label,
            placeholderCreateSentRef,
            nodeId,
            endpointUrl,
            targetOrgId,
          );
          placeholderCreateInFlightRef.current = createPromise;
          try {
            placeholder = await createPromise;
          } finally {
            // Clear only if still pointing at the same promise;
            // a later retry will install its own.
            if (placeholderCreateInFlightRef.current === createPromise) {
              placeholderCreateInFlightRef.current = null;
            }
          }
          // Record the placeholder id BEFORE the cancel check so
          // `releaseServerStateOnUnmount` can see it even when
          // the user cancelled / navigated away mid-create. If
          // we set this only on the success path, a late-
          // resolving create leaves the server-side
          // `pending_auth` placeholder orphaned: cleanup awaits
          // the promise (good), but then looks up `keyIdRef` to
          // revoke it (empty — bad).
          keyIdRef.current = placeholder.id;
        }
        if (cancel) return;

        // If a prior session already authorized the same provider and
        // the backend short-circuited to `active`, skip the popup.
        if (placeholder.status === "active") {
          await completeWithKey(placeholder.id);
          return;
        }

        const initiate = await api.get<InitiateOAuthResponse>(
          `/providers/${encodeURIComponent(providerId)}/connect/oauth?redirect_path=${encodeURIComponent(
            `/keys/${placeholder.id}`,
          )}`,
        );
        if (cancel) return;
        if (!initiate.authorization_url) {
          throw new Error("provider did not return an authorization_url");
        }
        setAuthUrl(initiate.authorization_url);
        // New tab so the pair page stays alive to poll. Popups are
        // blocked if not a user-initiated action; this effect runs on
        // mount after the user clicked "Create service", which is
        // enough gesture-context in every browser we support.
        const w = window.open(initiate.authorization_url, "_blank", "noopener,noreferrer");
        if (!w) {
          setPhase("waiting");
          setError(
            "Browser blocked the popup. Use the button below to open the provider sign-in.",
          );
          await pollUntilActive(placeholder.id);
          return;
        }
        setPhase("waiting");
        await pollUntilActive(placeholder.id);
      } catch (e) {
        if (cancel) return;
        setPhase("error");
        setError(extractMessage(e));
        // Free server-side state so a reload/close from the error
        // screen doesn't leave the pairing latched with
        // `action_started=true` and nothing to clean up. On a 4xx
        // from `createPlaceholderKey`, the ref was reset so the
        // reservation can safely rewind; later-stage failures (5xx,
        // missing authorization_url) fall through to `uncertain`
        // and the TTL reclaims the latch.
        void releaseServerState();
      }
    })();
    return () => {
      cancel = true;
      cancelledRef.current = true;
    };
    // Runs when `phase` transitions to "starting" — i.e. either on
    // initial mount for `system` providers or after the user OAuth
    // credentials sub-step finishes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [phase]);

  async function pollUntilActive(keyId: string) {
    await pollOAuthKeyUntilActive({
      keyId,
      getKey: (id) =>
        api.get<ActiveKeyResponse>(`/keys/${encodeURIComponent(id)}`),
      completeWithKey,
      isCancelled: () => cancelledRef.current,
      onTerminalFailure: () => {
        setPhase("error");
        setError(
          "Authorization didn't complete (it may have been canceled or denied on the provider page). Cancel and re-run to try again.",
        );
      },
      onTimeout: () => {
        setPhase("error");
        setError(
          "We didn't see authorization complete within 5 minutes. If you canceled on the provider page or it's taking longer than expected, cancel and re-run.",
        );
      },
    });
  }

  async function completeWithKey(keyId: string) {
    const finalKey = await api.get<ActiveKeyResponse>(
      `/keys/${encodeURIComponent(keyId)}`,
    );
    if (cancelledRef.current) return;
    successRef.current = true;
    setPhase("done");
    onSuccess({
      kind: "ai-key",
      service_id: finalKey.id,
      slug: finalKey.slug,
      label: finalKey.label,
    });
  }

  // User-OAuth-app credentials sub-step — mirrors the local wizard's
  // Step 2a / `OAuthCredentialsStep` in the frontend.
  if (phase === "needs-credentials" || phase === "saving-credentials") {
    const saving = phase === "saving-credentials";
    return (
      <div className="flex flex-col gap-4">
        <div className="flex flex-col gap-1">
          <h3 className="font-medium">Paste your OAuth app credentials</h3>
          <p className="text-[12px] text-muted-foreground">
            This provider expects you to register your own OAuth app
            (Developer Settings → OAuth Apps) and paste the resulting
            Client ID and Client Secret below.
          </p>
          {documentationUrl ? (
            <a
              href={documentationUrl}
              target="_blank"
              rel="noopener noreferrer"
              className="inline-flex items-center gap-1 text-xs text-muted-foreground underline-offset-2 hover:underline"
            >
              How to create an OAuth app
              <ExternalLink className="h-3 w-3" />
            </a>
          ) : null}
        </div>

        <div className="flex flex-col gap-3">
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="pair-aikey-oauth-client-id">Client ID</Label>
            <Input
              id="pair-aikey-oauth-client-id"
              value={clientId}
              onChange={(e) => {
                setClientId(e.target.value);
              }}
              autoFocus
              autoComplete="off"
            />
          </div>
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="pair-aikey-oauth-client-secret">
              Client Secret
            </Label>
            <Input
              id="pair-aikey-oauth-client-secret"
              type="password"
              value={clientSecret}
              onChange={(e) => {
                setClientSecret(e.target.value);
              }}
              autoComplete="off"
            />
          </div>
        </div>

        {error ? <ErrorLine message={error} /> : null}

        <Button
          variant="primary"
          onClick={() => void saveUserCredentials()}
          disabled={saving || !clientId.trim() || !clientSecret.trim()}
        >
          {saving ? "Saving..." : "Save and continue"}
        </Button>
        <Button
          variant="outline"
          onClick={() => void cancelAndCleanup()}
          disabled={saving}
        >
          Cancel
        </Button>
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-1">
        <h3 className="font-medium">Complete sign-in on the provider</h3>
        <p className="text-[12px] text-muted-foreground">
          We opened a new tab where you'll authorize NyxID. When it
          completes, come back — this page will finish automatically.
        </p>
      </div>
      {phase === "checking-credentials" ? (
        <div className="flex items-center gap-2 text-[12px] text-muted-foreground">
          <Loader2 className="h-4 w-4 animate-spin" />
          Checking provider credentials...
        </div>
      ) : phase === "starting" ? (
        <div className="flex items-center gap-2 text-[12px] text-muted-foreground">
          <Loader2 className="h-4 w-4 animate-spin" />
          Creating placeholder service...
        </div>
      ) : phase === "waiting" ? (
        <div className="flex items-center gap-2 text-[12px] text-muted-foreground">
          <Loader2 className="h-4 w-4 animate-spin" />
          Waiting for provider authorization...
        </div>
      ) : null}

      {authUrl && phase === "waiting" ? (
        <a
          href={authUrl}
          target="_blank"
          rel="noopener noreferrer"
          className="inline-flex items-center justify-center gap-2 rounded-lg border bg-muted/40 px-3 py-2 text-[12px] hover:bg-muted"
        >
          Reopen provider sign-in
          <ExternalLink className="h-4 w-4" />
        </a>
      ) : null}

      {error ? <ErrorLine message={error} /> : null}

      {phase !== "done" ? (
        <Button variant="outline" onClick={() => void cancelAndCleanup()}>
          Cancel
        </Button>
      ) : null}
    </div>
  );
}

// ── device code ──────────────────────────────────────────────────────

export function DeviceCodeFlow({
  providerId,
  slug,
  label,
  nodeId,
  targetOrgId,
  endpointUrl,
  pairingId,
  onSuccess,
  onCancel,
}: FlowProps) {
  const [code, setCode] = useState<string | null>(null);
  const [verifyUrl, setVerifyUrl] = useState<string | null>(null);
  const [phase, setPhase] = useState<
    "starting" | "waiting" | "expired" | "done" | "error"
  >("starting");
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  // Bumped on refresh to kill in-flight poll loops.
  const genRef = useRef(0);
  const keyIdRef = useRef<string | null>(null);
  const cancelledRef = useRef(false);
  // Set to `true` once THIS tab has successfully latched
  // `reservePairingAction` on the server. Used to gate the rewind
  // call in `cancelAndCleanup` so a losing tab (409 on reserve)
  // can't clear a winning tab's latch and enable a replay. If
  // this tab never reserved, cancel just tears down local state.
  const reservedRef = useRef(false);
  // Set to `true` right BEFORE the fetch for `createPlaceholderKey`
  // leaves the client. `createPlaceholderKey` resets it to `false`
  // only when the server returned a clean 4xx (proves no side
  // effect); any other failure keeps it set.
  const placeholderCreateSentRef = useRef(false);
  // Mirror of `OAuthFlow.placeholderCreateInFlightRef` — lets
  // `releaseServerState` await the create fetch to settle so a
  // 4xx that clears `placeholderCreateSentRef` is observed BEFORE
  // the rewind decision. Without this a Cancel during the request
  // would always return `uncertain` and strand the pairing.
  const placeholderCreateInFlightRef = useRef<Promise<unknown> | null>(null);
  // Mirror of `OAuthFlow.successRef`: gates the unmount /
  // beforeunload cleanup so a completed flow isn't followed by
  // a spurious DELETE on the now-active credential.
  const successRef = useRef(false);

  /**
   * Mirror of `OAuthFlow.releaseServerState`: delete any committed
   * placeholder (or promote to success if the credential already
   * flipped to active) and rewind the reservation when we can
   * prove no placeholder committed. Used by both the explicit
   * Cancel path AND the error catch so a reload from the error
   * screen doesn't leave the pairing latched.
   */
  async function releaseServerState(): Promise<
    "active" | "released" | "uncertain"
  > {
    // Wait for the in-flight `createPlaceholderKey` (if any) to
    // settle so a clean 4xx has a chance to reset
    // `placeholderCreateSentRef` before we read it. See the
    // matching comment in `OAuthFlow.releaseServerState`.
    const inFlight = placeholderCreateInFlightRef.current;
    if (inFlight) {
      try {
        await inFlight;
      } catch {
        // 4xx already reset the ref; other errors leave it set
        // and we fall through to `uncertain`.
      }
    }
    const stale = keyIdRef.current;
    keyIdRef.current = null;
    const outcome = await abandonPlaceholderKey(stale);
    if (outcome.kind === "active") {
      reservedRef.current = false;
      successRef.current = true;
      setPhase("done");
      onSuccess({
        kind: "ai-key",
        service_id: outcome.key.id,
        slug: outcome.key.slug,
        label: outcome.key.label,
      });
      return "active";
    }
    const noCreateSent = !placeholderCreateSentRef.current;
    const deleted = outcome.kind === "deleted";
    if (reservedRef.current && (deleted || noCreateSent)) {
      reservedRef.current = false;
      await rewindPairingAction(pairingId);
      return "released";
    }
    reservedRef.current = false;
    return "uncertain";
  }

  // Same cleanup wrapper as in OAuthFlow — ensures an abandoned
  // device-code session doesn't leave a permanent pending_auth key
  // in the user's AI Services list.
  async function cancelAndCleanup() {
    cancelledRef.current = true;
    genRef.current += 1;
    const result = await releaseServerState();
    if (result === "active") return;
    onCancel();
  }

  // Mirror of the OAuthFlow cleanup hook: covers both full-page
  // unload (beforeunload) AND SPA navigation / component unmount
  // (effect cleanup). Without the unmount arm, leaving
  // `/cli/pair` via router back or an in-app link leaves a
  // `pending_auth` placeholder and a latched `action_started`
  // reservation that only TTL reclaims. `successRef` gates both
  // paths so a completed flow doesn't revoke its own credential.
  useEffect(() => {
    function handleBeforeUnload() {
      if (successRef.current) return;
      const stale = keyIdRef.current;
      if (stale) {
        // Mirror of OAuthFlow: fire both conditional DELETE and
        // reconstructed /complete. Server's active-check
        // arbitrates — the appropriate one succeeds and the
        // other is a no-op.
        abandonPlaceholderKeyOnUnload(stale);
        tryCompleteAiKeyOnUnload(pairingId, stale, slug, label);
        return;
      }
      // In-flight create case: no key id to target yet, so
      // cancel the pairing so the CLI exits without waiting for
      // TTL. See the matching comment in `OAuthFlow`.
      if (placeholderCreateSentRef.current) {
        cancelPairingOnUnload(pairingId);
      }
    }
    window.addEventListener("beforeunload", handleBeforeUnload);
    return () => {
      window.removeEventListener("beforeunload", handleBeforeUnload);
      if (successRef.current) return;
      // Pass refs so the helper can await any in-flight
      // `createPlaceholderKey` and re-read them post-settle.
      // See `releaseServerStateOnUnmount` for the full decision
      // matrix (rewind / revoke / complete with ai-key ack when
      // the placeholder flipped to active).
      void releaseServerStateOnUnmount(
        pairingId,
        keyIdRef,
        reservedRef,
        placeholderCreateSentRef,
        placeholderCreateInFlightRef,
      );
    };
  }, [pairingId, slug, label]);

  useEffect(() => {
    cancelledRef.current = false;
    void startSession();
    return () => {
      cancelledRef.current = true;
      genRef.current += 1;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function startSession() {
    const myGen = ++genRef.current;
    setPhase("starting");
    setError(null);
    try {
      let keyId = keyIdRef.current;
      if (!keyId) {
        // See `OAuthFlow` for rationale: reserve the destructive
        // action server-side before creating the placeholder.
        // `reservedRef` is what `cancelAndCleanup` checks before
        // rewinding — without flipping it here, a losing tab's
        // cancel would skip rewind (good) but the WINNING tab
        // would also skip rewind on its own cancel (bad), leaving
        // the pairing latched forever and forcing a CLI rerun.
        await reservePairingAction(pairingId);
        reservedRef.current = true;
        // Flag BEFORE the fetch leaves: see the matching comment
        // in OAuthFlow. The server may commit a placeholder we
        // never observe if the user cancels mid-request.
        placeholderCreateSentRef.current = true;
        // Park the in-flight promise so `releaseServerState` can
        // wait for it to settle before the rewind decision; see
        // the matching comment in `OAuthFlow`.
        const createPromise = createPlaceholderKey(
          slug,
          label,
          placeholderCreateSentRef,
          nodeId,
          endpointUrl,
          targetOrgId,
        );
        placeholderCreateInFlightRef.current = createPromise;
        let placeholder: PlaceholderKeyResponse;
        try {
          placeholder = await createPromise;
        } finally {
          if (placeholderCreateInFlightRef.current === createPromise) {
            placeholderCreateInFlightRef.current = null;
          }
        }
        // Record the placeholder id BEFORE the generation-guard
        // check (see the matching comment in `OAuthFlow`). A
        // cancelled-mid-create flow that later resolves would
        // otherwise leave the `pending_auth` placeholder
        // orphaned because `releaseServerStateOnUnmount` looks
        // up `keyIdRef` to revoke / complete it.
        keyId = placeholder.id;
        keyIdRef.current = keyId;
        if (myGen !== genRef.current) return;

        // Fast-path: the user already connected this provider
        // on a prior session. `POST /keys` flips the placeholder
        // straight to `active` in that case, so there's nothing
        // to authorize here — skip the device-code prompt and
        // complete the flow. Mirrors `OAuthFlow`'s same-provider
        // short-circuit; without it the user sees a redundant
        // "enter code XXXX-XXXX at google.com/device" prompt
        // and the pairing times out if they ignore it.
        if (placeholder.status === "active") {
          const active = await api.get<ActiveKeyResponse>(
            `/keys/${encodeURIComponent(keyId)}`,
          );
          if (myGen !== genRef.current) return;
          successRef.current = true;
          setPhase("done");
          onSuccess({
            kind: "ai-key",
            service_id: active.id,
            slug: active.slug,
            label: active.label,
          });
          return;
        }
      }

      const init = await api.post<DeviceCodeInitiateResponse>(
        `/providers/${encodeURIComponent(providerId)}/connect/device-code/initiate`,
        {},
      );
      if (myGen !== genRef.current) return;

      setCode(init.user_code);
      setVerifyUrl(init.verification_uri);
      setPhase("waiting");

      let interval = Number(init.interval) || 5;
      const pollPath = `/providers/${encodeURIComponent(providerId)}/connect/device-code/poll`;
      const deadline = Date.now() + 10 * 60 * 1000;
      while (Date.now() < deadline) {
        if (myGen !== genRef.current || cancelledRef.current) return;
        await sleep(interval * 1000);
        if (myGen !== genRef.current || cancelledRef.current) return;
        let res: DeviceCodePollResponse;
        try {
          res = await api.post<DeviceCodePollResponse>(pollPath, {
            state: init.state,
          });
        } catch {
          continue;
        }
        if (myGen !== genRef.current) return;
        const status = res.status ?? "";
        if (
          status === "complete" ||
          status === "authorized" ||
          res.access_token
        ) {
          const final = await api.get<ActiveKeyResponse>(
            `/keys/${encodeURIComponent(keyId)}`,
          );
          if (myGen !== genRef.current) return;
          successRef.current = true;
          setPhase("done");
          onSuccess({
            kind: "ai-key",
            service_id: final.id,
            slug: final.slug,
            label: final.label,
          });
          return;
        }
        if (status === "expired") {
          setPhase("expired");
          return;
        }
        if (status === "denied") {
          setPhase("error");
          setError("Authorization denied on the provider side.");
          return;
        }
        if (status === "slow_down") {
          interval = Number(res.interval) || interval + 5;
        }
      }
      if (myGen === genRef.current) {
        setPhase("expired");
      }
    } catch (e) {
      if (myGen === genRef.current) {
        setPhase("error");
        setError(extractMessage(e));
        // Free server-side state so a reload/close from the error
        // screen doesn't leave the pairing latched with
        // `action_started=true` and nothing to clean up. See the
        // matching comment in `OAuthFlow`.
        void releaseServerState();
      }
    }
  }

  async function handleCopy() {
    if (!code) return;
    try {
      await navigator.clipboard.writeText(code);
      setCopied(true);
      window.setTimeout(() => {
        setCopied(false);
      }, 2000);
    } catch {
      // Clipboard API may be blocked in non-secure contexts.
    }
  }

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-1">
        <h3 className="font-medium">Authorize via device code</h3>
        <p className="text-[12px] text-muted-foreground">
          Open the verification URL, enter the code, and complete
          sign-in on the provider. This page will finish automatically.
        </p>
      </div>

      {phase === "starting" ? (
        <div className="flex items-center gap-2 text-[12px] text-muted-foreground">
          <Loader2 className="h-4 w-4 animate-spin" />
          Requesting device code...
        </div>
      ) : phase === "waiting" && code && verifyUrl ? (
        <div className="flex flex-col gap-3 rounded-lg border bg-muted/30 p-4">
          <div className="flex flex-col gap-1">
            <span className="text-xs uppercase tracking-wide text-muted-foreground">
              Code
            </span>
            <div className="flex items-center gap-2">
              <code className="rounded bg-background px-3 py-1.5 font-mono text-lg">
                {code}
              </code>
              <Button
                variant="outline"
                onClick={() => void handleCopy()}
              >
                <ButtonIcon><Copy className="h-3.5 w-3.5" /></ButtonIcon>
                {copied ? "Copied" : "Copy"}
              </Button>
            </div>
          </div>
          <div className="flex flex-col gap-1">
            <span className="text-xs uppercase tracking-wide text-muted-foreground">
              Visit
            </span>
            <a
              href={verifyUrl}
              target="_blank"
              rel="noopener noreferrer"
              className="inline-flex items-center gap-1.5 text-[12px] underline-offset-2 hover:underline"
            >
              {verifyUrl}
              <ExternalLink className="h-3.5 w-3.5" />
            </a>
          </div>
          <div className="flex items-center gap-2 text-xs text-muted-foreground">
            <Loader2 className="h-3 w-3 animate-spin" />
            Waiting for authorization...
          </div>
        </div>
      ) : phase === "expired" ? (
        <div className="flex flex-col gap-2">
          <p className="rounded-lg border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-[12px]">
            The device code expired before authorization completed.
          </p>
          <Button variant="primary" onClick={() => void startSession()}>
            Request a new code
          </Button>
        </div>
      ) : null}

      {error ? <ErrorLine message={error} /> : null}

      {phase !== "done" ? (
        <Button variant="outline" onClick={() => void cancelAndCleanup()}>
          Cancel
        </Button>
      ) : null}
    </div>
  );
}

// ── helpers ──────────────────────────────────────────────────────────

function ErrorLine({ message }: { readonly message: string }) {
  return (
    <p className="rounded-lg border border-destructive/40 bg-destructive/10 px-3 py-2 text-[12px] text-destructive">
      {message}
    </p>
  );
}

function extractMessage(e: unknown): string {
  if (e instanceof ApiError) return e.message;
  if (e instanceof Error) return e.message;
  return "Something went wrong. Please try again.";
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => {
    window.setTimeout(resolve, ms);
  });
}
