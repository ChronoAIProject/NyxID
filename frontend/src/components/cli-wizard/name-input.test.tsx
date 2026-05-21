import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { useState } from "react"
import { describe, expect, it, vi } from "vitest"

import { NameInput } from "./name-input"
import { nodeNameSchema, serviceLabelSchema } from "@/schemas/cli-wizard"

// Issue #787: NameInput is the validated free-text field every confirm
// panel reuses. Its contracts are: (1) keystrokes propagate via onChange;
// (2) invalid values surface the schema's first error message + flip
// aria-invalid; (3) validity is bubbled up via onValidityChange so the
// parent can gate submit; (4) optional+empty is treated as valid; and
// (5) the hint shows only while there's no error. These pin each branch.

/** Controlled wrapper so userEvent typing actually mutates the value. */
function Harness({
  onValidityChange,
  optional,
  hint,
  initial = "",
}: {
  readonly onValidityChange?: (valid: boolean) => void
  readonly optional?: boolean
  readonly hint?: string
  readonly initial?: string
}) {
  const [value, setValue] = useState(initial)
  return (
    <NameInput
      label="Node name"
      schema={nodeNameSchema}
      value={value}
      onChange={setValue}
      onValidityChange={onValidityChange}
      optional={optional}
      hint={hint}
    />
  )
}

describe("NameInput", () => {
  it("propagates each keystroke through onChange", async () => {
    const user = userEvent.setup()
    const onChange = vi.fn()
    render(
      <NameInput
        label="Node name"
        schema={nodeNameSchema}
        value=""
        onChange={onChange}
      />,
    )

    await user.type(screen.getByLabelText("Node name"), "ab")

    // Controlled input held at "" by the parent, so each call carries the
    // single typed character — proves the change handler fires per key.
    expect(onChange).toHaveBeenCalledTimes(2)
    expect(onChange).toHaveBeenNthCalledWith(1, "a")
    expect(onChange).toHaveBeenNthCalledWith(2, "b")
  })

  it("shows the schema's first error and marks the field invalid for a bad value", async () => {
    const user = userEvent.setup()
    render(<Harness />)

    // Uppercase violates the kebab-case node-name regex.
    await user.type(screen.getByLabelText("Node name"), "BadName")

    const input = screen.getByLabelText("Node name")
    expect(input).toHaveAttribute("aria-invalid", "true")
    expect(
      screen.getByText("Lowercase letters, digits, and hyphens only"),
    ).toBeInTheDocument()
  })

  it("clears the error and marks valid once the value parses clean", async () => {
    const user = userEvent.setup()
    render(<Harness />)

    const input = screen.getByLabelText("Node name")
    await user.type(input, "valid-node-1")

    expect(input).toHaveAttribute("aria-invalid", "false")
    expect(
      screen.queryByText("Lowercase letters, digits, and hyphens only"),
    ).not.toBeInTheDocument()
  })

  it("bubbles validity up through onValidityChange as the value changes", async () => {
    const user = userEvent.setup()
    const onValidityChange = vi.fn()
    render(<Harness onValidityChange={onValidityChange} />)

    // Empty value (non-optional) is invalid on first effect run.
    expect(onValidityChange).toHaveBeenLastCalledWith(false)

    await user.type(screen.getByLabelText("Node name"), "ok")

    expect(onValidityChange).toHaveBeenLastCalledWith(true)
  })

  it("treats empty as valid when optional, with no error shown", () => {
    const onValidityChange = vi.fn()
    render(<Harness optional onValidityChange={onValidityChange} />)

    expect(onValidityChange).toHaveBeenLastCalledWith(true)
    expect(screen.getByLabelText("Node name")).toHaveAttribute(
      "aria-invalid",
      "false",
    )
  })

  it("renders the hint when valid and replaces it with the error when invalid", async () => {
    const user = userEvent.setup()
    render(<Harness hint="Lowercase only, e.g. my-laptop" />)

    // Pristine empty value: non-optional node name is invalid, so the
    // error (not the hint) renders from the start.
    expect(
      screen.getByText("Node name is required"),
    ).toBeInTheDocument()
    expect(
      screen.queryByText("Lowercase only, e.g. my-laptop"),
    ).not.toBeInTheDocument()

    await user.type(screen.getByLabelText("Node name"), "my-laptop")

    // Now valid → hint takes over, error gone.
    expect(
      screen.getByText("Lowercase only, e.g. my-laptop"),
    ).toBeInTheDocument()
    expect(screen.queryByText("Node name is required")).not.toBeInTheDocument()
  })

  it("uses a provided id for the label/input association", () => {
    render(
      <NameInput
        id="custom-id"
        label="Service label"
        schema={serviceLabelSchema}
        value="My Service"
        onChange={() => {}}
      />,
    )

    expect(screen.getByLabelText("Service label")).toHaveAttribute(
      "id",
      "custom-id",
    )
  })
})
