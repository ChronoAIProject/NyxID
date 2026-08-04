// CLI pairing page. Mode B wizard endpoint (see docs / ADR when written).
//
// Flow:
//   Step 1 — user enters the pairing code the CLI printed
//   Step 2 — page claims the pairing, renders a kind-specific confirm panel
//   Step 3 — user executes the action; backend returns a one-time secret
//   Step 4 — DisplayOnce panel; "I saved it" POSTs the typed ack back
//            to /cli-pairings/{id}/complete so the CLI's next poll ends
//
// Secrets never leave the browser: the CLI receives only a non-secret
// identifier (api_key_id / token_id / resource_id) in the ack. Matches
// the local-server DisplayOnce wizard byte-for-byte in spirit.

import { useEffect, useState } from "react";
import { ApiError, api } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { WizardShell } from "@/components/cli-wizard/shell";
import { DisconnectBanner } from "@/components/cli-wizard/disconnect-banner";
import {
  ENTER_CODE_STEP,
  resolveStep,
  type WizardFlow,
} from "@/components/cli-wizard/step-label";
import type {
  AckPayload,
  AiKeyPrefill,
  ApiKeyCreatePrefill,
  ClaimResponse,
  DeveloperAppCreatePrefill,
  NodeRegisterPrefill,
  PairingKind,
  RotatePrefill,
  ServiceAccountCreatePrefill,
} from "./types";
import { isPairingKind } from "./types";
import { DisplayOncePanel, RecoveryCodesPanel } from "./display-once";
import {
  ApiKeyCreateConfirm,
  ApiKeyRotateConfirm,
  DeveloperAppCreateConfirm,
  DeveloperAppRotateSecretConfirm,
  MfaSetupConfirm,
  NodeRegisterConfirm,
  NodeRotateConfirm,
  ServiceAccountCreateConfirm,
  ServiceAccountRotateSecretConfirm,
  type ApiKeyCreateSuccess,
  type ApiKeyRotateSuccess,
  type DeveloperAppCreateSuccess,
  type DeveloperAppRotateSecretSuccess,
  type MfaSetupSuccess,
  type NodeRegisterSuccess,
  type NodeRotateSuccess,
  type ServiceAccountCreateSuccess,
  type ServiceAccountRotateSecretSuccess,
} from "@/components/cli-wizard/confirm-panels";
import {
  AiKeyConfirm,
  type AiKeyPairingSuccess,
} from "@/components/cli-wizard/ai-key-confirm-panel";
import { parseAiKeyPrefill } from "@/schemas/cli-wizard";

type ActionResult =
  | AiKeyPairingSuccess
  | ApiKeyCreateSuccess
  | ApiKeyRotateSuccess
  | NodeRegisterSuccess
  | NodeRotateSuccess
  | ServiceAccountCreateSuccess
  | ServiceAccountRotateSecretSuccess
  | DeveloperAppCreateSuccess
  | DeveloperAppRotateSecretSuccess
  | MfaSetupSuccess;

type Stage =
  | { readonly phase: "enter-code" }
  | { readonly phase: "claimed"; readonly claim: ClaimResponse }
  | {
      /**
       * DisplayOnce flows only: the destructive API already returned
       * a one-time secret, but we POST `/complete` BEFORE rendering
       * it. If the tab closes or the browser crashes while still in
       * this phase, the secret is lost but the CLI was never given
       * a false "completed" signal — it times out correctly. Without
       * this interstitial, a tab-close between mint and ack would
       * leave the pairing locked as `action_started=true` forever,
       * stranding the minted secret and forcing the user to re-run.
       *
       * `completeError` is `null` while the POST is in flight and
       * set to the failure message once a /complete attempt fails
       * — `NotifyingCliPanel` then swaps the spinner for a retry
       * button. The panel itself does NOT initiate the POST; the
       * parent handler does, in the same turn as the destructive
       * API success, to avoid a render-gap race where a reload
       * strands the pairing.
       */
      readonly phase: "notifying-cli";
      readonly claim: ClaimResponse;
      readonly result: ActionResult;
      readonly completeError: string | null;
    }
  | {
      readonly phase: "secret";
      readonly claim: ClaimResponse;
      readonly result: ActionResult;
    }
  | {
      /**
       * ai-key flow only: the user's credential was supplied as input,
       * so there's nothing to DisplayOnce. We still fire `/complete`
       * from the parent (same rationale as `notifying-cli`) and show
       * a lightweight confirmation. `completeError` mirrors the
       * DisplayOnce path for retry UX.
       */
      readonly phase: "acking";
      readonly claim: ClaimResponse;
      readonly result: AiKeyPairingSuccess;
      readonly completeError: string | null;
    }
  | {
      /**
       * Disambiguation screen shown when a rotation claim comes
       * back as `resumed && action_started`. We can't tell from
       * server state alone whether the prior tab's rotation
       * actually succeeded or failed with `action_started_at`
       * left latched (5xx / network error path); blindly
       * sending the reconstructed ack would give the CLI a
       * false-success summary for case (b). This phase shows
       * the user a choice — "the other tab showed me a secret,
       * notify the CLI" vs "I don't have a secret, cancel this
       * pairing" — and only advances to `resending-ack` when
       * the user affirmatively confirms success.
       */
      readonly phase: "resumed-rotation-choice";
      readonly claim: ClaimResponse;
      readonly ack: AckPayload;
    }
  | {
      /**
       * Terminal-like warning for non-rotation `resumed &&
       * action_started` claims (api-key-create,
       * node-register-token, ai-key). We can't reconstruct the
       * ack (new ids aren't in prefill), so there is no
       * recovery. On entering this phase we fire
       * `/cli-pairings/{id}/cancel` fire-and-forget so the
       * waiting CLI exits promptly with Cancelled instead of
       * polling until TTL. The user sees guidance to check
       * their Keys / API Keys page — the resource may already
       * exist from the first tab even though this tab can't
       * finish the pairing.
       */
      readonly phase: "resumed-create-warning";
      readonly claim: ClaimResponse;
    }
  | {
      /**
       * Recovery path for `resumed && action_started` ROTATION
       * claims where the user has confirmed (via
       * `resumed-rotation-choice`) that the first tab actually
       * showed them a new secret. Fires the reconstructed ack
       * so the CLI unblocks.
       */
      readonly phase: "resending-ack";
      readonly claim: ClaimResponse;
      readonly ack: AckPayload;
      readonly completeError: string | null;
    }
  | {
      /**
       * Terminal success/close screen. Carries the flow it completed so
       * the shared step resolver can render the flow's final
       * "Step N of N · done" step. Without this the done stage had no
       * flow and the resolver fell back to the pre-flow "enter code"
       * label, contradicting the "Pairing complete" panel (NyxID#734).
       */
      readonly phase: "done";
      readonly flow: WizardFlow;
    };

/** Every stage except the pre-flow `enter-code` — see `stageFlow`. */
type PostClaimStage = Exclude<Stage, { readonly phase: "enter-code" }>;

export function CliPairPage() {
  const { isAuthenticated, isLoading } = useAuthStore();

  useEffect(() => {
    if (isLoading) return;
    if (!isAuthenticated) {
      // Preserve return-to INCLUDING `window.location.search`. The
      // pair URL intentionally carries `?code=ABCD-1234` so same-
      // device handoffs arrive prefilled; dropping the search string
      // in the login redirect would bounce the user back to a blank
      // form and strand the pairing if they only had the URL (for
      // example: an AI agent relayed the URL but not the code
      // separately).
      const returnTo = `${window.location.origin}${window.location.pathname}${window.location.search}`;
      window.location.assign(
        `/login?return_to=${encodeURIComponent(returnTo)}`,
      );
    }
  }, [isAuthenticated, isLoading]);

  if (isLoading || !isAuthenticated) {
    return (
      <WizardShell context="pair">
        <Skeleton className="h-40 w-full" />
      </WizardShell>
    );
  }

  return <StageRouter />;
}

/**
 * Attempt to reconstruct a rotation ack from a `resumed &&
 * action_started` claim using only `prefill.resource_id`. Returns
 * `null` for non-rotation kinds (create flows mint fresh ids we
 * can't see from this tab).
 *
 * Used by the recovery path when a prior tab completed the
 * destructive rotate but lost the `/complete` POST. Because the
 * server `complete` handler is idempotent on same-ack retries,
 * resending this ack is safe — the CLI's next poll sees Completed
 * instead of timing out.
 */
function reconstructRotationAck(claim: ClaimResponse): AckPayload | null {
  if (
    claim.kind !== "api-key-rotate" &&
    claim.kind !== "node-rotate-token" &&
    claim.kind !== "service-account-rotate-secret" &&
    claim.kind !== "developer-app-rotate-secret"
  ) {
    return null;
  }
  const prefill = claim.prefill as { readonly resource_id?: unknown };
  const resourceId = prefill.resource_id;
  if (typeof resourceId !== "string" || resourceId.length === 0) {
    return null;
  }
  return { acknowledged: true, resource_id: resourceId };
}

function StageRouter() {
  const [stage, setStage] = useState<Stage>({ phase: "enter-code" });
  // Pairing-side liveness. Once a claim lands, poll the backend
  // every 4s for status so we notice when the CLI cancels or the
  // record TTLs out without requiring the user to click a button.
  const [pairingLost, setPairingLost] = useState<
    "cancelled" | "expired" | null
  >(null);

  const claimId =
    "claim" in stage ? stage.claim.id : null;

  useEffect(() => {
    if (!claimId) return;
    // Stop polling once we've reached a terminal local state — the
    // browser already knows the outcome and the poll just burns API
    // calls. `secret` and `acking` keep polling so a user sitting on
    // the DisplayOnce panel still sees "CLI cancelled" if it
    // happens.
    if (stage.phase === "done" || stage.phase === "resumed-create-warning") {
      return;
    }
    let stopped = false;
    async function tick() {
      if (stopped) return;
      try {
        const resp = await api.get<{ readonly status: string }>(
          `/cli-pairings/${encodeURIComponent(claimId!)}/poll`,
        );
        if (resp.status === "cancelled") {
          setPairingLost("cancelled");
        } else if (resp.status === "expired") {
          setPairingLost("expired");
        } else if (pairingLost && resp.status === "claimed") {
          // Transient network hiccup that recovered. Clear banner.
          setPairingLost(null);
        }
      } catch {
        // Don't flip "lost" on transient errors — only on server-
        // reported terminal states. Fetch failures are likely offline
        // / reload noise; the next tick catches up.
      }
    }
    const handle = window.setInterval(() => {
      void tick();
    }, 4_000);
    void tick();
    return () => {
      stopped = true;
      window.clearInterval(handle);
    };
  }, [claimId, stage.phase, pairingLost]);

  /**
   * Resend a reconstructed ack for a resumed rotation. Mirrors
   * `postComplete` but does NOT transition to `secret` — this tab
   * never held the rotated secret, so there's nothing to show. On
   * success we go straight to `done` and the CLI unblocks.
   */
  async function resendRotationAck(
    claim: ClaimResponse,
    ack: AckPayload,
  ): Promise<void> {
    try {
      await api.post(
        `/cli-pairings/${encodeURIComponent(claim.id)}/complete`,
        { ack },
      );
      setStage({ phase: "done", flow: claim.kind as WizardFlow });
    } catch (e) {
      setStage({
        phase: "resending-ack",
        claim,
        ack,
        completeError: extractErrorMessage(e),
      });
    }
  }

  // Fire `/cli-pairings/{id}/complete`. Called directly from the
  // `onActionComplete` callback in the same turn that the
  // destructive API success handler runs (NOT from a child's mount
  // `useEffect`). The mount-effect pattern introduced a render gap
  // where a browser reload between the secret being minted and the
  // effect firing would strand the pairing as
  // `claimed / action_started=true`, so the waiting CLI would time
  // out even though the key/token was already created server-side.
  // By calling this synchronously after the destructive API
  // resolves, the fetch is in flight before React even commits the
  // next render.
  async function postComplete(
    claim: ClaimResponse,
    result: ActionResult,
  ): Promise<void> {
    const ack =
      result.kind === "ai-key"
        ? {
            acknowledged: true as const,
            service_id: result.service_id,
            slug: result.slug,
            label: result.label,
          }
        : ackForResult(result);
    try {
      await api.post(
        `/cli-pairings/${encodeURIComponent(claim.id)}/complete`,
        { ack },
      );
      // Safe to transition: /complete landed, so the CLI's next
      // poll will see Completed. For DisplayOnce flows we now show
      // the secret; for ai-key we go straight to the done screen.
      if (result.kind === "ai-key") {
        setStage({ phase: "done", flow: claim.kind as WizardFlow });
      } else {
        setStage({ phase: "secret", claim, result });
      }
    } catch (e) {
      // /complete failed — secret has been minted server-side, so
      // we stay in the notifying-cli / acking phase with the error
      // so the user can retry via the button. The pairing's
      // action_started latch is already set; retries replay the
      // same ack.
      const msg = extractErrorMessage(e);
      if (result.kind === "ai-key") {
        setStage({
          phase: "acking",
          claim,
          result,
          completeError: msg,
        });
      } else {
        setStage({
          phase: "notifying-cli",
          claim,
          result,
          completeError: msg,
        });
      }
    }
  }

  // `enter-code` is the only phase without a known flow, so it renders the
  // constant pre-flow step; every other stage carries its flow (NyxID#734),
  // letting the resolver land on the right "Step N of N · …" copy.
  const step =
    stage.phase === "enter-code"
      ? ENTER_CODE_STEP
      : resolveStep(stage.phase, stageFlow(stage));

  return (
    <WizardShell context="pair" step={step}>
      {pairingLost ? (
        <DisconnectBanner
          state="disconnected"
          context="pair"
          pairingStatus={pairingLost}
        />
      ) : null}
      {renderStage()}
    </WizardShell>
  );

  function renderStage() {
    switch (stage.phase) {
    case "enter-code":
      return (
        <EnterCodeForm
          onClaim={(claim) => {
            // `resumed && action_started` means another tab
            // latched `action_started_at` but never posted
            // `/complete`. For ROTATION flows the ack is
            // reconstructible from `prefill.resource_id` — but
            // we CANNOT auto-resend it: `withRewindOnError`
            // intentionally leaves the latch set on 5xx /
            // timeout, so from this tab's view "latched without
            // /complete" is ambiguous between
            //   (a) rotation succeeded, /complete was lost, OR
            //   (b) rotation never succeeded (5xx / net error).
            // Blindly acking case (b) would tell the CLI the
            // rotation completed when no new secret was ever
            // minted — a false-success terminal summary in the
            // exact failure path where users need an accurate
            // signal. Route rotations to a choice panel so the
            // user decides based on what they saw in the other
            // tab; non-rotation kinds fall through to the plain
            // warning below (their ack can't be reconstructed
            // anyway).
            if (claim.resumed && claim.action_started) {
              const recoveryAck = reconstructRotationAck(claim);
              if (recoveryAck) {
                setStage({
                  phase: "resumed-rotation-choice",
                  claim,
                  ack: recoveryAck,
                });
                return;
              }
              // Non-rotation resumed+action_started: the ack
              // can't be reconstructed from prefill (the ids are
              // freshly minted on the other tab), so there's no
              // recovery. Route to a warning stage that fires
              // `/cancel` on mount — without that the original
              // CLI would poll until TTL, and the previous
              // warning text advised the user to re-run which
              // risks creating a duplicate resource.
              setStage({ phase: "resumed-create-warning", claim });
              return;
            }
            setStage({ phase: "claimed", claim });
          }}
        />
      );
    case "claimed":
      return (
        <ConfirmPanel
          claim={stage.claim}
          onActionComplete={(result) => {
            // Show the notifying/acking UI immediately AND fire
            // the POST in the same tick. The fetch starts before
            // React commits the re-render; a reload after the
            // destructive API but before the fetch reaches the
            // network is essentially impossible, whereas the
            // previous mount-effect pattern left an exploitable
            // gap.
            if (result.kind === "ai-key") {
              setStage({
                phase: "acking",
                claim: stage.claim,
                result,
                completeError: null,
              });
            } else {
              setStage({
                phase: "notifying-cli",
                claim: stage.claim,
                result,
                completeError: null,
              });
            }
            void postComplete(stage.claim, result);
          }}
        />
      );
    case "notifying-cli":
      return (
        <NotifyingCliPanel
          result={stage.result}
          completeError={stage.completeError}
          onRetry={() => {
            setStage({
              phase: "notifying-cli",
              claim: stage.claim,
              result: stage.result,
              completeError: null,
            });
            void postComplete(stage.claim, stage.result);
          }}
        />
      );
    case "secret":
      return (
        <SecretPanel
          result={stage.result}
          onAcknowledged={() => {
            setStage({ phase: "done", flow: stage.claim.kind as WizardFlow });
          }}
        />
      );
    case "acking":
      return (
        <AiKeyAckPanel
          result={stage.result}
          completeError={stage.completeError}
          onRetry={() => {
            setStage({
              phase: "acking",
              claim: stage.claim,
              result: stage.result,
              completeError: null,
            });
            void postComplete(stage.claim, stage.result);
          }}
        />
      );
    case "resumed-create-warning":
      return (
        <ResumedCreateWarningPanel
          claim={stage.claim}
          onAcknowledged={() => {
            setStage({ phase: "done", flow: stage.claim.kind as WizardFlow });
          }}
        />
      );
    case "resumed-rotation-choice":
      return (
        <ResumedRotationChoicePanel
          kind={stage.claim.kind}
          onConfirmSuccess={() => {
            setStage({
              phase: "resending-ack",
              claim: stage.claim,
              ack: stage.ack,
              completeError: null,
            });
            void resendRotationAck(stage.claim, stage.ack);
          }}
          onCancel={async () => {
            // Explicit "rotation didn't succeed" path: cancel the
            // pairing so the CLI exits promptly with a
            // Cancelled status instead of waiting for TTL. Best-
            // effort — if the cancel call fails the CLI still
            // times out cleanly.
            try {
              await api.post(
                `/cli-pairings/${encodeURIComponent(stage.claim.id)}/cancel`,
                {},
              );
            } catch {
              // Ignored — worst case the CLI times out.
            }
            setStage({ phase: "done", flow: stage.claim.kind as WizardFlow });
          }}
        />
      );
    case "resending-ack":
      return (
        <ResendingAckPanel
          kind={stage.claim.kind}
          completeError={stage.completeError}
          onRetry={() => {
            setStage({
              phase: "resending-ack",
              claim: stage.claim,
              ack: stage.ack,
              completeError: null,
            });
            void resendRotationAck(stage.claim, stage.ack);
          }}
        />
      );
    case "done":
      return <DonePanel />;
    }
  }
}

/**
 * The `WizardFlow` for a post-claim stage. Every non-`enter-code` stage
 * either carries the `claim` (whose `kind` is the flow) or, for the
 * terminal `done` stage, the flow captured at completion — so the flow is
 * always known here by construction. `enter-code` has no flow and is
 * handled by the caller via `ENTER_CODE_STEP`, so it never reaches this
 * function (the `PostClaimStage` parameter makes that a type error).
 */
function stageFlow(stage: PostClaimStage): WizardFlow {
  if (stage.phase === "done") {
    return stage.flow;
  }
  return stage.claim.kind as WizardFlow;
}

// ── Step 1: enter code ──────────────────────────────────────────────

function EnterCodeForm({
  onClaim,
}: {
  readonly onClaim: (claim: ClaimResponse) => void;
}) {
  const [code, setCode] = useState(() => codeFromQueryString());
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // When the URL supplied the code (agent or same-device handoff),
  // focus the submit button instead of the input — the user can
  // confirm with a single Enter/click while keeping the explicit
  // intent gesture. See the code-vs-URL design note.
  const prefilled = code.length > 0;

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    if (!code.trim()) return;
    setLoading(true);
    setError(null);
    try {
      const res = await api.post<ClaimResponse>("/cli-pairings/claim", {
        code: code.trim(),
      });
      if (!isPairingKind(res.kind)) {
        // Version skew: the CLI / server emitted a kind this frontend
        // doesn't know how to render. We've already transitioned the
        // pairing to `claimed` server-side; if we just show an error,
        // the waiting CLI keeps polling until TTL. Fire a cancel so
        // the CLI exits promptly with a "please update web app" hint.
        try {
          await api.post(
            `/cli-pairings/${encodeURIComponent(res.id)}/cancel`,
            {},
          );
        } catch {
          // Best-effort. If cancel fails, the CLI times out after TTL
          // the same way it did before this guard.
        }
        setError(
          `Unsupported pairing kind from server: ${String(res.kind)}. Please update the NyxID web app, then re-run the CLI.`,
        );
        return;
      }
      onClaim(res);
    } catch (e) {
      setError(extractErrorMessage(e));
    } finally {
      setLoading(false);
    }
  }

  return (
    <form className="flex flex-col gap-4" onSubmit={(e) => void submit(e)}>
      <p className="text-[12px] text-muted-foreground">
        {prefilled
          ? "We've filled in the code from the URL. Confirm to continue."
          : "Enter the pairing code shown in your terminal. The CLI running on your remote box is waiting for you to complete the wizard here."}
      </p>
      <div className="flex flex-col gap-1.5">
        <Label htmlFor="pair-code">Pairing code</Label>
        <Input
          id="pair-code"
          value={code}
          onChange={(e) => {
            setCode(e.target.value.toUpperCase());
          }}
          placeholder="ABCD-1234"
          // Focus the input only when we couldn't prefill; otherwise
          // the submit button gets `autoFocus` below so Enter confirms.
          autoFocus={!prefilled}
          autoComplete="off"
          spellCheck={false}
          className="font-mono text-lg tracking-widest"
        />
      </div>
      {error ? (
        <p className="rounded-lg border border-destructive/40 bg-destructive/10 px-3 py-2 text-[12px] text-destructive">
          {error}
        </p>
      ) : null}
      <Button variant="primary" type="submit" disabled={loading || !code.trim()} autoFocus={prefilled}>
        {loading ? "Verifying..." : "Continue"}
      </Button>
    </form>
  );
}

/**
 * Read the optional `?code=ABCD-1234` query param the CLI embeds in
 * the pair URL. Normalizes to uppercase and strips dashes/whitespace;
 * the backend accepts either shape but the display should be
 * consistent with what a typed entry produces.
 *
 * Using `window.location` directly (rather than TanStack Router's
 * `useSearch`) because this route is declared without
 * `validateSearch` — the pair page is a leaf and doesn't re-render
 * on navigation. A single read at mount is what we want.
 */
function codeFromQueryString(): string {
  if (typeof window === "undefined") return "";
  const param = new URLSearchParams(window.location.search).get("code");
  if (!param) return "";
  const normalized = param
    .toUpperCase()
    .replace(/\s|-/g, "")
    .slice(0, 16);
  // Re-insert the conventional dash at the 4/4 split so the field
  // renders as `ABCD-1234` — matches exactly what the CLI printed,
  // making "does this match my terminal?" a one-glance check.
  if (normalized.length === 8) {
    return `${normalized.slice(0, 4)}-${normalized.slice(4)}`;
  }
  return normalized;
}

// ── Step 2: per-kind confirm ────────────────────────────────────────

function ConfirmPanel({
  claim,
  onActionComplete,
}: {
  readonly claim: ClaimResponse;
  readonly onActionComplete: (result: ActionResult) => void;
}) {
  const prefill = claim.prefill;

  // NOTE: `resumed && action_started` is handled upstream in
  // `StageRouter.onClaim` — rotations route to
  // `resumed-rotation-choice` and creates to
  // `resumed-create-warning`. ConfirmPanel therefore only sees
  // either fresh claims or pre-action resumes
  // (`resumed: true, action_started: false`) which are fully
  // recoverable by running the confirm step.

  switch (claim.kind) {
    case "ai-key":
      return (
        <AiKeyConfirm
          prefill={parseAiKeyPrefill(prefill) as AiKeyPrefill}
          pairingId={claim.id}
          onSuccess={onActionComplete}
        />
      );
    case "api-key-create":
      return (
        <ApiKeyCreateConfirm
          prefill={prefill as unknown as ApiKeyCreatePrefill}
          pairingId={claim.id}
          onSuccess={onActionComplete}
        />
      );
    case "api-key-rotate":
      return (
        <ApiKeyRotateConfirm
          prefill={prefill as unknown as RotatePrefill}
          pairingId={claim.id}
          onSuccess={onActionComplete}
        />
      );
    case "node-register-token":
      return (
        <NodeRegisterConfirm
          prefill={prefill as unknown as NodeRegisterPrefill}
          pairingId={claim.id}
          onSuccess={onActionComplete}
        />
      );
    case "node-rotate-token":
      return (
        <NodeRotateConfirm
          prefill={prefill as unknown as RotatePrefill}
          pairingId={claim.id}
          onSuccess={onActionComplete}
        />
      );
    case "service-account-create":
      return (
        <ServiceAccountCreateConfirm
          prefill={prefill as unknown as ServiceAccountCreatePrefill}
          pairingId={claim.id}
          onSuccess={onActionComplete}
        />
      );
    case "service-account-rotate-secret":
      return (
        <ServiceAccountRotateSecretConfirm
          prefill={prefill as unknown as RotatePrefill}
          pairingId={claim.id}
          onSuccess={onActionComplete}
        />
      );
    case "developer-app-create":
      return (
        <DeveloperAppCreateConfirm
          prefill={prefill as unknown as DeveloperAppCreatePrefill}
          pairingId={claim.id}
          onSuccess={onActionComplete}
        />
      );
    case "developer-app-rotate-secret":
      return (
        <DeveloperAppRotateSecretConfirm
          prefill={prefill as unknown as RotatePrefill}
          pairingId={claim.id}
          onSuccess={onActionComplete}
        />
      );
    case "mfa-setup":
      return (
        <MfaSetupConfirm
          pairingId={claim.id}
          onSuccess={onActionComplete}
        />
      );
  }
}

/**
 * Terminal warning for `resumed && action_started` claims on
 * NON-rotation flows (api-key-create, node-register-token,
 * ai-key). Unlike rotations, the ack can't be reconstructed from
 * `prefill` — the created resource id lives only in the tab that
 * ran the first destructive call. So there's no recovery; our
 * only job here is to stop lying to the user and to stop
 * stranding the waiting CLI.
 *
 * On mount we fire `/cli-pairings/{id}/cancel` fire-and-forget
 * so the CLI exits promptly with a Cancelled status instead of
 * polling until TTL. The user sees guidance pointing them at
 * `/keys` / `/api-keys` — the resource may already exist from
 * the first tab even though this tab can't finish the pairing,
 * and they can manage it (or delete a stray one) from there.
 */
function ResumedCreateWarningPanel({
  claim,
  onAcknowledged,
}: {
  readonly claim: ClaimResponse;
  readonly onAcknowledged: () => void;
}) {
  const [cancelling, setCancelling] = useState(false);
  const [manageHref, manageLabel] = managementTargetFor(claim.kind);

  // Intentionally NOT auto-cancelling on mount. The same
  // `resumed && action_started` state is reached by both
  //   (a) "the other tab is dead, strand-recover me" and
  //   (b) "I just refreshed this tab while `/complete` was in
  //       flight".
  // Auto-cancelling would flip case (b) to Cancelled and reject
  // the still-running original tab's `/complete` retry, so the
  // CLI would exit as cancelled even though the resource was
  // successfully created. Let the user decide.
  async function cancelPairing() {
    if (cancelling) return;
    setCancelling(true);
    try {
      await api.post(
        `/cli-pairings/${encodeURIComponent(claim.id)}/cancel`,
        {},
      );
    } catch {
      // Best-effort — if cancel fails the CLI falls back to its
      // TTL timeout. We still close the panel so the user
      // isn't stuck.
    } finally {
      onAcknowledged();
    }
  }

  return (
    <div className="flex flex-col gap-4">
      <h2 className="text-xl font-semibold">
        This pairing was already started
      </h2>
      <p className="text-[12px] text-muted-foreground">
        Another tab or window began the {labelForKind(claim.kind)}{" "}
        flow for this code. We can't safely replay it from here —
        that first tab may still be finishing, or it may have
        already created the resource.
      </p>
      <p className="rounded-lg border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-xs text-muted-foreground">
        Check your {manageLabel} first — if a new entry is there,
        the first tab succeeded and you can close this tab. If
        nothing shows up after a minute, use "Cancel Pairing"
        below so the waiting CLI exits promptly.
      </p>
      <div className="flex flex-col gap-2 sm:flex-row">
        <a
          href={manageHref}
          className="inline-flex flex-1 items-center justify-center rounded-lg bg-primary px-3 py-2 text-[12px] font-medium text-primary-foreground hover:bg-primary/90"
        >
          Open {manageLabel}
        </a>
        <Button
          variant="outline"
          onClick={() => void cancelPairing()}
          disabled={cancelling}
          className="flex-1"
        >
          {cancelling ? "Cancelling..." : "Cancel Pairing"}
        </Button>
      </div>
    </div>
  );
}

/**
 * Where the user should look to confirm whether their prior
 * tab's create actually succeeded. All DisplayOnce-create kinds
 * land here; the mapping matches the navigation the user would
 * pick manually.
 */
function managementTargetFor(
  kind: PairingKind,
): readonly [string, string] {
  switch (kind) {
    case "api-key-create":
      return ["/keys?tab=nyxid", "NyxID API Keys page"];
    case "node-register-token":
      return ["/nodes", "Nodes page"];
    case "ai-key":
      return ["/keys?tab=services", "AI Services page"];
    // Rotation kinds don't land on this panel (they route to
    // `ResumedRotationChoicePanel` instead) but the exhaustive
    // match keeps TypeScript happy.
    case "api-key-rotate":
      return ["/keys?tab=nyxid", "NyxID API Keys page"];
    case "node-rotate-token":
      return ["/nodes", "Nodes page"];
    case "service-account-create":
    case "service-account-rotate-secret":
      return ["/admin/service-accounts", "Service Accounts page"];
    case "developer-app-create":
    case "developer-app-rotate-secret":
      return ["/developer/apps", "Developer Apps page"];
    case "mfa-setup":
      return ["/account/security", "Account Security page"];
  }
}

function labelForKind(kind: PairingKind): string {
  switch (kind) {
    case "api-key-create":
      return "API key creation";
    case "api-key-rotate":
      return "API key rotation";
    case "node-register-token":
      return "node registration";
    case "node-rotate-token":
      return "node token rotation";
    case "ai-key":
      return "service setup";
    case "service-account-create":
      return "service account creation";
    case "service-account-rotate-secret":
      return "service account secret rotation";
    case "developer-app-create":
      return "developer app creation";
    case "developer-app-rotate-secret":
      return "developer app secret rotation";
    case "mfa-setup":
      return "MFA enrollment";
  }
}

// ── Step 3: DisplayOnce ─────────────────────────────────────────────
//
// IMPORTANT: `/cli-pairings/{id}/complete` is POSTed immediately on
// mount, NOT on the "I've saved it" click. This closes a
// refresh-replay window:
//
//   before:  action → secret on-screen → user clicks "I saved it"
//            → POST /complete → transitions to Completed
//            ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
//            Refresh in this window → re-claim (Claimed state is
//            idempotent) → user runs the action again → ORIGINAL
//            secret the user saved is now invalid.
//
//   after:   action → POST /complete (Completed) → secret on-screen
//            → user clicks "Close" (no API call)
//            Refresh after /complete → re-claim sees Completed →
//            frontend shows "too late, check CLI".
//
// The CLI's next poll now sees Completed the instant the destructive
// step finishes — same contract as the local-server wizard, which
// tears down its HTTP server the moment the browser POSTs complete.

/**
 * Interstitial between the destructive action and the secret
 * render for DisplayOnce flows. Purely presentational: the parent
 * (`StageRouter`) fires `/cli-pairings/{id}/complete` in the same
 * tick as the destructive API success so there is no render-gap
 * where a reload could strand the pairing. While `completeError`
 * is `null` we show a spinner; on failure the parent re-renders
 * us with the message and we surface a retry button that calls
 * back into the parent.
 */
function NotifyingCliPanel({
  result,
  completeError,
  onRetry,
}: {
  readonly result: ActionResult;
  readonly completeError: string | null;
  readonly onRetry: () => void;
}) {
  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-1">
        <h2 className="text-xl font-semibold">
          Notifying CLI...
        </h2>
        <p className="text-[12px] text-muted-foreground">
          Your {describeResultKind(result.kind)} is ready on the
          server. Once the CLI acknowledges, we'll show the secret
          here to copy. If this page closes before then, the CLI
          will time out cleanly — re-run the command to try again.
        </p>
      </div>
      {completeError ? (
        <>
          <p className="rounded-lg border border-destructive/40 bg-destructive/10 px-3 py-2 text-[12px] text-destructive">
            Couldn't notify CLI: {completeError}. The server-side
            action succeeded; retry to receive the secret, or run
            the CLI command again for a fresh pairing.
          </p>
          <button
            type="button"
            onClick={onRetry}
            className="rounded-lg border bg-primary px-4 py-2 text-[12px] font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-60"
          >
            Retry
          </button>
        </>
      ) : (
        <Skeleton className="h-9 w-full" />
      )}
    </div>
  );
}

function describeResultKind(kind: ActionResult["kind"]): string {
  switch (kind) {
    case "ai-key":
      return "service";
    case "api-key-create":
      return "API key";
    case "api-key-rotate":
      return "rotated API key";
    case "node-register-token":
      return "registration token";
    case "node-rotate-token":
      return "rotated node credentials";
    case "service-account-create":
      return "service account";
    case "service-account-rotate-secret":
      return "rotated service account secret";
    case "developer-app-create":
      return "developer app";
    case "developer-app-rotate-secret":
      return "rotated developer app secret";
    case "mfa-setup":
      return "MFA enrollment";
  }
}

/**
 * Pure render of the DisplayOnce secret. `/cli-pairings/{id}/complete`
 * has ALREADY landed by the time this mounts (see `NotifyingCliPanel`
 * and the state machine in `StageRouter`), so closing the tab here
 * only loses the unshown-to-the-user-now secret; the CLI was already
 * notified.
 */
function SecretPanel({
  result,
  onAcknowledged,
}: {
  readonly result: ActionResult;
  readonly onAcknowledged: () => void;
}) {
  const content = (() => {
    switch (result.kind) {
      case "ai-key":
        // Unreachable — ai-key routes through `AiKeyAckPanel`. Kept
        // exhaustive so the switch type-checks.
        return null;
      case "api-key-create":
        return (
          <DisplayOncePanel
            title="API key created"
            description="Copy this key into your CLI / agent environment. NyxID will never show it again."
            secret={result.full_key}
            ackButtonLabel="Close — I've saved the key"
            onAcknowledge={onAcknowledged}
            isAcknowledging={false}
          />
        );
      case "api-key-rotate":
        return (
          <DisplayOncePanel
            title="API key rotated"
            description="The previous key has been revoked. Update any callers using it."
            secret={result.full_key}
            ackButtonLabel="Close — I've saved the new key"
            onAcknowledge={onAcknowledged}
            isAcknowledging={false}
          />
        );
      case "node-register-token":
        return (
          <DisplayOncePanel
            title="Registration token generated"
            description="Use this token once with nyxid node register, then it's gone."
            secret={result.token}
            ackButtonLabel="Close — I've saved the token"
            onAcknowledge={onAcknowledged}
            isAcknowledging={false}
          />
        );
      case "node-rotate-token":
        return (
          <DisplayOncePanel
            title="Node credentials rotated"
            description="Paste both values into the node agent config; the previous pair is revoked."
            secret={result.auth_token}
            secondarySecret={{
              label: "Signing secret",
              value: result.signing_secret,
            }}
            ackButtonLabel="Close — I've saved both values"
            onAcknowledge={onAcknowledged}
            isAcknowledging={false}
          />
        );
      case "service-account-create":
        return (
          <DisplayOncePanel
            title="Service account created"
            description="Save the client_secret — it isn't shown again. Use it with the OAuth client_credentials flow."
            secret={result.client_secret}
            secondarySecret={{ label: "Client ID", value: result.client_id }}
            ackButtonLabel="Close — I've saved the secret"
            onAcknowledge={onAcknowledged}
            isAcknowledging={false}
          />
        );
      case "service-account-rotate-secret":
        return (
          <DisplayOncePanel
            title="Service account secret rotated"
            description="All previously-issued tokens have been revoked. Save this new client_secret — it isn't shown again."
            secret={result.client_secret}
            secondarySecret={{ label: "Client ID", value: result.client_id }}
            ackButtonLabel="Close — I've saved the new secret"
            onAcknowledge={onAcknowledged}
            isAcknowledging={false}
          />
        );
      case "developer-app-create":
        return (
          <DisplayOncePanel
            title="Developer app created"
            description="Save the client_secret — it isn't shown again. Use it to sign Sign-in-with-NyxID requests."
            secret={result.client_secret}
            ackButtonLabel="Close — I've saved the secret"
            onAcknowledge={onAcknowledged}
            isAcknowledging={false}
          />
        );
      case "developer-app-rotate-secret":
        return (
          <DisplayOncePanel
            title="Developer app secret rotated"
            description="The previous client_secret no longer authenticates. Update any deployments using it."
            secret={result.client_secret}
            ackButtonLabel="Close — I've saved the new secret"
            onAcknowledge={onAcknowledged}
            isAcknowledging={false}
          />
        );
      case "mfa-setup":
        return (
          <RecoveryCodesPanel
            codes={result.recovery_codes}
            onAcknowledged={onAcknowledged}
          />
        );
    }
  })();

  return <div className="flex flex-col gap-4">{content}</div>;
}

// ── Step 3b (ai-key only): fire ack immediately ─────────────────────

/**
 * Ack confirmation for ai-key flows. Purely presentational: the
 * parent (`StageRouter`) owns the `/complete` POST so it fires in
 * the same turn as the destructive API's success handler and
 * can't be orphaned by a render-gap reload.
 */
function AiKeyAckPanel({
  result,
  completeError,
  onRetry,
}: {
  readonly result: AiKeyPairingSuccess;
  readonly completeError: string | null;
  readonly onRetry: () => void;
}) {
  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-1">
        <h2 className="text-xl font-semibold">
          Service created
        </h2>
        <p className="text-[12px] text-muted-foreground">
          <strong>{result.label}</strong> is now connected. Check your
          terminal — the CLI is printing the proxy URL and next steps.
        </p>
      </div>
      {completeError ? (
        <>
          <p className="rounded-lg border border-destructive/40 bg-destructive/10 px-3 py-2 text-[12px] text-destructive">
            Couldn't notify CLI: {completeError}
          </p>
          <Button variant="primary" onClick={onRetry}>Retry</Button>
        </>
      ) : (
        <Skeleton className="h-9 w-full" />
      )}
    </div>
  );
}

/**
 * Disambiguation screen for `resumed && action_started` rotation
 * claims. `action_started_at` alone is ambiguous — it's latched
 * by `reservePairingAction` BEFORE the destructive rotate call
 * and intentionally left set on 5xx / network errors. So the
 * first tab may have:
 *   (a) rotated successfully then lost `/complete`, or
 *   (b) never rotated (the destructive call itself failed).
 * We can't tell from server state, but the user can: did the
 * other tab show them a new secret? This panel asks.
 */
function ResumedRotationChoicePanel({
  kind,
  onConfirmSuccess,
  onCancel,
}: {
  readonly kind: PairingKind;
  readonly onConfirmSuccess: () => void;
  readonly onCancel: () => void;
}) {
  return (
    <div className="flex flex-col gap-4">
      <h2 className="text-xl font-semibold">
        This pairing was already started
      </h2>
      <p className="text-[12px] text-muted-foreground">
        Another tab or window began the {labelForKind(kind)} flow
        for this code but never finished notifying the CLI. We
        need to know what happened there so we don't tell the CLI
        something false:
      </p>
      <ul className="ml-4 list-disc text-[12px] text-muted-foreground">
        <li>
          If the other tab <strong>showed you a new secret you
          saved</strong> (the rotation succeeded), click "Notify
          CLI" below so the CLI stops waiting.
        </li>
        <li>
          If you <strong>did not see a secret</strong> (the
          rotation may have failed), click "Cancel Pairing". The
          CLI will exit; re-run the original command to try again.
        </li>
      </ul>
      <div className="flex flex-col gap-2 sm:flex-row">
        <Button variant="primary" onClick={onConfirmSuccess} className="flex-1">
          Notify CLI
        </Button>
        <Button
          variant="outline"
          onClick={onCancel}
          className="flex-1"
        >
          Cancel Pairing
        </Button>
      </div>
    </div>
  );
}

/**
 * Recovery screen for `resumed && action_started` rotation
 * claims. The first tab already rotated the credential but lost
 * the `/complete` POST; this panel resends the ack so the CLI
 * can exit cleanly. No secret is shown because the rotated
 * secret was one-time and is no longer recoverable — the user
 * will have saved it from the first tab or must re-rotate later
 * via `nyxid api-key rotate` / `nyxid node rotate-token`.
 */
function ResendingAckPanel({
  kind,
  completeError,
  onRetry,
}: {
  readonly kind: PairingKind;
  readonly completeError: string | null;
  readonly onRetry: () => void;
}) {
  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-1">
        <h2 className="text-xl font-semibold">
          Notifying CLI...
        </h2>
        <p className="text-[12px] text-muted-foreground">
          This {labelForKind(kind)} was already completed on
          another tab. We couldn't find the secret here, but we
          can still tell the CLI so it stops waiting. The new
          credential should be in whichever tab finished the
          rotation; check there for the secret to save.
        </p>
      </div>
      {completeError ? (
        <>
          <p className="rounded-lg border border-destructive/40 bg-destructive/10 px-3 py-2 text-[12px] text-destructive">
            Couldn't notify CLI: {completeError}
          </p>
          <Button variant="primary" onClick={onRetry}>Retry</Button>
        </>
      ) : (
        <Skeleton className="h-9 w-full" />
      )}
    </div>
  );
}

// ── Step 4: done ────────────────────────────────────────────────────

function DonePanel() {
  return (
    <div className="flex flex-col gap-3 text-center">
      <h2 className="text-xl font-semibold">
        Pairing complete
      </h2>
      <p className="text-[12px] text-muted-foreground">
        You can close this tab. Your CLI should now show a success
        message in the terminal.
      </p>
    </div>
  );
}

// ── helpers ─────────────────────────────────────────────────────────

function ackForResult(result: ActionResult): AckPayload {
  switch (result.kind) {
    case "ai-key":
      // Unreachable from SecretPanel (ai-key routes through
      // AiKeyAckPanel instead) but kept exhaustive for safety.
      return {
        acknowledged: true,
        service_id: result.service_id,
        slug: result.slug,
        label: result.label,
      };
    case "api-key-create":
      return { acknowledged: true, api_key_id: result.api_key_id };
    case "api-key-rotate":
      return { acknowledged: true, resource_id: result.resource_id };
    case "node-register-token":
      return { acknowledged: true, token_id: result.token_id };
    case "node-rotate-token":
      return { acknowledged: true, resource_id: result.resource_id };
    case "service-account-create":
      return {
        acknowledged: true,
        service_account_id: result.service_account_id,
      };
    case "service-account-rotate-secret":
      return { acknowledged: true, resource_id: result.resource_id };
    case "developer-app-create":
      return {
        acknowledged: true,
        developer_app_id: result.developer_app_id,
      };
    case "developer-app-rotate-secret":
      return { acknowledged: true, resource_id: result.resource_id };
    case "mfa-setup":
      return { acknowledged: true, factor_id: result.factor_id };
  }
}

function extractErrorMessage(e: unknown): string {
  if (e instanceof ApiError) return e.message;
  if (e instanceof Error) return e.message;
  return "Something went wrong. Please try again.";
}

// Keep the unused type reference so PairingKind stays exported.
export type { PairingKind };

// Compile-time guard for the `claim.kind as WizardFlow` casts (in
// `stageFlow` and every transition into `done`). Those casts are sound only
// while PairingKind (the kinds the backend can send) and WizardFlow (the
// kinds the wizard chrome can render) stay identical. If they diverge,
// `resolveStep`'s exhaustive `switch (flow)` would fall through to
// `undefined` at runtime for the unmapped kind — exactly the kind of silent
// gap NyxID#734 was about. This turns adding a kind to one union but not the
// other into a build error instead. The exported alias keeps the helpers
// "used"; it has no runtime footprint.
type _UnionEquals<X, Y> =
  (<T>() => T extends X ? 1 : 2) extends (<T>() => T extends Y ? 1 : 2)
    ? true
    : false;
type _ExpectTrue<T extends true> = T;
export type _PairingKindMatchesWizardFlow = _ExpectTrue<
  _UnionEquals<PairingKind, WizardFlow>
>;
