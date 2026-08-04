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
  | "service-account-create"
  | "service-account-rotate-secret"
  | "developer-app-create"
  | "developer-app-rotate-secret"
  | "mfa-setup"

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

/**
 * Every phase except the pre-flow `enter-code`. Once a code is claimed the
 * flow is always known, so `resolveStep` requires a `WizardFlow` for these
 * phases. Modeling the absence of a flow past `enter-code` as a compile
 * error (rather than silently rendering the neutral label) is what closes
 * NyxID#734 at the type level instead of one call site at a time.
 */
export type PostClaimPhase = Exclude<WizardPhase, "enter-code">

export interface WizardStep {
  readonly current: number
  readonly total: number
  readonly label: string
}

/**
 * The sole pre-flow step. `enter-code` is the only phase that legitimately
 * has no `WizardFlow` yet — the user hasn't claimed a code — so its label is
 * a constant the shell can render directly, not a (flow, phase) lookup.
 * Every other phase resolves through `resolveStep`, which requires a flow.
 */
export const ENTER_CODE_STEP: WizardStep = {
  current: 1,
  total: 3,
  label: "enter code",
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
  // Issue #506 — service-account / developer-app create are 3-step
  // (configure → save → done) like api-key create. Rotation flows
  // are 2-step (confirm → save). MFA enrollment has an extra
  // verify-code step inside the confirm panel but it's purely
  // internal to that panel; the outer step counter still treats
  // it as 2 (configure → save the recovery codes).
  "service-account-create": 3,
  "service-account-rotate-secret": 2,
  "developer-app-create": 3,
  "developer-app-rotate-secret": 2,
  "mfa-setup": 2,
}

/**
 * Resolve the (flow, phase) pair to "Step X of Y · label" copy.
 *
 * The pre-flow `enter-code` phase is handled by the caller via
 * `ENTER_CODE_STEP` — it's the only phase that has no `WizardFlow`, so it
 * never reaches here. Every phase this function accepts is post-claim and
 * therefore has a known flow (`PostClaimPhase` + required `flow`); there is
 * deliberately no `flow === undefined` fallback, so a lost-flow state is a
 * type error rather than a silent "Step 1 of 3 · enter code" on a screen
 * that is not asking for a code (NyxID#734).
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
  phase: PostClaimPhase,
  flow: WizardFlow,
  opts?: { readonly slugPicked?: boolean },
): WizardStep {
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
    case "service-account-create":
      if (phase === "claimed") return { current: 2, total, label: "configure account" }
      if (phase === "notifying-cli") return { current: 3, total, label: "notifying CLI" }
      if (phase === "secret") return { current: 3, total, label: "save the secret" }
      return { current: 3, total, label: "done" }
    case "service-account-rotate-secret":
      if (phase === "claimed") return { current: 1, total, label: "confirm rotate" }
      if (phase === "notifying-cli") return { current: 2, total, label: "notifying CLI" }
      if (phase === "secret") return { current: 2, total, label: "save the secret" }
      return { current: 2, total, label: "done" }
    case "developer-app-create":
      if (phase === "claimed") return { current: 2, total, label: "configure app" }
      if (phase === "notifying-cli") return { current: 3, total, label: "notifying CLI" }
      if (phase === "secret") return { current: 3, total, label: "save the secret" }
      return { current: 3, total, label: "done" }
    case "developer-app-rotate-secret":
      if (phase === "claimed") return { current: 1, total, label: "confirm rotate" }
      if (phase === "notifying-cli") return { current: 2, total, label: "notifying CLI" }
      if (phase === "secret") return { current: 2, total, label: "save the secret" }
      return { current: 2, total, label: "done" }
    case "mfa-setup":
      if (phase === "claimed") return { current: 1, total, label: "scan and verify" }
      if (phase === "notifying-cli") return { current: 2, total, label: "notifying CLI" }
      if (phase === "secret") return { current: 2, total, label: "save recovery codes" }
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
