import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { MachineCapabilities, MachineCapabilityForm } from "./machine-capabilities";
import type { MachineAccess } from "@/schemas/machine-access";
const { save, contextSave, feature, query } = vi.hoisted(() => ({ save: vi.fn(), contextSave: vi.fn(), feature: vi.fn(), query: vi.fn() }));
vi.mock("@/hooks/use-feature-flag", () => ({ useFeature: feature }));
vi.mock("@/hooks/use-machine-access", () => ({ useMachineAccess: query, useSetMachineAccess: () => ({ mutateAsync: save, isPending: false }), useRequestMachineContext: () => ({ mutateAsync: contextSave, isPending: false, isSuccess: false }) }));
const off = { shell: false, files: false, browser: false, computer: false, developer_browser: false };
const machine: MachineAccess = { node_id: "node", name: "Work Mac", revision: 3, protocol_v2: true, can_edit: true, capabilities: off, ceiling: { shell: true, files: true, browser: true, computer: true, developer_browser: true }, legacy: false, saved_login_ids: null, separated: { available: true, landlock_abi: 6, reason: null } };
beforeEach(() => { vi.clearAllMocks(); feature.mockReturnValue(true); query.mockReturnValue({ data: [machine] }); save.mockResolvedValue([]); contextSave.mockResolvedValue({ status: "pending" }); });
describe("machine capability editor", () => {
  it("keeps configured headless shared work usable and separates setup diagnostics", () => {
    render(<MachineCapabilityForm machine={{ ...machine, mode: "shared_legacy", legacy: true,
      capabilities: { ...off, shell: true, files: true }, ceiling: { ...off, shell: true, files: true },
      separated: { available: false, landlock_abi: 8, reason: "separated_requires_managed_browser" },
    }} agentId="agent" contextEnabled />);
    expect(screen.getByText(/run in shared mode, as configured, even when separation is unavailable/)).toBeInTheDocument();
    expect(screen.getByRole("checkbox", { name: "Shell commands" })).toBeEnabled();
    expect(screen.getByRole("checkbox", { name: /Secure browser/ })).toBeDisabled();
    expect(screen.queryByRole("button", { name: "Separate workspace and browser for this agent" })).not.toBeInTheDocument();
    expect(screen.getByText(/Browser permissions are not required/)).toBeInTheDocument();
    expect(screen.getByText("separated_requires_managed_browser").closest("details")).not.toHaveAttribute("open");
  });
  it("refuses unavailable separated work and shows the owner recovery path with the flag off", () => {
    render(<MachineCapabilityForm machine={{ ...machine, mode: "separated", separated: { available: false, landlock_abi: 8, reason: "separated_requires_managed_browser" } }} agentId="agent" />);
    expect(screen.getByText(/Execution is refused.*owner action card/)).toBeInTheDocument();
    expect(screen.queryByText(/run in shared mode, as configured/)).not.toBeInTheDocument();
    expect(screen.getByText(/To enable separate workspaces and browsers/)).toBeInTheDocument();
  });
  it("renders server mode diagnostics from the additive API fields", () => {
    render(<MachineCapabilityForm machine={{ ...machine, execution_note: "Granted commands and file tools run in shared mode, as configured.", separated: null, separated_setup_note: "Restore the managed installation before requesting separated mode." }} agentId="agent" />);
    expect(screen.getByText("Granted commands and file tools run in shared mode, as configured.")).toBeInTheDocument();
    expect(screen.getByText("Restore the managed installation before requesting separated mode.")).toBeInTheDocument();
  });
  it.each([
    [undefined, "Separation support unknown"],
    [null, "Separation support unknown"],
    [{ available: false, landlock_abi: null, reason: "separated_capabilities_not_reported" }, "Separation support unknown"],
    [{ available: false, landlock_abi: 8, reason: "separated_requires_managed_browser" }, "Separation prerequisites not met"],
    [{ available: false, landlock_abi: 8, reason: "separated_probe_timeout" }, "Separation probe timed out"],
    [{ available: true, landlock_abi: 8, reason: "future_reason" }, "Separation support unknown"],
    [{ available: true, landlock_abi: 8, reason: null }, "Separation support confirmed"],
  ] as const)("shows support separately from an existing legacy assignment (%j)", (separated, label) => {
    render(<MachineCapabilityForm machine={{ ...machine, mode: "shared_legacy", separated }} agentId="agent" contextEnabled />);
    expect(screen.getByText(label)).toBeVisible();
    expect(screen.getByText("Shared legacy")).toBeVisible();
    expect(screen.getByText(/Granted commands and file tools run in shared mode/)).toBeVisible();
    if (!separated?.available || separated.reason) expect(screen.queryByRole("button", { name: "Separate workspace and browser for this agent" })).not.toBeInTheDocument();
  });
  it("labels separated contexts without promising full isolation", () => {
    render(<MachineCapabilityForm machine={{ ...machine, mode: "separated", saved_login_ids: [] }} agentId="agent" />);
    expect(screen.getByText("Separate workspace and browser")).toBeInTheDocument();
    expect(screen.getByText(/separate machine container or VM.*containers still share their host kernel/)).toBeInTheDocument();
    expect(screen.queryByText("Shared legacy")).not.toBeInTheDocument();
  });
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
    expect(screen.getByText(/Earlier commands are stopping/)).toBeInTheDocument();
    expect(screen.queryByText(/Online revocation is immediate/)).not.toBeInTheDocument();
  });
  it("shows neutral status while the recovery time is being checked", () => {
    render(<MachineCapabilityForm machine={{ ...machine, revocation_pending: true, revocation_initialization_pending: true }} agentId="agent" />);
    expect(screen.getByText(/Checking when earlier commands will stop/)).toBeInTheDocument();
    expect(screen.queryByText(/Update and reconnect/)).not.toBeInTheDocument();
    expect(screen.queryByText(/They should stop by/)).not.toBeInTheDocument();
  });
  it("shows the recovery time or upgrade action, and a separate recovery error", () => {
    const deadline = "2026-10-16T02:43:26Z";
    const { rerender } = render(<MachineCapabilityForm machine={{ ...machine, revocation_pending: true, revocation_expires_at: deadline }} agentId="agent" />);
    expect(screen.getByText(/They should stop by/)).toHaveTextContent(new Date(deadline).toLocaleString());
    rerender(<MachineCapabilityForm machine={{ ...machine, revocation_pending: true, revocation_diagnostic: "authority_timing_upgrade_required" }} agentId="agent" />);
    expect(screen.getByText(/Update and reconnect this machine to confirm permission changes/)).toBeInTheDocument();
    rerender(<MachineCapabilityForm machine={{ ...machine, revocation_pending: true, revocation_diagnostic: "revocation_deadline_invalid" }} agentId="agent" />);
    expect(screen.getByText(/Recovery status needs administrator attention/)).toBeInTheDocument();
  });
  it("distinguishes execution expiry from undelivered fences and quarantine", () => {
    const { rerender } = render(<MachineCapabilityForm machine={{ ...machine, revocation_pending: false, execution_revocation_pending: false, delivery_pending: true }} agentId="agent" />);
    expect(screen.getByText(/Earlier commands have stopped/)).toBeInTheDocument();
    rerender(<MachineCapabilityForm machine={{ ...machine, revocation_pending: true, execution_revocation_pending: false, delivery_pending: true, quarantine_pending: true }} agentId="agent" />);
    expect(screen.getByText(/Browser cleanup is pending/)).toBeInTheDocument();
    expect(screen.queryByText(/Earlier commands are stopping/)).not.toBeInTheDocument();
  });
  it("refreshes status explicitly without a capability write", () => {
    const refetch = vi.fn();
    query.mockReturnValue({ data: [machine], refetch });
    render(<MachineCapabilities agentId="agent" />);
    fireEvent.click(screen.getByRole("button", { name: "Refresh machine status" }));
    expect(refetch).toHaveBeenCalledOnce();
    expect(save).not.toHaveBeenCalled();
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
  it("requests separated mode through the owner card with an explicit login list", async () => {
    render(<MachineCapabilities agentId="agent" />);
    fireEvent.click(screen.getByRole("button", { name: "Separate workspace and browser for this agent" }));
    await waitFor(() => expect(contextSave).toHaveBeenCalledWith({
      node: "node",
      selection: { mode: "separated", expected_revision: 3, capabilities: off, saved_login_ids: [] },
    }));
    expect(screen.getByText("Owner approval requested in the NyxBot Assistant thread.")).toBeInTheDocument();
  });
});

it("keeps missing evidence actionable with flags off and requires affirmative legacy consent", async () => {
  feature.mockReturnValue(false);
  query.mockReturnValue({ data: [{ ...machine, mode: "selection_required", selection_required: true, selection_reason: "Old node: no capability report", separated: null }] });
  render(<MachineCapabilities agentId="agent" />);
  expect(screen.getByText("Owner selection required")).toBeInTheDocument();
  expect(screen.getByText("Old node: no capability report")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Save capabilities" })).toBeDisabled();
  const request = screen.getByRole("button", { name: "Request shared legacy owner approval" });
  expect(request).toBeDisabled();
  fireEvent.click(screen.getByRole("checkbox", { name: /I explicitly choose shared legacy/ }));
  fireEvent.click(request);
  await waitFor(() => expect(contextSave).toHaveBeenCalledWith({ node: "node", selection: { mode: "shared_legacy", expected_revision: 3, capabilities: off, saved_login_ids: [] } }));
  expect(screen.getByText(/may read node tokens, signing secrets and stored credentials/)).toBeInTheDocument();
});

 it("keeps owner selection available with capability editing off and disables it for a read-only view", () => {
   const pending = { ...machine, mode: "selection_required" as const, selection_required: true };
   const view = render(<MachineCapabilityForm machine={pending} agentId="agent" capabilityEditing={false} />);
   const consent = screen.getByRole("checkbox", { name: /I explicitly choose shared legacy/ });
   expect(consent).toBeEnabled();
   fireEvent.click(consent);
   expect(screen.getByRole("button", { name: "Request shared legacy owner approval" })).toBeEnabled();
   expect(screen.getByRole("button", { name: "Save capabilities" })).toBeDisabled();
   view.rerender(<MachineCapabilityForm machine={pending} agentId="agent" capabilityEditing={false} disabled />);
   expect(consent).toBeDisabled();
   expect(screen.getByRole("button", { name: "Request shared legacy owner approval" })).toBeDisabled();
 });
