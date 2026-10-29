import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { MachineCapabilities, MachineCapabilityForm } from "./machine-capabilities";
import type { MachineAccess } from "@/schemas/machine-access";
import { machineAccessFixture, machineContextReviewFixture } from "@/test/machine-access-fixture";
import { FEATURE_FLAG } from "@/lib/feature-flags";
const { save, contextSave, feature, query } = vi.hoisted(() => ({ save: vi.fn(), contextSave: vi.fn(), feature: vi.fn(), query: vi.fn() }));
vi.mock("@/hooks/use-feature-flag", () => ({ useFeature: feature }));
vi.mock("@/hooks/use-machine-access", () => ({ useMachineAccess: query, useSetMachineAccess: () => ({ mutateAsync: save, isPending: false }), usePrepareMachineContext: () => ({ mutateAsync: contextSave, isPending: false }), useCommitMachineContext: () => ({ mutateAsync: vi.fn(), isPending: false }), useMachineContextReview: () => ({ refetch: vi.fn() }) }));
const metadata = vi.hoisted(() => ({ nodes: { data: [] as unknown[], isError: false }, remotes: { data: [] as unknown[], isError: false }, logins: { data: [] as { id: string; label: string; username_hint: string }[], isError: false, isPending: false } }));
vi.mock("@/hooks/use-nodes", () => ({ useNodes: () => metadata.nodes }));
vi.mock("@/hooks/use-machines", () => ({ useRemoteMachines: () => metadata.remotes }));
vi.mock("@/hooks/use-saved-logins", () => ({ useSavedLogins: () => metadata.logins }));
const off = { shell: false, files: false, browser: false, computer: false, developer_browser: false };
const machine: MachineAccess = { ...machineAccessFixture, machine_configuration: null, availability_reason: null, mode: undefined, node_id: "node", name: "Work Mac", revision: 3, protocol_v2: true, can_edit: true, capabilities: off, ceiling: { shell: true, files: true, browser: true, computer: true, developer_browser: true }, legacy: false, saved_login_ids: null, separated: { available: true, landlock_abi: 6, reason: null } };
beforeEach(() => { vi.clearAllMocks(); feature.mockReturnValue(true); query.mockReturnValue({ data: [machine] }); save.mockResolvedValue([]); contextSave.mockResolvedValue(machineContextReviewFixture); });
describe("machine capability editor", () => {
  it("keeps configured headless shared work usable and separates setup diagnostics", () => {
    render(<MachineCapabilityForm machine={{ ...machine, mode: "shared_legacy", legacy: true,
      capabilities: { ...off, shell: true, files: true }, ceiling: { ...off, shell: true, files: true },
      separated: { available: false, landlock_abi: 8, reason: "separated_requires_managed_browser" },
    }} agentId="agent" contextEnabled />);
    expect(screen.getByText(/run in shared mode, as configured, even when separation is unavailable/)).toBeInTheDocument();
    expect(screen.getByRole("checkbox", { name: "Shell commands" })).toBeEnabled();
    expect(screen.getByRole("checkbox", { name: /Secure browser/ })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Separate workspace and browser for this agent" })).toBeDisabled();
    expect(screen.getByText(/Browser permissions are not required/)).toBeInTheDocument();
    expect(screen.getByText("separated_requires_managed_browser").closest("details")).not.toHaveAttribute("open");
  });
  it("refuses unavailable separated work and shows the owner recovery path with the flag off", () => {
    render(<MachineCapabilityForm machine={{ ...machine, mode: "separated", separated: { available: false, landlock_abi: 8, reason: "separated_requires_managed_browser" } }} agentId="agent" />);
    expect(screen.getByText(/Execution is refused.*owner review/)).toBeInTheDocument();
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
    if (!separated?.available || separated.reason) expect(screen.getByRole("button", { name: "Separate workspace and browser for this agent" })).toBeDisabled();
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
    expect(screen.getByText(/Capability editing is not enabled for this account.*Existing restrictions still apply/)).toBeInTheDocument();
    expect(screen.getByRole("checkbox", { name: "File tools" })).toBeDisabled();
  });
  it("explains saved restrictions and pending revocation without promising instant delivery", () => {
    query.mockReturnValue({ data: [{ ...machine, revocation_pending: true }] });
    render(<MachineCapabilities agentId="agent" />);
    expect(screen.getByText(/Existing restrictions still apply when editing is disabled/)).toBeInTheDocument();
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
    expect(screen.getByText(/Their execution authority expires by/)).toHaveTextContent(new Date(deadline).toLocaleString());
    rerender(<MachineCapabilityForm machine={{ ...machine, revocation_pending: true, revocation_diagnostic: "authority_timing_upgrade_required" }} agentId="agent" />);
    expect(screen.getByText(/Update and reconnect this machine to confirm permission changes/)).toBeInTheDocument();
    rerender(<MachineCapabilityForm machine={{ ...machine, revocation_pending: true, revocation_diagnostic: "revocation_deadline_invalid" }} agentId="agent" />);
    expect(screen.getByText(/Recovery status needs administrator attention/)).toBeInTheDocument();
  });
  it("distinguishes execution expiry from undelivered fences and quarantine", () => {
    const { rerender } = render(<MachineCapabilityForm machine={{ ...machine, revocation_pending: false, execution_revocation_pending: false, delivery_pending: true }} agentId="agent" />);
    expect(screen.getByText(/Earlier execution authority has expired/)).toBeInTheDocument();
    expect(screen.getByText(/Waiting for machine acknowledgement/)).toBeInTheDocument();
    expect(screen.queryByText(/Earlier commands have stopped/)).not.toBeInTheDocument();
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
  it("requests separated mode through a stored owner review without a client login list", async () => {
    render(<MachineCapabilities agentId="agent" />);
    fireEvent.click(screen.getByRole("button", { name: "Separate workspace and browser for this agent" }));
    await waitFor(() => expect(contextSave).toHaveBeenCalledWith({
      node: "node",
      body: { mode: "separated", policy_version: "machine-context-mode-v1", acknowledge_separation: true },
    }));
    expect(screen.getByText("Stored review review-1")).toBeInTheDocument();
  });
});

it.each([false, true])("shows saved partial grants with capability editing %s", (editing) => {
  render(<MachineCapabilityForm machine={{ ...machine, mode: "shared_legacy", capabilities: { ...off, files: true } }} agentId="agent" capabilityEditing={editing} />);
  expect(screen.getByText("Saved tools: File tools")).toBeVisible();
  expect(screen.queryByText(/permissions required/i)).not.toBeInTheDocument();
  expect(screen.getByRole("checkbox", { name: "File tools" })).toBeChecked();
  expect(save).not.toHaveBeenCalled();
});

it("treats all-off as an explicit saved policy and allows revoking the last tool", async () => {
  render(<MachineCapabilityForm machine={{ ...machine, mode: "shared_legacy", capabilities: { ...off, files: true }, saved_login_ids: ["login-1"] }} agentId="agent" />);
  fireEvent.click(screen.getByRole("checkbox", { name: "File tools" }));
  fireEvent.click(screen.getByRole("button", { name: "Save capabilities" }));
  await waitFor(() => expect(save).toHaveBeenCalledWith({ node: "node", selection: { expected_revision: 3, capabilities: off, saved_login_ids: ["login-1"] } }));
  expect(contextSave).not.toHaveBeenCalled();
  expect(screen.getByText(/Capabilities saved.*does not confirm machine acknowledgement/)).toBeVisible();
  expect(screen.queryByText(/Environment applied/i)).not.toBeInTheDocument();
});

it.each([[false, false], [false, true], [true, false], [true, true]])("Details keeps capability editing %s independent from context opt-in %s", (editing, contexts) => {
  feature.mockImplementation((flag: string) => flag === FEATURE_FLAG.MACHINE_CONTEXTS ? contexts : editing);
  render(<MachineCapabilities agentId="agent" />);
  const shell = screen.getByRole("checkbox", { name: "Shell commands" });
  if (editing) expect(shell).toBeEnabled();
  else expect(shell).toBeDisabled();
  const request = screen.getByRole("button", { name: "Separate workspace and browser for this agent" });
  if (contexts) expect(request).toBeEnabled();
  else expect(request).toBeDisabled();
});

it("explains all capability gates and browser dependencies in accessible descriptions", () => {
  render(<MachineCapabilityForm machine={{ ...machine, mode: "selection_required", protocol_v2: false, can_edit: false }} agentId="agent" capabilityEditing={false} />);
  expect(screen.getByText(/Capability editing is not enabled for this account/)).toBeVisible();
  expect(screen.getByText(/Choose an environment mode before editing capabilities/)).toBeVisible();
  expect(screen.getByText(/Only the machine owner/)).toBeVisible();
  expect(screen.getByText(/Update this machine before changing/)).toBeVisible();
  expect(screen.getByRole("checkbox", { name: "File tools" })).toHaveAccessibleDescription(/Capability editing is not enabled.*Choose an environment mode.*Only the machine owner.*Update this machine/s);
});

it("explains the browser prerequisite without granting it automatically", () => {
  render(<MachineCapabilityForm machine={machine} agentId="agent" />);
  expect(screen.getByRole("checkbox", { name: "Full computer control" })).toHaveAccessibleDescription(/Enable Secure browser for this agent first/);
  expect(screen.getByRole("checkbox", { name: "Developer browser" })).toHaveAccessibleDescription(/Enable Secure browser for this agent first/);
  expect(screen.getByRole("checkbox", { name: "Secure browser" })).not.toBeChecked();
});

it("does not infer authority ACK or hosted configuration evidence from a settled read", () => {
  render(<MachineCapabilityForm machine={{ ...machine, mode: "separated", revocation_pending: false, delivery_pending: false, quarantine_pending: false }} agentId="agent" />);
  expect(screen.getByText(/No pending Agent authority changes reported/)).toBeVisible();
  expect(screen.getByText(/Saved permissions do not confirm runtime availability/)).toBeVisible();
  expect(screen.queryByText("Agent authorization applied")).not.toBeInTheDocument();
  expect(screen.queryByText("Environment mode applied")).not.toBeInTheDocument();
});

it("keeps quarantine independent from execution expiry and delivery", () => {
  render(<MachineCapabilityForm machine={{ ...machine, execution_revocation_pending: false, revocation_pending: false, delivery_pending: false, quarantine_pending: true }} agentId="agent" />);
  expect(screen.getByText(/Browser cleanup is pending/)).toBeVisible();
  expect(screen.queryByText(/No pending Agent authority changes/)).not.toBeInTheDocument();
  expect(screen.queryByText(/Earlier commands have stopped/)).not.toBeInTheDocument();
});

it("keeps a stale read visible for diagnosis while refusing all writes", () => {
  query.mockReturnValue({ data: [machine], error: new Error("access denied") });
  render(<MachineCapabilities agentId="agent" />);
  expect(screen.getByRole("alert")).toHaveTextContent(/Could not refresh/);
  expect(screen.getByRole("checkbox", { name: "File tools" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Separate workspace and browser for this agent" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Save capabilities" })).toBeDisabled();
});

it("does not reuse shared consent or request feedback after an authorization revision changes", async () => {
  query.mockReturnValue({ data: [{ ...machine, mode: "selection_required" }] });
  const view = render(<MachineCapabilities agentId="agent" />);
  fireEvent.click(screen.getByRole("checkbox", { name: /I explicitly choose shared legacy/ }));
  fireEvent.click(screen.getByRole("button", { name: "Review shared legacy mode" }));
  await screen.findByText("Stored review review-1");
  expect(screen.getByRole("button", { name: "Approve and apply reviewed mode" })).toBeDisabled();
  query.mockReturnValue({ data: [{ ...machine, mode: "selection_required", revision: 4 }] });
  view.rerender(<MachineCapabilities agentId="agent" />);
  expect(screen.getByRole("checkbox", { name: /I explicitly choose shared legacy/ })).not.toBeChecked();
  expect(screen.getByRole("button", { name: "Review shared legacy mode" })).toBeDisabled();
  expect(screen.queryByText("Owner approval requested in the NyxBot Assistant thread.")).not.toBeInTheDocument();
});

it("shows the Agent membership alongside a saved partial grant in Details", () => {
  query.mockReturnValue({ data: [{ ...machine, mode: "shared_legacy", capabilities: { ...off, files: true } }] });
  render(<MachineCapabilities agentId="agent" agent={{ kind: "specialist", machines: ["node"] }} />);
  expect(screen.getByText("Assigned")).toBeVisible();
  expect(screen.getByText("Saved tools: File tools")).toBeVisible();
  expect(save).not.toHaveBeenCalled();
});

it.each([off, { ...off, files: true }])("requests a mode using the saved tools and logins without a capability write (%j)", async (capabilities) => {
  render(<MachineCapabilityForm machine={{ ...machine, mode: "shared_legacy", capabilities, saved_login_ids: ["login-1"] }} agentId="agent" contextEnabled />);
  fireEvent.click(screen.getByRole("button", { name: "Separate workspace and browser for this agent" }));
  await waitFor(() => expect(contextSave).toHaveBeenCalledWith({ node: "node", body: { mode: "separated", policy_version: "machine-context-mode-v1", acknowledge_separation: true } }));
  expect(save).not.toHaveBeenCalled();
});

it("keeps missing evidence actionable with flags off and requires affirmative legacy consent", async () => {
  feature.mockReturnValue(false);
  query.mockReturnValue({ data: [{ ...machine, mode: "selection_required", selection_required: true, selection_reason: "Old node: no capability report", separated: null }] });
  render(<MachineCapabilities agentId="agent" />);
  expect(screen.getByText("Owner selection required")).toBeInTheDocument();
  expect(screen.getByText("Old node: no capability report")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Save capabilities" })).toBeDisabled();
  const request = screen.getByRole("button", { name: "Review shared legacy mode" });
  expect(request).toBeDisabled();
  fireEvent.click(screen.getByRole("checkbox", { name: /I explicitly choose shared legacy/ }));
  fireEvent.click(request);
  await waitFor(() => expect(contextSave).toHaveBeenCalledWith({ node: "node", body: { mode: "shared_legacy", policy_version: "machine-context-mode-v1", acknowledge_separation: true } }));
  expect(screen.getByText(/may read node tokens, signing secrets and stored credentials/)).toBeInTheDocument();
});

 it("keeps owner selection available with capability editing off and disables it for a read-only view", () => {
   const pending = { ...machine, mode: "selection_required" as const, selection_required: true };
   const view = render(<MachineCapabilityForm machine={pending} agentId="agent" capabilityEditing={false} />);
   const consent = screen.getByRole("checkbox", { name: /I explicitly choose shared legacy/ });
   expect(consent).toBeEnabled();
   fireEvent.click(consent);
   expect(screen.getByRole("button", { name: "Review shared legacy mode" })).toBeEnabled();
   expect(screen.getByRole("button", { name: "Save capabilities" })).toBeDisabled();
   view.rerender(<MachineCapabilityForm machine={pending} agentId="agent" capabilityEditing={false} disabled />);
   expect(consent).toBeDisabled();
   expect(screen.getByRole("button", { name: "Review shared legacy mode" })).toBeDisabled();
 });

it.each([null, [], ["missing", "login-a"]] as const)("preserves Machine login scope %j when saving unrelated capabilities", async (ids) => {
  render(<MachineCapabilityForm machine={{ ...machine, mode: "separated", capabilities: { ...off, browser: true }, saved_login_ids: ids === null ? null : [...ids] }} agentId="agent" availableLogins={[{ id: "login-a", label: "Account A", username_hint: "a", eligible: true }, { id: "login-b", label: "Account B", username_hint: "b", eligible: false }]} />);
  expect(screen.getByRole("checkbox", { name: "Allow Account B" })).toBeDisabled();
  fireEvent.click(screen.getByRole("checkbox", { name: "File tools" }));
  fireEvent.click(screen.getByRole("button", { name: "Save capabilities" }));
  await waitFor(() => expect(save).toHaveBeenCalledWith(expect.objectContaining({ selection: expect.objectContaining({ saved_login_ids: ids }) })));
});
it("retains explicit login IDs and capability edits when login metadata fails", async () => {
  render(<MachineCapabilityForm machine={{ ...machine, mode: "shared_legacy", capabilities: { ...off, browser: true }, saved_login_ids: ["login-a", "unlisted"] }} agentId="agent" sharedLoginsAllowed availableLogins={[{ id: "login-a", label: "Account A", username_hint: "a", eligible: true }]} savedLoginsState="error" />);
  expect(screen.getByRole("checkbox", { name: "Allow Account A" })).toBeChecked();
  expect(screen.getByRole("checkbox", { name: "Allow Account A" })).toBeDisabled();
  expect(screen.getByText(/Saved logins could not be refreshed/)).toBeVisible();
  fireEvent.click(screen.getByRole("checkbox", { name: "File tools" }));
  fireEvent.click(screen.getByRole("button", { name: "Save capabilities" }));
  await waitFor(() => expect(save).toHaveBeenCalledWith(expect.objectContaining({ selection: expect.objectContaining({ saved_login_ids: ["login-a", "unlisted"] }) })));
});
it("loads per-node Shared policy and account eligibility in Agent Grants", () => {
  metadata.nodes.data = [{ id: "node", allow_single_user_saved_logins: false, metadata: null }];
  metadata.logins.data = [{ id: "login-a", label: "Account A", username_hint: "a" }, { id: "login-b", label: "Account B", username_hint: "b" }];
  query.mockReturnValue({ data: [{ ...machine, mode: "shared_legacy", capabilities: { ...off, browser: true }, saved_login_ids: null }] });
  const view = render(<MachineCapabilities agentId="agent" agent={{ kind: "specialist", machines: ["node"], logins: ["login-a"] }} />);
  expect(screen.getByRole("checkbox", { name: "Allow Account A" })).toBeDisabled();
  expect(screen.queryByText(/Hosted configuration evidence/)).not.toBeInTheDocument();
  metadata.nodes.data = [{ id: "node", allow_single_user_saved_logins: true, metadata: null }];
  view.rerender(<MachineCapabilities agentId="agent" agent={{ kind: "specialist", machines: ["node"], logins: ["login-a"] }} />);
  expect(screen.getByRole("checkbox", { name: "Allow Account A" })).toBeEnabled();
  expect(screen.getByRole("checkbox", { name: "Allow Account B" })).toBeDisabled();
  metadata.nodes.data = [];
  metadata.logins.data = [];
});
