import { render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { useState } from "react"
import { beforeEach, describe, expect, it, vi } from "vitest"

import {
  AccessScopeCard,
  type AccessScopeState,
} from "./access-scope-card"

// Issue #787: AccessScopeCard is the Services+Nodes multi-select that
// builds the proxy access scope on an API key. Its contracts:
//   (1) toggling "Allow all" flips allow_all_* AND hides the list;
//   (2) with allow-all off, the live service/node rows render and ticking
//       a row adds its id to the corresponding selected set (and again
//       removes it);
//   (3) the loading and empty states render the right placeholder copy.
// The data comes from useKeys()/useNodes(); both are mocked.

const { mockUseKeys, mockUseNodes } = vi.hoisted(() => ({
  mockUseKeys: vi.fn(),
  mockUseNodes: vi.fn(),
}))

vi.mock("@/hooks/use-keys", () => ({ useKeys: mockUseKeys }))
vi.mock("@/hooks/use-nodes", () => ({ useNodes: mockUseNodes }))

const emptyState: AccessScopeState = {
  allowAllServices: false,
  allowAllNodes: false,
  selectedServiceIds: new Set(),
  selectedNodeIds: new Set(),
}

/** Controlled wrapper so toggles round-trip through real state. */
function Harness({
  initial = emptyState,
  onChangeSpy,
}: {
  readonly initial?: AccessScopeState
  readonly onChangeSpy?: (next: AccessScopeState) => void
}) {
  const [value, setValue] = useState(initial)
  return (
    <AccessScopeCard
      value={value}
      onChange={(next) => {
        onChangeSpy?.(next)
        setValue(next)
      }}
    />
  )
}

function group(name: "Services" | "Nodes") {
  // Each AccessGroup is the column whose master label reads
  // "Allow all services" / "Allow all nodes".
  return screen
    .getByText(`Allow all ${name.toLowerCase()}`)
    .closest("div.flex.flex-col.gap-2") as HTMLElement
}

beforeEach(() => {
  mockUseKeys.mockReturnValue({
    isLoading: false,
    data: [
      { id: "svc-1", label: "OpenAI", slug: "llm-openai" },
      { id: "svc-2", label: "GitHub", slug: "github" },
    ],
  })
  mockUseNodes.mockReturnValue({
    isLoading: false,
    data: [{ id: "node-1", name: "home-server", status: "online" }],
  })
})

describe("AccessScopeCard", () => {
  it("renders the live service rows with label + slug when allow-all is off", () => {
    render(<Harness />)

    const services = group("Services")
    expect(within(services).getByText("OpenAI")).toBeInTheDocument()
    expect(within(services).getByText("(llm-openai)")).toBeInTheDocument()
    expect(within(services).getByText("GitHub")).toBeInTheDocument()
  })

  it("flips allowAllServices and hides the service list when 'Allow all services' is ticked", async () => {
    const user = userEvent.setup()
    const onChange = vi.fn()
    render(<Harness onChangeSpy={onChange} />)

    await user.click(
      screen.getByRole("checkbox", { name: /Allow all services/i }),
    )

    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ allowAllServices: true }),
    )
    // List is replaced by nothing — the service row is gone.
    expect(screen.queryByText("(llm-openai)")).not.toBeInTheDocument()
    // Nodes group is unaffected.
    expect(
      screen.getByRole("checkbox", { name: /Allow all nodes/i }),
    ).not.toBeChecked()
  })

  it("adds a service id to the selected set when its row is ticked, and removes it when unticked", async () => {
    const user = userEvent.setup()
    const onChange = vi.fn()
    render(<Harness onChangeSpy={onChange} />)

    const services = group("Services")
    const openaiRow = within(services)
      .getByText("OpenAI")
      .closest("label") as HTMLElement
    const openaiCheckbox = within(openaiRow).getByRole("checkbox")

    await user.click(openaiCheckbox)
    expect(
      [...(onChange.mock.calls.at(-1)![0] as AccessScopeState).selectedServiceIds],
    ).toEqual(["svc-1"])

    // Tick again → removed (toggle off).
    await user.click(openaiCheckbox)
    expect(
      [...(onChange.mock.calls.at(-1)![0] as AccessScopeState).selectedServiceIds],
    ).toEqual([])
  })

  it("toggles a node id through the Nodes group independently of services", async () => {
    const user = userEvent.setup()
    const onChange = vi.fn()
    render(<Harness onChangeSpy={onChange} />)

    const nodes = group("Nodes")
    expect(within(nodes).getByText("home-server")).toBeInTheDocument()
    const nodeRow = within(nodes)
      .getByText("home-server")
      .closest("label") as HTMLElement

    await user.click(within(nodeRow).getByRole("checkbox"))

    const last = onChange.mock.calls.at(-1)![0] as AccessScopeState
    expect([...last.selectedNodeIds]).toEqual(["node-1"])
    expect([...last.selectedServiceIds]).toEqual([])
  })

  it("shows the loading placeholder while services are loading", () => {
    mockUseKeys.mockReturnValue({ isLoading: true, data: undefined })
    render(<Harness />)

    const services = group("Services")
    expect(within(services).getByText("Loading…")).toBeInTheDocument()
  })

  it("shows the empty placeholder when there are no nodes to select", () => {
    mockUseNodes.mockReturnValue({ isLoading: false, data: [] })
    render(<Harness />)

    const nodes = group("Nodes")
    expect(
      within(nodes).getByText(/None available\. Add one first/i),
    ).toBeInTheDocument()
  })
})
