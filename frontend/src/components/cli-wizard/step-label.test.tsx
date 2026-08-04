import { describe, expect, it } from "vitest"

import { ENTER_CODE_STEP, formatStepLabel, resolveStep } from "./step-label"

/** [current, total, label] — the shape resolveStep returns, flattened for the table. */
type WizardStepTuple = readonly [number, number, string]

// Issue #787: step-label is the single source of truth for the
// "Step X of Y · <label>" copy in the wizard header, shared across both
// Mode A (local) and Mode B (pairing). The contract is the (flow, phase)
// -> step mapping plus formatStepLabel's Recovery-vs-numbered rendering.
// These pin the distinct states (current step number changes per phase,
// total changes per flow, and the Recovery break-out) rather than just
// "it returns an object".

describe("ENTER_CODE_STEP — the sole pre-flow step", () => {
  // NyxID#734: `enter-code` is the ONLY phase that legitimately has no
  // flow, so its label is a constant the caller renders directly rather
  // than a (flow, phase) lookup. resolveStep now requires a WizardFlow for
  // every other phase, which makes the old "flow undefined past enter-code"
  // fallback — the bug behind #734 — a compile error instead of a silent
  // neutral label. The prohibited calls (`resolveStep("enter-code")`,
  // `resolveStep("claimed", undefined)`) are therefore intentionally
  // un-testable here: TypeScript rejects them at the call site.
  it("is the neutral 'Step 1 of 3 · enter code' shown before any flow is known", () => {
    expect(ENTER_CODE_STEP).toEqual({
      current: 1,
      total: 3,
      label: "enter code",
    })
  })
})

describe("resolveStep — ai-key flow slugPicked branch", () => {
  it("is step 1 'pick a service' on claimed when no slug is picked", () => {
    expect(resolveStep("claimed", "ai-key")).toEqual({
      current: 1,
      total: 3,
      label: "pick a service",
    })
  })

  it("advances to step 2 'enter credential' once a slug is picked", () => {
    expect(resolveStep("claimed", "ai-key", { slugPicked: true })).toEqual({
      current: 2,
      total: 3,
      label: "enter credential",
    })
  })

  it("collapses notifying-cli and acking to step 3 'notifying CLI'", () => {
    expect(resolveStep("notifying-cli", "ai-key")).toEqual({
      current: 3,
      total: 3,
      label: "notifying CLI",
    })
    expect(resolveStep("acking", "ai-key")).toEqual({
      current: 3,
      total: 3,
      label: "notifying CLI",
    })
  })
})

describe("resolveStep — totals differ per flow", () => {
  it("uses the 3-step total for create flows", () => {
    expect(resolveStep("claimed", "api-key-create")).toEqual({
      current: 2,
      total: 3,
      label: "configure scope",
    })
  })

  it("uses the 2-step total for rotate flows", () => {
    expect(resolveStep("claimed", "api-key-rotate")).toEqual({
      current: 1,
      total: 2,
      label: "confirm rotate",
    })
    // 'secret' on a 2-step rotate flow lands on the final step.
    expect(resolveStep("secret", "node-rotate-token")).toEqual({
      current: 2,
      total: 2,
      label: "save the value",
    })
  })

  it("maps the done phase to the final step of the flow", () => {
    expect(resolveStep("done", "service-account-create")).toEqual({
      current: 3,
      total: 3,
      label: "done",
    })
  })
})

describe("resolveStep — recovery phases break the numbering pattern", () => {
  it("labels resumed-rotation-choice with a Recovery prefix", () => {
    expect(resolveStep("resumed-rotation-choice", "api-key-rotate")).toEqual({
      current: 1,
      total: 2,
      label: "Recovery · confirm outcome",
    })
  })

  it("labels resending-ack as Recovery on the final step", () => {
    expect(resolveStep("resending-ack", "api-key-create")).toEqual({
      current: 3,
      total: 3,
      label: "Recovery · notifying CLI",
    })
  })
})

describe("resolveStep — full per-flow step copy table", () => {
  // The header copy is the user's only orientation cue across 10 distinct
  // wizard flows. This table pins the exact (current, total, label) each
  // flow emits for its claimed / notifying-cli / secret main-path phases,
  // so a copy or step-number drift in any single flow fails one row rather
  // than going unnoticed. (ai-key + enter-code branches are covered above.)
  const cases: Array<
    [
      Parameters<typeof resolveStep>[1],
      Parameters<typeof resolveStep>[0],
      WizardStepTuple,
    ]
  > = [
    ["api-key-create", "notifying-cli", [3, 3, "notifying CLI"]],
    ["api-key-create", "secret", [3, 3, "save the value"]],
    ["api-key-rotate", "notifying-cli", [2, 2, "notifying CLI"]],
    ["api-key-rotate", "secret", [2, 2, "save the value"]],
    ["node-register-token", "claimed", [1, 2, "name this node"]],
    ["node-register-token", "notifying-cli", [2, 2, "notifying CLI"]],
    ["node-register-token", "secret", [2, 2, "save the value"]],
    ["node-rotate-token", "claimed", [1, 2, "confirm rotate"]],
    ["node-rotate-token", "notifying-cli", [2, 2, "notifying CLI"]],
    ["service-account-create", "claimed", [2, 3, "configure account"]],
    ["service-account-create", "notifying-cli", [3, 3, "notifying CLI"]],
    ["service-account-create", "secret", [3, 3, "save the secret"]],
    ["service-account-rotate-secret", "claimed", [1, 2, "confirm rotate"]],
    ["service-account-rotate-secret", "notifying-cli", [2, 2, "notifying CLI"]],
    ["service-account-rotate-secret", "secret", [2, 2, "save the secret"]],
    ["developer-app-create", "claimed", [2, 3, "configure app"]],
    ["developer-app-create", "notifying-cli", [3, 3, "notifying CLI"]],
    ["developer-app-create", "secret", [3, 3, "save the secret"]],
    ["developer-app-rotate-secret", "claimed", [1, 2, "confirm rotate"]],
    ["developer-app-rotate-secret", "notifying-cli", [2, 2, "notifying CLI"]],
    ["developer-app-rotate-secret", "secret", [2, 2, "save the secret"]],
    ["mfa-setup", "claimed", [1, 2, "scan and verify"]],
    ["mfa-setup", "notifying-cli", [2, 2, "notifying CLI"]],
    ["mfa-setup", "secret", [2, 2, "save recovery codes"]],
  ]

  it.each(cases)("%s @ %s", (flow, phase, [current, total, label]) => {
    expect(resolveStep(phase, flow)).toEqual({ current, total, label })
  })

  // The trailing `return { ...done }` arm of each flow's switch case is the
  // fallthrough for any otherwise-unmatched phase (e.g. `acking` on the
  // non-ai-key flows). Pin it lands on the flow's final "done" step.
  it("falls through to the flow's done step for unmatched phases", () => {
    expect(resolveStep("acking", "api-key-create")).toEqual({
      current: 3,
      total: 3,
      label: "done",
    })
    expect(resolveStep("acking", "node-rotate-token")).toEqual({
      current: 2,
      total: 2,
      label: "done",
    })
    expect(resolveStep("acking", "mfa-setup")).toEqual({
      current: 2,
      total: 2,
      label: "done",
    })
  })
})

describe("formatStepLabel", () => {
  it("renders normal steps as 'Step X of Y · label'", () => {
    expect(
      formatStepLabel({ current: 2, total: 3, label: "configure scope" }),
    ).toBe("Step 2 of 3 · configure scope")
  })

  it("renders Recovery labels verbatim without step numbering", () => {
    // Recovery phases are out-of-band; prefixing them with "Step X of Y"
    // would falsely imply normal forward progress.
    expect(
      formatStepLabel({ current: 1, total: 2, label: "Recovery · confirm outcome" }),
    ).toBe("Recovery · confirm outcome")
  })
})
