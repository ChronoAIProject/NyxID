import { render, screen } from "@testing-library/react"
import { describe, expect, it } from "vitest"

import { WizardShell } from "./shell"
import type { WizardStep } from "./step-label"

// Issue #787: the shell is the chrome wrapped around every wizard step in
// both modes. Its real contracts are: (1) it renders the step's children
// inside <main>; (2) the header step indicator is conditional on the
// `step` prop and renders the formatted "Step X of Y · label" copy; and
// (3) the footer copy follows the `context` prop. These pin those
// branches rather than asserting a bare render.

const sampleStep: WizardStep = {
  current: 2,
  total: 3,
  label: "configure scope",
}

describe("WizardShell", () => {
  it("renders its children inside the main content area", () => {
    render(
      <WizardShell context="local">
        <div data-testid="step-body">STEP_CONTENT</div>
      </WizardShell>,
    )

    const body = screen.getByTestId("step-body")
    expect(body).toHaveTextContent("STEP_CONTENT")
    // Children belong to <main>, not the header/footer chrome.
    expect(body.closest("main")).not.toBeNull()
  })

  it("renders the formatted step indicator in the header when a step is given", () => {
    render(
      <WizardShell context="local" step={sampleStep}>
        <div>body</div>
      </WizardShell>,
    )

    const indicator = screen.getByText("Step 2 of 3 · configure scope")
    expect(indicator).toBeInTheDocument()
    expect(indicator.closest("header")).not.toBeNull()
  })

  it("omits the step indicator entirely when no step prop is provided", () => {
    render(
      <WizardShell context="local">
        <div>body</div>
      </WizardShell>,
    )

    expect(screen.queryByText(/Step \d of \d/)).not.toBeInTheDocument()
  })

  // The card has to hug its content. With `flex-1` on <main> (and
  // `min-h-screen` on the column) a short step stretched the card to the
  // full viewport height, leaving a large empty panel below the last
  // control. Pin the two classes that caused it plus the two that keep
  // tall steps scrolling internally — not the whole class string.
  it("sizes the card to its content instead of the viewport", () => {
    render(
      <WizardShell context="local">
        <div data-testid="step-body">body</div>
      </WizardShell>,
    )

    const main = screen.getByTestId("step-body").closest("main")
    expect(main).not.toBeNull()
    expect(main).not.toHaveClass("flex-1")
    // Floor so a transient state can't collapse the card to a sliver.
    expect(main).toHaveClass("min-h-[240px]")

    const column = main?.parentElement
    expect(column).not.toHaveClass("min-h-screen")
    // Still capped, so content taller than the window scrolls in-card.
    expect(column).toHaveClass("max-h-screen")
    expect(main).toHaveClass("overflow-y-auto")
  })

  it("passes context through to the footer (local trust copy)", () => {
    render(
      <WizardShell context="local" localOrigin="127.0.0.1:9000">
        <div>body</div>
      </WizardShell>,
    )

    expect(screen.getByText(/Nothing leaves your machine/i)).toBeInTheDocument()
    expect(screen.getByText("127.0.0.1:9000")).toBeInTheDocument()
  })

  it("passes context through to the footer (remote pairing copy)", () => {
    render(
      <WizardShell context="pair">
        <div>body</div>
      </WizardShell>,
    )

    expect(screen.getByText(/Pairing with a remote CLI/i)).toBeInTheDocument()
    expect(
      screen.queryByText(/Nothing leaves your machine/i),
    ).not.toBeInTheDocument()
  })
})
