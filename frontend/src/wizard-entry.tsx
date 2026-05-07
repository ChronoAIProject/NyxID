/* eslint-disable react-refresh/only-export-components -- standalone entry exports a pure helper for focused tests. */

/**
 * Standalone entry for the CLI's locally-served wizard (Mode A).
 *
 * Built by `vite.wizard.config.ts` into a single self-contained
 * HTML file (JS + CSS inlined) that the CLI embeds via `rust_embed`
 * and serves from its local axum server on `127.0.0.1:<port>/wizard`.
 *
 * Bootstrap: the server splices a `<script>window.__WIZARD_BOOTSTRAP__
 * = { flow, csrf, baseUrl, context: "local", prefill }</script>` block
 * into the bundle's `<head>` at serve time. This entry reads it on
 * mount, installs a fetch shim that routes `/api/v1/*` through
 * `/api/proxy/*` (see `components/cli-wizard/client.ts`), then renders
 * the same shared confirm panels as Mode B (`/cli/pair`) against the
 * proxy.
 *
 * The pairing-specific panels in `pages/cli-pair/index.tsx` (enter
 * code, poll, resumed-rotation-choice, etc.) are NOT used here —
 * Mode A has no pairing record; the flow is confirm → execute →
 * display-once → POST `/api/proxy/complete`. `components/cli-wizard/
 * client.ts` synthesizes no-op responses for the pairing calls the
 * confirm panels still make, keeping one component hierarchy usable
 * for both modes.
 */

import { StrictMode, useEffect, useState } from "react"
import { createRoot } from "react-dom/client"
import {
  QueryClient,
  QueryClientProvider,
} from "@tanstack/react-query"
import "./app.css"

import { WizardShell } from "@/components/cli-wizard/shell"
import {
  resolveStep,
  type WizardFlow,
  type WizardPhase,
} from "@/components/cli-wizard/step-label"
import { DisplayOncePanel } from "@/components/cli-wizard/display-once-panel"
import {
  ApiKeyCreateConfirm,
  ApiKeyRotateConfirm,
  NodeRegisterConfirm,
  NodeRotateConfirm,
  type ApiKeyCreateSuccess,
  type ApiKeyRotateSuccess,
  type NodeRegisterSuccess,
  type NodeRotateSuccess,
} from "@/components/cli-wizard/confirm-panels"
import {
  AiKeyConfirm,
  type AiKeyPairingSuccess,
} from "@/components/cli-wizard/ai-key-confirm-panel"
import type {
  AiKeyPrefill,
  ApiKeyCreatePrefill,
  NodeRegisterPrefill,
  RotatePrefill,
} from "@/pages/cli-pair/types"
import {
  installHeartbeat,
  installModeAFetchShim,
  postWizardCancel,
  postWizardComplete,
  type WizardBootstrap,
} from "@/components/cli-wizard/client"
import { DisconnectBanner } from "@/components/cli-wizard/disconnect-banner"
import { Button } from "@/components/ui/button"
import { parseAiKeyPrefill } from "@/schemas/cli-wizard"

declare global {
  interface Window {
    __WIZARD_BOOTSTRAP__?: WizardBootstrap
  }
}

type ActionResult =
  | AiKeyPairingSuccess
  | ApiKeyCreateSuccess
  | ApiKeyRotateSuccess
  | NodeRegisterSuccess
  | NodeRotateSuccess

type ModeAPhase =
  | { readonly phase: "claimed" }
  | { readonly phase: "secret"; readonly result: ActionResult }
  | { readonly phase: "acking"; readonly result: AiKeyPairingSuccess }
  | { readonly phase: "done" }
  | { readonly phase: "cancelled" }

const bootstrap = window.__WIZARD_BOOTSTRAP__

// Install the Mode-A fetch shim SYNCHRONOUSLY at module load —
// BEFORE React mounts and TanStack Query fires its first fetch on
// `useKeys` / `useNodes` mount. Doing this inside a `useEffect`
// creates a race where those hooks dispatch against `/api/v1/*`
// (404s on the CLI's embedded server) a frame before the shim
// installs.
if (bootstrap) {
  installModeAFetchShim(bootstrap)
}

const queryClient = new QueryClient({
  defaultOptions: {
    queries: { retry: 1, staleTime: 30_000 },
  },
})

export function shouldShowDisconnectBanner(
  phase: ModeAPhase["phase"],
  disconnected: boolean,
): boolean {
  if (!disconnected) return false
  return phase !== "done" && phase !== "cancelled"
}

function WizardApp() {
  const [stage, setStage] = useState<ModeAPhase>({ phase: "claimed" })
  const [completeError, setCompleteError] = useState<string | null>(null)
  // ai-key sub-state: `false` → catalog grid (step 1 · pick a
  // service), `true` → credential form (step 2 · enter credential).
  // Seeded from the CLI prefill so users who ran `nyxid service add
  // <slug>` skip the grid and start at step 2.
  const [slugPicked, setSlugPicked] = useState<boolean>(
    Boolean((bootstrap?.prefill as { slug?: string } | undefined)?.slug),
  )
  // `true` after several consecutive heartbeat misses. Surfaced as a
  // non-blocking banner while the CLI's more tolerant watchdog gives
  // the browser a chance to recover.
  const [disconnected, setDisconnected] = useState(false)

  // Shim is installed at module-load above; here we kick off the
  // heartbeat so the CLI's watchdog keeps the server alive, AND we
  // watch its failure status so the UI can flip to a "CLI has gone
  // away" banner if the CLI process dies.
  useEffect(() => {
    if (!bootstrap) return
    const stopHeartbeat = installHeartbeat({
      onDisconnect: () => {
        setDisconnected(true)
      },
      onReconnect: () => {
        setDisconnected(false)
      },
    })
    return () => {
      stopHeartbeat()
    }
  }, [])

  // `bootstrap` must be set before the app renders. The server always
  // injects it; if it's missing, the bundle was loaded outside the CLI.
  // Check AFTER the hooks so we don't violate the rules-of-hooks by
  // calling them conditionally.
  if (!bootstrap) {
    return <NoBootstrapFallback />
  }

  const phase = toWizardPhase(stage.phase)
  const step = resolveStep(phase, bootstrap.flow, { slugPicked })

  async function handleSuccess(result: ActionResult) {
    // ai-key: no secret to display, but we still need to post the ack.
    if (result.kind === "ai-key") {
      setStage({ phase: "acking", result })
      await fireComplete(result, setCompleteError, setStage)
      return
    }
    setStage({ phase: "secret", result })
  }

  async function handleAck() {
    if (stage.phase !== "secret") return
    await fireComplete(stage.result, setCompleteError, setStage)
  }

  return (
    <WizardShell context="local" step={step}>
      {shouldShowDisconnectBanner(stage.phase, disconnected) ? (
        <DisconnectBanner state="disconnected" context="local" />
      ) : null}
      {stage.phase === "claimed" ? (
        <ConfirmDispatcher
          flow={bootstrap.flow}
          prefill={bootstrap.prefill ?? {}}
          onSuccess={(r) => void handleSuccess(r)}
          onCancel={() => {
            // Transition to "cancelled" immediately so the user sees
            // the click landed; the POST races them but either order
            // produces the same outcome (CLI exits with Cancelled).
            setStage({ phase: "cancelled" })
            void postWizardCancel()
          }}
          onSlugPicked={(slug) => {
            setSlugPicked(Boolean(slug))
          }}
        />
      ) : stage.phase === "secret" ? (
        <SecretDispatcher
          result={stage.result}
          completeError={completeError}
          onAck={() => void handleAck()}
        />
      ) : stage.phase === "acking" ? (
        <AckingPanel
          result={stage.result}
          completeError={completeError}
          onRetry={() => {
            void handleSuccess(stage.result)
          }}
        />
      ) : stage.phase === "cancelled" ? (
        <CancelledPanel />
      ) : (
        <DonePanel />
      )}
    </WizardShell>
  )
}

async function fireComplete(
  result: ActionResult,
  setError: (msg: string | null) => void,
  setStage: (next: ModeAPhase) => void,
): Promise<void> {
  try {
    const ack: Record<string, unknown> =
      result.kind === "ai-key"
        ? {
            acknowledged: true,
            service_id: result.service_id,
            slug: result.slug,
            label: result.label,
          }
        : result.kind === "api-key-create"
          ? {
              acknowledged: true,
              api_key_id: result.api_key_id,
            }
          : result.kind === "node-register-token"
            ? {
                acknowledged: true,
                token_id: result.token_id,
              }
            : {
                acknowledged: true,
                resource_id: result.resource_id,
              }
    await postWizardComplete(ack)
    setError(null)
    setStage({ phase: "done" })
  } catch (e) {
    setError(e instanceof Error ? e.message : String(e))
  }
}

function toWizardPhase(phase: ModeAPhase["phase"]): WizardPhase {
  // Mode A only uses a subset of the full WizardPhase union; the
  // "cancelled" phase collapses to "done" for step-label purposes
  // (nothing to track; the CLI is gone).
  if (phase === "claimed") return "claimed"
  if (phase === "secret") return "secret"
  if (phase === "acking") return "acking"
  return "done"
}

export function ConfirmDispatcher({
  flow,
  prefill,
  onSuccess,
  onCancel,
  onSlugPicked,
}: {
  readonly flow: WizardFlow
  readonly prefill: Record<string, unknown>
  readonly onSuccess: (r: ActionResult) => void
  readonly onCancel: () => void
  readonly onSlugPicked: (slug: string) => void
}) {
  // Sentinel pairingId — Mode A has no pairing record, but the
  // confirm panels pass it through to `reservePairingAction` /
  // `withRewindOnError` which are intercepted as no-ops by
  // `installModeAFetchShim`.
  const pairingId = "local"

  switch (flow) {
    case "api-key-create":
      return (
        <div className="flex flex-col gap-4">
          <ApiKeyCreateConfirm
            prefill={prefill as ApiKeyCreatePrefill}
            pairingId={pairingId}
            onSuccess={onSuccess}
          />
          <CancelLink onCancel={onCancel} />
        </div>
      )
    case "api-key-rotate":
      return (
        <div className="flex flex-col gap-4">
          <ApiKeyRotateConfirm
            prefill={prefill as unknown as RotatePrefill}
            pairingId={pairingId}
            onSuccess={onSuccess}
          />
          <CancelLink onCancel={onCancel} />
        </div>
      )
    case "node-register-token":
      return (
        <div className="flex flex-col gap-4">
          <NodeRegisterConfirm
            prefill={prefill as NodeRegisterPrefill}
            pairingId={pairingId}
            onSuccess={onSuccess}
          />
          <CancelLink onCancel={onCancel} />
        </div>
      )
    case "node-rotate-token":
      return (
        <div className="flex flex-col gap-4">
          <NodeRotateConfirm
            prefill={prefill as unknown as RotatePrefill}
            pairingId={pairingId}
            onSuccess={onSuccess}
          />
          <CancelLink onCancel={onCancel} />
        </div>
      )
    case "ai-key":
      return (
        <div className="flex flex-col gap-4">
          <AiKeyConfirm
            prefill={parseAiKeyPrefill(prefill) as AiKeyPrefill}
            pairingId={pairingId}
            onSuccess={onSuccess}
            onSlugPicked={onSlugPicked}
          />
          <CancelLink onCancel={onCancel} />
        </div>
      )
  }
}

function SecretDispatcher({
  result,
  completeError,
  onAck,
}: {
  readonly result: ActionResult
  readonly completeError: string | null
  readonly onAck: () => void
}) {
  const isCompleting = completeError === null
  if (result.kind === "api-key-create") {
    return (
      <DisplayOncePanel
        title="API key created"
        description="Save this key now — it won't be shown again."
        secret={result.full_key}
        ackButtonLabel="I have saved this — close"
        onAcknowledge={onAck}
        isAcknowledging={isCompleting && completeError === null && false}
      />
    )
  }
  if (result.kind === "api-key-rotate") {
    return (
      <DisplayOncePanel
        title="API key rotated"
        description="The previous key is revoked. Save this new value now — it won't be shown again."
        secret={result.full_key}
        ackButtonLabel="I have saved this — close"
        onAcknowledge={onAck}
        isAcknowledging={false}
      />
    )
  }
  if (result.kind === "node-register-token") {
    return (
      <DisplayOncePanel
        title="Registration token generated"
        description="Use this with `nyxid node register`. Save it now — it won't be shown again."
        secret={result.token}
        ackButtonLabel="I have saved this — close"
        onAcknowledge={onAck}
        isAcknowledging={false}
      />
    )
  }
  if (result.kind === "node-rotate-token") {
    return (
      <DisplayOncePanel
        title="Node tokens rotated"
        description="Update the node with `nyxid node rekey`. Save both values now — they won't be shown again."
        secret={result.auth_token}
        secondarySecret={{
          label: "Signing secret",
          value: result.signing_secret,
        }}
        ackButtonLabel="I have saved this — close"
        onAcknowledge={onAck}
        isAcknowledging={false}
      />
    )
  }
  // ai-key uses `acking` phase instead of `secret`, so this is
  // unreachable — keep it exhaustive.
  return <p className="text-sm text-destructive">Unknown result kind.</p>
}

function AckingPanel({
  result,
  completeError,
  onRetry,
}: {
  readonly result: AiKeyPairingSuccess
  readonly completeError: string | null
  readonly onRetry: () => void
}) {
  return (
    <div className="flex flex-col gap-4">
      <h2 className="font-serif text-[28px] font-normal">Service added</h2>
      <p className="text-sm text-muted-foreground">
        <code className="font-mono text-xs">{result.slug}</code> is now connected. Check
        your terminal for the final summary.
      </p>
      {completeError ? (
        <div className="flex flex-col gap-2">
          <p className="text-sm text-destructive">
            Couldn't notify CLI: {completeError}
          </p>
          <Button variant="outline" onClick={onRetry}>
            Retry
          </Button>
        </div>
      ) : null}
    </div>
  )
}

function DonePanel() {
  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-1">
        <h2 className="font-serif text-[28px] font-normal">Done</h2>
        <p className="text-sm text-muted-foreground">
          You can close this tab and return to your terminal.
        </p>
      </div>
    </div>
  )
}

function CancelledPanel() {
  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-1">
        <h2 className="font-serif text-[28px] font-normal">Cancelled</h2>
        <p className="text-sm text-muted-foreground">
          Nothing was created. You can close this tab — your CLI should
          already be back at the prompt.
        </p>
      </div>
    </div>
  )
}

function CancelLink({ onCancel }: { readonly onCancel: () => void }) {
  return (
    <button
      type="button"
      onClick={onCancel}
      className="self-start text-xs text-muted-foreground underline underline-offset-2 hover:text-foreground"
    >
      Cancel and return to terminal
    </button>
  )
}

function NoBootstrapFallback() {
  return (
    <div className="mx-auto flex min-h-screen max-w-md items-center justify-center p-6">
      <div className="w-full rounded-xl border border-border bg-card p-8">
        <h1 className="font-serif text-xl">Wizard bootstrap missing</h1>
        <p className="mt-2 text-sm text-muted-foreground">
          This page expects to be served by the <code>nyxid</code> CLI's local
          wizard server, which injects bootstrap config on request. Open the URL
          printed by the CLI instead.
        </p>
      </div>
    </div>
  )
}

const root = document.getElementById("wizard-root")
if (root) {
  createRoot(root).render(
    <StrictMode>
      <QueryClientProvider client={queryClient}>
        <WizardApp />
      </QueryClientProvider>
    </StrictMode>,
  )
}
