import { render, screen } from "@testing-library/react"
import { describe, expect, it } from "vitest"

import { DisconnectBanner } from "./disconnect-banner"

// Issue #787: the disconnect banner is the user's only signal that the
// wizard has lost contact with the CLI / pairing record. These tests pin
// the title + body copy the banner derives from (state, context,
// pairingStatus) — that copy is the contract the user reads to decide
// whether to keep the tab open or re-run the CLI command.

describe("DisconnectBanner", () => {
  it("renders the local 'Connection to CLI interrupted' message when disconnected locally", () => {
    render(<DisconnectBanner state="disconnected" context="local" />)

    expect(screen.getByRole("alert")).toBeInTheDocument()
    expect(
      screen.getByText("Connection to CLI interrupted"),
    ).toBeInTheDocument()
    expect(
      screen.getByText(/missed several heartbeat checks/i),
    ).toBeInTheDocument()
  })

  it("shows the reconnecting copy regardless of context while retrying", () => {
    // `reconnecting` short-circuits before the context/pairingStatus
    // branches, so even a "pair" + "cancelled" combination must still
    // surface the neutral retry copy.
    render(
      <DisconnectBanner
        state="reconnecting"
        context="pair"
        pairingStatus="cancelled"
      />,
    )

    expect(screen.getByText("Reconnecting…")).toBeInTheDocument()
    expect(screen.getByText("Retrying the last check…")).toBeInTheDocument()
    expect(
      screen.queryByText(/CLI sent a cancel/i),
    ).not.toBeInTheDocument()
  })

  it("shows the cancelled pairing copy when context is pair and status is cancelled", () => {
    render(
      <DisconnectBanner
        state="disconnected"
        context="pair"
        pairingStatus="cancelled"
      />,
    )

    expect(screen.getByText("CLI cancelled this pairing")).toBeInTheDocument()
    expect(
      screen.getByText(/nothing was created on the server/i),
    ).toBeInTheDocument()
  })

  it("shows the expired pairing copy when status is expired", () => {
    render(
      <DisconnectBanner
        state="disconnected"
        context="pair"
        pairingStatus="expired"
      />,
    )

    expect(screen.getByText("Pairing expired")).toBeInTheDocument()
    expect(screen.getByText(/15-minute TTL/i)).toBeInTheDocument()
  })

  it("falls back to the 'went stale' copy for an unknown/absent pairing status", () => {
    // No pairingStatus prop → the final else branch in both the title
    // and body ternaries.
    render(<DisconnectBanner state="disconnected" context="pair" />)

    expect(screen.getByText("Pairing went stale")).toBeInTheDocument()
    expect(
      screen.getByText(/pairing record is no longer reachable/i),
    ).toBeInTheDocument()
  })
})
