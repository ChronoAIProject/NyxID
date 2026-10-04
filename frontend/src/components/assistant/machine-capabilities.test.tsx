import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { MachineCapabilities, MachineCapabilityForm } from "./machine-capabilities";
import type { MachineAccess } from "@/schemas/machine-access";
const { save, feature, query } = vi.hoisted(() => ({ save: vi.fn(), feature: vi.fn(), query: vi.fn() }));
vi.mock("@/hooks/use-feature-flag", () => ({ useFeature: feature }));
vi.mock("@/hooks/use-machine-access", () => ({ useMachineAccess: query, useSetMachineAccess: () => ({ mutateAsync: save, isPending: false }) }));
const off = { shell: false, files: false, browser: false, computer: false, developer_browser: false };
const machine: MachineAccess = { node_id: "node", name: "Work Mac", revision: 3, protocol_v2: true, can_edit: true, capabilities: off, ceiling: { shell: true, files: true, browser: true, computer: true, developer_browser: true }, legacy: false, saved_login_ids: null };
beforeEach(() => { vi.clearAllMocks(); feature.mockReturnValue(true); query.mockReturnValue({ data: [machine] }); save.mockResolvedValue([]); });
describe("machine capability editor", () => {
  it("starts denied, uses names and requires an explicit dirty change", async () => {
    render(<MachineCapabilityForm machine={machine} agentId="agent" />);
    expect(screen.getByText("Work Mac")).toBeInTheDocument();
    expect(screen.getByText("Shared legacy")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Save capabilities" })).toBeDisabled();
    for (const box of screen.getAllByRole("checkbox")) expect(box).not.toBeChecked();
    fireEvent.click(screen.getByRole("checkbox", { name: "File tools" }));
    fireEvent.click(screen.getByRole("button", { name: "Save capabilities" }));
    await waitFor(() => expect(save).toHaveBeenCalledWith({ node: "node", selection: { expected_revision: 3, capabilities: { ...off, files: true }, saved_login_ids: null } }));
  });
  it("computer and developer browser require the browser grant", () => {
    render(<MachineCapabilityForm machine={machine} agentId="agent" />);
    expect(screen.getByRole("checkbox", { name: "Full computer control" })).toBeDisabled();
    fireEvent.click(screen.getByRole("checkbox", { name: "Secure browser" }));
    fireEvent.click(screen.getByRole("checkbox", { name: "Developer browser" }));
    expect(screen.getByRole("checkbox", { name: "Developer browser" })).toBeChecked();
    fireEvent.click(screen.getByRole("checkbox", { name: "Secure browser" }));
    expect(screen.getByRole("checkbox", { name: "Developer browser" })).not.toBeChecked();
  });
  it("old nodes preserve their visible legacy grant and require upgrade", () => {
    render(<MachineCapabilityForm machine={{ ...machine, protocol_v2: false, legacy: true, capabilities: { ...off, shell: true } }} agentId="agent" />);
    expect(screen.getByText(/Update this machine before changing/)).toBeInTheDocument();
    expect(screen.getByRole("checkbox", { name: "Shell commands" })).toBeChecked();
    expect(screen.getByRole("checkbox", { name: "Shell commands" })).toBeDisabled();
  });
  it("flag off keeps effective permissions visible but disables writes", () => {
    feature.mockReturnValue(false);
    render(<MachineCapabilities agentId="agent" />);
    expect(feature).toHaveBeenCalledWith("assistant:machine-capabilities");
    expect(screen.getByText(/Existing restrictions still apply/)).toBeInTheDocument();
    expect(screen.getByRole("checkbox", { name: "File tools" })).toBeDisabled();
  });
  it("explains legacy defaults and the offline grace without promising instant delivery", () => {
    query.mockReturnValue({ data: [{ ...machine, revocation_pending: true }] });
    render(<MachineCapabilities agentId="agent" />);
    expect(screen.getByText(/Existing legacy access stays until edited/)).toBeInTheDocument();
    expect(screen.getByText(/leased work stops within 45 seconds/)).toBeInTheDocument();
  });
  it("members can see permissions but cannot edit the machine owner's grants", () => {
    render(<MachineCapabilityForm machine={{ ...machine, can_edit: false }} agentId="agent" />);
    expect(screen.getByText(/Only the machine owner/)).toBeInTheDocument();
    for (const box of screen.getAllByRole("checkbox")) expect(box).toBeDisabled();
    expect(screen.getByRole("button", { name: "Save capabilities" })).toBeDisabled();
  });
  it("local ceilings and failures stay visible", async () => {
    save.mockRejectedValue(new Error("conflict"));
    render(<MachineCapabilityForm machine={{ ...machine, ceiling: { ...machine.ceiling, shell: false } }} agentId="agent" />);
    expect(screen.getByRole("checkbox", { name: /Shell commands/ })).toBeDisabled();
    fireEvent.click(screen.getByRole("checkbox", { name: "File tools" }));
    fireEvent.click(screen.getByRole("button", { name: "Save capabilities" }));
    await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent("Reload if access changed"));
  });
});
