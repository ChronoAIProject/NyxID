import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { MachineGrantPicker } from "./machine-grant-picker";

vi.mock("@/hooks/use-nodes", () => ({ useNodes: () => ({ data: [], error: null }) }));
vi.mock("@/hooks/use-saved-logins", () => ({ useSavedLogins: () => ({ data: [], error: null }) }));

afterEach(cleanup);

it.each([
  ["machines", "Machines", "Pair a machine first, then grant it here."],
  ["logins", "Saved logins", "Save a login first, then grant it here."],
] as const)("tells the owner how to get a %s grant candidate instead of deferring to NyxBot", (kind, legend, hint) => {
  render(<MachineGrantPicker kind={kind} value={[]} onChange={vi.fn()} />);
  const group = screen.getByRole("group", { name: legend });
  expect(group).toHaveTextContent(`None available. ${hint}`);
  // A specialist you talk to directly reports missing access to you; nothing here says NyxBot steps in.
  expect(group).not.toHaveTextContent(/NyxBot/);
});

it("still lists an unavailable granted id so the owner can remove it", () => {
  render(<MachineGrantPicker kind="machines" value={["node-gone"]} onChange={vi.fn()} />);
  expect(screen.getByRole("checkbox", { name: "node-gone (unavailable)" })).toBeChecked();
  expect(screen.queryByText(/None available/)).not.toBeInTheDocument();
});
