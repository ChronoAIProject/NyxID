import { useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { useAppForm } from "@/components/ui/form";
import { Checkbox } from "@/components/ui/checkbox";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { useFeature } from "@/hooks/use-feature-flag";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { useMachineAccess, useSetMachineAccess } from "@/hooks/use-machine-access";
import { machineSelectionSchema, type MachineAccess, type MachineSelection } from "@/schemas/machine-access";
const labels = { shell: "Shell commands", files: "File tools", browser: "Secure browser", computer: "Full computer control", developer_browser: "Developer browser" } as const;

export function MachineCapabilities({ agentId, disabled = false }: { readonly agentId: string; readonly disabled?: boolean }) {
  const enabled = useFeature(FEATURE_FLAG.MACHINE_CAPABILITIES);
  const query = useMachineAccess(agentId, enabled && !disabled);
  if (!enabled && !query.data?.length) return null;
  return <section aria-label="Machine capabilities" className="space-y-3">
    <p className="text-[12px] text-muted-foreground">Choose what this agent can do on each machine. With capability editing enabled, newly selected machines start with all permissions off. Existing legacy access stays until edited. Saving changes cancels this agent’s current work on that machine.</p>
    {!enabled ? <p role="status" className="text-[12px] text-muted-foreground">Capability editing is not enabled yet. Existing restrictions still apply.</p> : null}
    {query.error ? <p role="alert" className="text-[12px] text-destructive">Could not load machine capabilities.</p> : null}
    {query.data?.map(machine => <MachineCapabilityForm key={`${machine.node_id}:${machine.revision}`} machine={machine} agentId={agentId} disabled={disabled || !enabled} />)}
  </section>;
}
export function MachineCapabilityForm({ machine, agentId, disabled = false }: { readonly machine: MachineAccess; readonly agentId: string; readonly disabled?: boolean }) {
  const mutation = useSetMachineAccess(agentId);
  const [error, setError] = useState<string>();
  const form = useAppForm<MachineSelection>({ resolver: zodResolver(machineSelectionSchema), defaultValues: {
    expected_revision: machine.revision, capabilities: machine.capabilities, saved_login_ids: machine.saved_login_ids,
  } });
  const caps = form.watch("capabilities");
  return <form className="space-y-3 rounded-lg border p-4" aria-label={`${machine.name} capabilities`} onSubmit={form.handleSubmit(async selection => {
    setError(undefined);
    try { await mutation.mutateAsync({ node: machine.node_id, selection }); form.reset(selection); }
    catch { setError("Could not save capabilities. Reload if access changed."); }
  })}>
    <div className="flex flex-wrap items-center gap-2"><span className="text-[13px] font-medium">{machine.name}</span><Badge variant="secondary">Shared legacy</Badge></div>
    <p className="text-[12px] text-muted-foreground">Workspace and browser sessions are shared with other agents on this machine.</p>
    {machine.revocation_pending ? <p role="status" className="text-[12px] text-muted-foreground">Revocation pending on machine. Online revocation is immediate. If delivery is interrupted, v2 leased work stops within 45 seconds.</p> : null}
    {!machine.can_edit ? <p role="status" className="text-[12px]">Only the machine owner or an organization admin can edit this access.</p> : null}
    {!machine.protocol_v2 ? <p role="status" className="text-[12px]">Update this machine before changing capabilities. Its existing access continues.</p> : null}
    <fieldset disabled={disabled || !machine.can_edit || !machine.protocol_v2 || mutation.isPending} className="grid gap-2 sm:grid-cols-2">
      <legend className="sr-only">Allowed capabilities</legend>
      {(Object.keys(labels) as Array<keyof typeof labels>).map(key => <label key={key} className="flex items-center gap-2 text-[12px]">
        <Checkbox checked={caps[key]} disabled={!machine.ceiling[key] || ((key === "computer" || key === "developer_browser") && !caps.browser)} onCheckedChange={value => {
          form.setValue(`capabilities.${key}`, value === true);
          if (key === "browser" && value !== true) { form.setValue("capabilities.computer", false); form.setValue("capabilities.developer_browser", false); }
        }} />{labels[key]}{!machine.ceiling[key] ? <span className="text-muted-foreground">(off on node)</span> : null}
      </label>)}
    </fieldset>
    <details className="text-[12px] text-muted-foreground"><summary className="cursor-pointer">What these permissions mean</summary><p className="mt-2">Shell commands can read and write anything their OS user can access, even with file tools off. Full computer control can operate applications through their UI. Browser-only access avoids desktop authority. Developer browser permits JavaScript evaluation in its separate browser; saved logins stay in the secure browser.</p></details>
    {error ? <p role="alert" className="text-[12px] text-destructive">{error}</p> : null}
    <Button type="submit" size="sm" disabled={disabled || !machine.can_edit || !machine.protocol_v2 || !form.formState.isDirty || mutation.isPending}>Save capabilities</Button>
  </form>;
}
