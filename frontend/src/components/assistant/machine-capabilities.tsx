import { SHARED_LEGACY_WARNING } from "@/schemas/machines";
import { MachineSeparationSupport } from "@/components/shared/machine-isolation";
import type { MachineContextSelection } from "@/schemas/machine-access";
import { useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { useAppForm } from "@/components/ui/form";
import { Checkbox } from "@/components/ui/checkbox";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { useFeature } from "@/hooks/use-feature-flag";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import {
  useMachineAccess,
  useRequestMachineContext,
  useSetMachineAccess,
} from "@/hooks/use-machine-access";
import { machineSelectionSchema, type MachineAccess, type MachineSelection } from "@/schemas/machine-access";
const labels = { shell: "Shell commands", files: "File tools", browser: "Secure browser", computer: "Full computer control", developer_browser: "Developer browser" } as const;

export function MachineCapabilities({ agentId, disabled = false }: { readonly agentId: string; readonly disabled?: boolean }) {
  const enabled = useFeature(FEATURE_FLAG.MACHINE_CAPABILITIES);
  const contextsEnabled = useFeature(FEATURE_FLAG.MACHINE_CONTEXTS);
  const query = useMachineAccess(agentId, !disabled);
  if (!enabled && !contextsEnabled && !query.data?.length) return null;
  return <section aria-label="Machine capabilities" className="space-y-3">
    <p className="text-12 text-muted-foreground">Choose what this agent can do on each machine. With capability editing enabled, newly selected machines start with all permissions off. Existing legacy access stays until edited. Saving changes cancels this agent’s current work on that machine.</p>
    {!enabled ? <p role="status" className="text-12 text-muted-foreground">Capability editing is not enabled yet. Existing restrictions still apply.</p> : null}
    {query.error ? <p role="alert" className="text-12 text-destructive">Could not refresh machine capabilities. Displayed status may be out of date.</p> : null}
    {query.data?.some(machine => machine.revocation_pending || machine.delivery_pending) ? <p className="text-12 text-muted-foreground">Status refreshes for up to one minute while this page is active. Use Refresh machine status for a later check.</p> : null}
    <Button type="button" size="sm" variant="secondary" disabled={query.isFetching || disabled} onClick={() => void query.refetch()}>Refresh machine status</Button>
    {query.data?.map(machine => <MachineCapabilityForm key={`${machine.node_id}:${machine.revision}`} machine={machine} agentId={agentId} disabled={disabled} capabilityEditing={enabled} contextEnabled={contextsEnabled} />)}
  </section>;
}
export function MachineCapabilityForm({ machine, agentId, disabled = false, capabilityEditing = true, contextEnabled = false }: { readonly machine: MachineAccess; readonly agentId: string; readonly disabled?: boolean; readonly capabilityEditing?: boolean; readonly contextEnabled?: boolean }) {
  const mutation = useSetMachineAccess(agentId);
  const contextMutation = useRequestMachineContext(agentId);
  const [error, setError] = useState<string>();
  const [contextRequested, setContextRequested] = useState(false);
  const [legacyAccepted, setLegacyAccepted] = useState(false);
  const needsSelection = machine.selection_required === true || machine.mode === "selection_required";
  const separationAvailable = machine.separated?.available === true && !machine.separated.reason;
  const form = useAppForm<MachineSelection>({ resolver: zodResolver(machineSelectionSchema), defaultValues: {
    expected_revision: machine.revision, capabilities: machine.capabilities, saved_login_ids: machine.saved_login_ids,
  } });
  const caps = form.watch("capabilities");
  const requestContext = async (mode: MachineContextSelection["mode"] = "separated") => {
    setError(undefined);
    try {
      await contextMutation.mutateAsync({
        node: machine.node_id,
        selection: {
          mode,
          expected_revision: machine.revision,
          capabilities: machine.capabilities,
          saved_login_ids: machine.saved_login_ids ?? [],
        },
      });
      setContextRequested(true);
    } catch {
      setError("Could not request this machine selection. Reload if access changed; automatic and required need current confirmed support.");
    }
  };
  return <form className="space-y-3 rounded-lg border p-4" aria-label={`${machine.name} capabilities`} onSubmit={form.handleSubmit(async selection => {
    setError(undefined);
    try { await mutation.mutateAsync({ node: machine.node_id, selection }); form.reset(selection); }
    catch { setError("Could not save capabilities. Reload if access changed."); }
  })}>
    <div className="flex flex-wrap items-center gap-2"><span className="text-13 font-medium">{machine.name}</span><Badge variant="secondary">{needsSelection ? "Owner selection required" : machine.mode === "separated" ? "Separate workspace and browser" : "Shared legacy"}</Badge></div>
    <p className="text-12 text-muted-foreground">{needsSelection ? "Execution is blocked until the owner records a supported choice or explicitly approves legacy access." : machine.mode === "separated" ? "This agent has its own workspace and browser profiles. Use a separate machine container or VM per agent for stronger separation; containers still share their host kernel." : "Workspace and browser sessions are shared with other agents in shared legacy mode on this machine."}</p>
    <p role="status" className="text-12 text-muted-foreground">{machine.execution_note ?? (needsSelection ? "Execution is blocked until the owner completes separation selection." : machine.mode === "separated"
      ? separationAvailable
        ? "Granted commands and file tools use this agent’s separated workspace. No shared fallback."
        : "This assignment requires separated mode, but separation is unavailable. Execution is refused. Restore support or ask the owner to change mode through an owner action card; no automatic shared fallback."
      : "Granted commands and file tools run in shared mode, as configured, even when separation is unavailable. Shell uses the command user’s full OS permissions.")}</p>
    {needsSelection ? <p role="status" className="text-12">{machine.selection_reason ?? "Choose a separation mode. Old nodes without capability evidence require an upgrade for separated access or an informed explicit legacy choice."}</p> : null}
    {machine.can_edit && machine.mode !== "shared_legacy" ? <div className="space-y-2 rounded-lg border p-3 text-12">
      <p>{SHARED_LEGACY_WARNING}</p>
      <label className="flex items-start gap-2"><Checkbox disabled={disabled || contextMutation.isPending} checked={legacyAccepted} onCheckedChange={value => setLegacyAccepted(value === true)} />I explicitly choose shared legacy access and acknowledge this warning.</label>
      <Button type="button" size="sm" variant="secondary" disabled={disabled || !legacyAccepted || contextMutation.isPending} onClick={() => void requestContext("shared_legacy")}>Request shared legacy owner approval</Button>
    </div> : null}
    {contextRequested || contextMutation.isSuccess ? <p role="status">Owner approval requested in the NyxBot Assistant thread.</p> : null}
    <MachineSeparationSupport support={machine.separated} setupNote={machine.separated_setup_note} />
    {machine.quarantine_pending ? <p role="status" className="text-12 text-warning">Browser cleanup is pending. Reconnect the machine and refresh status. New browser work stays blocked until cleanup finishes.</p> : null}
    {machine.revocation_initialization_pending ? <p role="status" className="text-12 text-muted-foreground">Checking when earlier commands will stop. Refresh status in a few seconds.</p> : (machine.execution_revocation_pending ?? machine.revocation_pending) ? <p role="status" className="text-12 text-muted-foreground">Earlier commands are stopping.{machine.revocation_expires_at ? ` They should stop by ${new Date(machine.revocation_expires_at).toLocaleString()}.` : " Reconnect and refresh status to confirm they have stopped."} Refresh status before starting new work.</p> : machine.delivery_pending ? <p role="status" className="text-12 text-muted-foreground">Earlier commands have stopped. Updated permissions still need machine acknowledgement. Reconnect the machine and refresh status.</p> : null}
    {machine.revocation_diagnostic ? <p role="status" className="text-12 text-warning">{machine.revocation_diagnostic === "authority_timing_upgrade_required" ? "Update and reconnect this machine to confirm permission changes." : "Recovery status needs administrator attention. Earlier commands are still marked as stopping; check machine diagnostics before continuing."}</p> : null}
    {!machine.can_edit ? <p role="status" className="text-12">Only the machine owner or an organization admin can edit this access.</p> : null}
    {!machine.protocol_v2 ? <p role="status" className="text-12">Update this machine before changing capabilities. Its existing access continues.</p> : null}
    {contextEnabled && machine.mode !== "separated" && machine.can_edit ? (
      separationAvailable ? (
        <div className="space-y-2 rounded-lg border border-primary/30 bg-primary/5 p-3">
          <p className="text-12">Give this agent a separate workspace and browser through an owner-approved Assistant card.</p>
          <Button type="button" size="sm" variant="secondary" disabled={disabled || contextMutation.isPending} onClick={() => void requestContext("separated")}>
            Separate workspace and browser for this agent
          </Button>
        </div>
      ) : null
    ) : null}
    <fieldset disabled={disabled || !capabilityEditing || needsSelection || !machine.can_edit || !machine.protocol_v2 || mutation.isPending} className="grid gap-2 sm:grid-cols-2">
      <legend className="sr-only">Allowed capabilities</legend>
      {(Object.keys(labels) as Array<keyof typeof labels>).map(key => <label key={key} className="flex items-center gap-2 text-12">
        <Checkbox checked={caps[key]} disabled={!machine.ceiling[key] || ((key === "computer" || key === "developer_browser") && !caps.browser)} onCheckedChange={value => {
          form.setValue(`capabilities.${key}`, value === true);
          if (key === "browser" && value !== true) { form.setValue("capabilities.computer", false); form.setValue("capabilities.developer_browser", false); }
        }} />{labels[key]}{!machine.ceiling[key] ? <span className="text-muted-foreground">(off on node)</span> : null}
      </label>)}
    </fieldset>
    <details className="text-12 text-muted-foreground"><summary className="cursor-pointer">What these permissions mean</summary><p className="mt-2">Shell commands can read and write anything their OS user can access, even with file tools off. Full computer control can operate applications through their UI. Browser-only access avoids desktop authority. Developer browser permits JavaScript evaluation in its separate browser; saved logins stay in the secure browser.</p></details>
    {error ? <p role="alert" className="text-12 text-destructive">{error}</p> : null}
    <Button type="submit" size="sm" disabled={disabled || !capabilityEditing || needsSelection || !machine.can_edit || !machine.protocol_v2 || !form.formState.isDirty || mutation.isPending}>Save capabilities</Button>
  </form>;
}
