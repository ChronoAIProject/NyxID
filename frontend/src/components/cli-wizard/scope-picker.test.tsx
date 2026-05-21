import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { useState } from "react"
import { describe, expect, it, vi } from "vitest"

import { ScopePicker } from "./scope-picker"
import { API_KEY_SCOPES, type ApiKeyScope } from "@/schemas/api-keys"

// Issue #787: ScopePicker is the chip row that builds the API-key scope
// set. Contracts: (1) toggling an unchecked chip adds it to the set;
// (2) toggling a checked chip removes it (toggle off); (3) onChange always
// receives the FULL updated set, not a delta; (4) the empty set surfaces
// the required-state copy + aria-invalid="true" on the group; and (5)
// every backend scope is offered as a chip.

/** Each chip is a checkbox whose accessible name is the scope string. */
function chip(scope: ApiKeyScope) {
  return screen.getByRole("checkbox", { name: scope })
}

/** Controlled wrapper mirroring how a confirm panel owns the scope set. */
function Harness({
  initial,
  onChangeSpy,
}: {
  readonly initial: ApiKeyScope[]
  readonly onChangeSpy: (next: Set<ApiKeyScope>) => void
}) {
  const [value, setValue] = useState<Set<ApiKeyScope>>(new Set(initial))
  return (
    <ScopePicker
      value={value}
      onChange={(next) => {
        onChangeSpy(next)
        setValue(next)
      }}
    />
  )
}

describe("ScopePicker", () => {
  it("renders one chip per backend scope, with a reflected checked state", () => {
    render(<ScopePicker value={new Set(["read"])} onChange={() => {}} />)

    for (const scope of API_KEY_SCOPES) {
      expect(chip(scope)).toBeInTheDocument()
    }
    expect(chip("read")).toBeChecked()
    expect(chip("write")).not.toBeChecked()
  })

  it("adds a scope to the set when an unchecked chip is toggled on", async () => {
    const user = userEvent.setup()
    const onChange = vi.fn()
    render(<Harness initial={["read"]} onChangeSpy={onChange} />)

    await user.click(chip("write"))

    // onChange receives the full set, not just the delta.
    expect(onChange).toHaveBeenCalledTimes(1)
    const next = onChange.mock.calls[0]![0] as Set<ApiKeyScope>
    expect([...next].sort()).toEqual(["read", "write"])
    expect(chip("write")).toBeChecked()
  })

  it("removes a scope from the set when a checked chip is toggled off", async () => {
    const user = userEvent.setup()
    const onChange = vi.fn()
    render(<Harness initial={["read", "write"]} onChangeSpy={onChange} />)

    await user.click(chip("read"))

    const next = onChange.mock.calls[0]![0] as Set<ApiKeyScope>
    expect([...next]).toEqual(["write"])
    expect(chip("read")).not.toBeChecked()
  })

  it("flags the empty set as invalid and shows the required-scope copy", () => {
    render(<ScopePicker value={new Set()} onChange={() => {}} />)

    const group = screen.getByRole("group", { name: "Scopes" })
    expect(group).toHaveAttribute("aria-invalid", "true")
    expect(
      screen.getByText("At least one scope is required."),
    ).toBeInTheDocument()
  })

  it("shows the hint (not the required warning) once a scope is selected", () => {
    render(
      <ScopePicker
        value={new Set(["read"])}
        onChange={() => {}}
        hint="Pick the minimum needed."
      />,
    )

    expect(screen.getByRole("group", { name: "Scopes" })).toHaveAttribute(
      "aria-invalid",
      "false",
    )
    expect(screen.getByText("Pick the minimum needed.")).toBeInTheDocument()
    expect(
      screen.queryByText("At least one scope is required."),
    ).not.toBeInTheDocument()
  })
})
