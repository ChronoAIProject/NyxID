import { render, screen } from "@testing-library/react"
import { describe, expect, it } from "vitest"

import { WizardFooter } from "./wizard-footer"

// Issue #787: the footer is a trust anchor — it tells each user the truth
// about where the wizard page is served from and what does/doesn't leave
// their machine. The single contract is the context branch: Mode A
// ("local") promises "Nothing leaves your machine" and prints the serving
// origin; Mode B ("pair") must NOT make that promise because the page is
// served by the NyxID frontend. Getting this copy wrong is a security/UX
// trust regression, so we pin both branches plus the localOrigin source.

describe("WizardFooter", () => {
  it("shows the local trust copy and the explicit origin when context=local", () => {
    render(<WizardFooter context="local" localOrigin="127.0.0.1:8473" />)

    expect(screen.getByText("127.0.0.1:8473")).toBeInTheDocument()
    expect(screen.getByText(/Served locally from/i)).toBeInTheDocument()
    expect(
      screen.getByText(/Nothing leaves your machine/i),
    ).toBeInTheDocument()
  })

  it("falls back to window.location.host when no localOrigin is given", () => {
    // The CLI server's bound address isn't always known to the React
    // tree, so the footer falls back to the current host. happy-dom's
    // default host is "localhost".
    render(<WizardFooter context="local" />)

    expect(screen.getByText(window.location.host)).toBeInTheDocument()
  })

  it("shows the remote-pairing copy and omits the local promise when context=pair", () => {
    render(<WizardFooter context="pair" />)

    expect(screen.getByText(/Pairing with a remote CLI/i)).toBeInTheDocument()
    expect(
      screen.getByText(/Secrets never travel back through the pairing channel/i),
    ).toBeInTheDocument()
    // The "nothing leaves your machine" promise is local-only — it would
    // be a lie in the remote pairing context.
    expect(
      screen.queryByText(/Nothing leaves your machine/i),
    ).not.toBeInTheDocument()
    expect(screen.queryByText(/Served locally from/i)).not.toBeInTheDocument()
  })
})
