/**
 * Step-label mapping for the shared wizard chrome.
 *
 * Mode A's header shows "Step X of Y · <label>" on every screen. Mode B
 * (remote pairing) currently shows no step indicator at all. This module
 * encodes a single table the shell header can read, keyed by flow + phase,
 * so both modes render identical step copy.
 *
 * Source of truth for the copy values:
 * - Mode A `cli/src/wizard/assets/wizard.js` per-flow init functions
 *   (ai-key: STEP_LABELS line 251, api-key-create: line 1952, rotate: 1672,
 *   node-register: 1765, node-rotate: 1952)
 * - Mode A `wizard.html:19` default "Step 1 of 3 · pick a service"
 *
 * Flows = the 5 `PairingFlow` kinds from `cli/src/wizard/pairing.rs:43-62`.
 * Phases = the discriminated-union `phase` values from
 * `frontend/src/pages/cli-pair/index.tsx:52-145` plus a neutral
 * `enter-code` used before any flow is known.
 */

export type WizardFlow =
  | "ai-key"
  | "api-key-create"
  | "api-key-rotate"
  | "node-register-token"
  | "node-rotate-token"

export type WizardPhase =
  | "enter-code"
  | "claimed"
  | "notifying-cli"
  | "secret"
  | "acking"
  | "done"
  | "resumed-rotation-choice"
  | "resumed-create-warning"
  | "resending-ack"

export interface WizardStep {
  readonly current: number
  readonly total: number
  readonly label: string
}

/**
 * Step totals by flow. Mirrors Mode A:
 * - ai-key is a 3-step flow (catalog → credential → confirm)
 * - api-key-create is 3 steps (configure → save → done)
 * - rotate/register flows are 2 steps (confirm → save)
 */
const TOTALS: Record<WizardFlow, number> = {
  "ai-key": 3,
  "api-key-create": 3,
  "api-key-rotate": 2,
  "node-register-token": 2,
  "node-rotate-token": 2,
}

/**
 * Resolve the (flow, phase) pair to "Step X of Y · label" copy.
 *
 * `enter-code` is pre-flow (no `WizardFlow` known yet) so it gets a neutral
 * "Step 1 of 3 · enter code" label that fits any flow — once the user
 * submits the code and the claim returns, the actual flow takes over.
 *
 * For the ai-key flow, the confirm panel has two sub-states (catalog
 * pick vs credential form). The `slugPicked` flag (tracked upstream)
 * bumps the step from 1 → 2 inside the same `claimed` phase so the
 * header reflects where the user actually is in the guided flow.
 *
 * Recovery phases (`resumed-*`, `resending-ack`) intentionally break the
 * "Step X of Y" pattern — they are out-of-band from the main flow and
 * carry a "Recovery · …" prefix instead so the user knows they're not in
 * a normal step.
 */
export function resolveStep(
  phase: WizardPhase,
  flow?: WizardFlow,
  opts?: { readonly slugPicked?: boolean },
): WizardStep {
  if (phase === "enter-code" || flow === undefined) {
    return { current: 1, total: 3, label: "enter code" }
  }

  const total = TOTALS[flow]

  if (phase === "resumed-rotation-choice") {
    return { current: 1, total, label: "Recovery · confirm outcome" }
  }
  if (phase === "resumed-create-warning") {
    return { current: 1, total, label: "Recovery · pairing already started" }
  }
  if (phase === "resending-ack") {
    return { current: total, total, label: "Recovery · notifying CLI" }
  }
  if (phase === "done") {
    return { current: total, total, label: "done" }
  }

  // Per-flow branching for the main-path phases (claimed → notifying/secret/acking).
  switch (flow) {
    case "ai-key":
      if (phase === "claimed") {
        // `slugPicked` true → user is on the credential form (step 2).
        // Default / unset → catalog grid (step 1).
        return opts?.slugPicked
          ? { current: 2, total, label: "enter credential" }
          : { current: 1, total, label: "pick a service" }
      }
      if (phase === "notifying-cli" || phase === "acking")
        return { current: 3, total, label: "notifying CLI" }
      return { current: 3, total, label: "done" }
    case "api-key-create":
      if (phase === "claimed") return { current: 2, total, label: "configure scope" }
      if (phase === "notifying-cli") return { current: 3, total, label: "notifying CLI" }
      if (phase === "secret") return { current: 3, total, label: "save the value" }
      return { current: 3, total, label: "done" }
    case "api-key-rotate":
      if (phase === "claimed") return { current: 1, total, label: "confirm rotate" }
      if (phase === "notifying-cli") return { current: 2, total, label: "notifying CLI" }
      if (phase === "secret") return { current: 2, total, label: "save the value" }
      return { current: 2, total, label: "done" }
    case "node-register-token":
      if (phase === "claimed") return { current: 1, total, label: "name this node" }
      if (phase === "notifying-cli") return { current: 2, total, label: "notifying CLI" }
      if (phase === "secret") return { current: 2, total, label: "save the value" }
      return { current: 2, total, label: "done" }
    case "node-rotate-token":
      if (phase === "claimed") return { current: 1, total, label: "confirm rotate" }
      if (phase === "notifying-cli") return { current: 2, total, label: "notifying CLI" }
      if (phase === "secret") return { current: 2, total, label: "save the value" }
      return { current: 2, total, label: "done" }
  }
}

/**
 * Render a step into the header's text: `Step 2 of 3 · configure scope`.
 */
export function formatStepLabel(step: WizardStep): string {
  if (step.label.startsWith("Recovery")) {
    // Recovery phases show "Recovery · …" on its own, no step numbering.
    return step.label
  }
  return `Step ${step.current} of ${step.total} · ${step.label}`
}
