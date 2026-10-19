import { useId, useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { useNodes } from "@/hooks/use-nodes";
import { useUserServices } from "@/hooks/use-user-services";
import { useAuthStore } from "@/stores/auth-store";
import { machineGrantSelectionSchema, type MachineGrantSelection } from "@/schemas/machine-grants";
import { Form, FormSubmitErrors, useAppForm } from "@/components/ui/form";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { Button } from "@/components/ui/button";
import { ErrorBanner } from "@/components/shared/error-banner";

export function MachineGrantEditor({ initial, onSubmit, onSelectionChange, submitting = false, submitLabel = "Save Machine permissions", disabled = false, allowedNodeIds, allowedServiceIds }: {
  readonly initial?: MachineGrantSelection;
  readonly onSubmit: (value: MachineGrantSelection) => Promise<void>;
  readonly onSelectionChange?: () => void;
  readonly submitting?: boolean;
  readonly submitLabel?: string;
  readonly disabled?: boolean;
  readonly allowedNodeIds?: readonly string[];
  readonly allowedServiceIds?: readonly string[];
}) {
  const id = useId();
  const ownerId = useAuthStore((state) => state.user?.id);
  const nodes = useNodes();
  const services = useUserServices();
  const [defaultExpiry] = useState(() => new Date(Date.now() + 30 * 86400000).toISOString());
  const form = useAppForm<MachineGrantSelection>({
    resolver: zodResolver(machineGrantSelectionSchema),
    defaultValues: initial ?? { assignments: {}, expires_at: defaultExpiry },
  });
  const assignments = form.watch("assignments");
  const expiry = form.watch("expires_at");
  const inventory = (nodes.data ?? []).filter((node) => node.owner.kind === "user" && node.owner.id === ownerId && node.machine && (node.machine.shell || node.machine.files) && (!allowedNodeIds || allowedNodeIds.includes(node.id)));
  const availableServices = (services.data ?? []).filter((service) => service.is_active && (service.credential_source.type === "personal" || service.credential_source.allowed) && (!allowedServiceIds || allowedServiceIds.includes(service.id)));
  const unavailable = Object.keys(assignments).filter((nodeId) => !inventory.some((node) => node.id === nodeId));
  function setAssignments(value: MachineGrantSelection["assignments"]) {
    onSelectionChange?.();
    form.setValue("assignments", value);
  }
  function update(nodeId: string, value: MachineGrantSelection["assignments"][string]) {
    setAssignments({ ...assignments, [nodeId]: value });
  }
  const busy = submitting || disabled || nodes.isLoading || Boolean(nodes.error);
  return <Form {...form}>
    <form className="space-y-4" onSubmit={form.handleSubmit(onSubmit)}>
      <p className="text-12 text-muted-foreground">Permissions apply to this real OAuth client or parent Key. All its valid connections share the grant; parent Key login credentials share its task view. This does not authorize paid resource management.</p>
      <div className="space-y-2 rounded-lg border border-warning/40 bg-warning/5 p-3 text-12" role="note" aria-label="Shell access and credentials">
        <p>Shared OS and workspace: file roots constrain file tools and working directories, not what shell commands can access. Clients are not secret-isolated.</p>
        <p>On a self-hosted Node, non-isolated shell can access Node tokens, signing secrets and stored credentials under the command user’s OS permissions. Use the container or deploy with <code>--separate-users</code> to separate command access from Node identity.</p>
        <p>On managed Machines, Node identity protection depends on verified supervisor and command-user separation and protected mounts. This does not isolate clients or their workspace secrets.</p>
        <p>Browser, desktop and saved-login tools are excluded. Excluding these tools does not protect their underlying files from shell access.</p>
      </div>
      {nodes.isLoading && <p role="status">Loading owned Machines…</p>}
      {nodes.error && <ErrorBanner message="Could not load owned Machines." onRetry={() => void nodes.refetch()} />}
      {!nodes.isLoading && !nodes.error && inventory.length === 0 && <p className="text-12 text-muted-foreground">No eligible personal Machines are available. Add a Machine in Assistant → Machines first.</p>}
      {unavailable.map((nodeId) => <div key={nodeId} className="rounded-lg border border-destructive/40 p-3"><p className="text-12">Machine {nodeId} is unavailable or outside the source’s current node ceiling.</p><Button type="button" variant="outline" disabled={busy} onClick={() => { const next = { ...assignments }; delete next[nodeId]; setAssignments(next); }}>Remove unavailable Machine</Button></div>)}
      {inventory.map((node) => {
        const assignment = assignments[node.id];
        return <fieldset key={node.id} className="space-y-3 rounded-lg border border-border p-3" disabled={submitting || disabled}>
          <legend className="px-1 text-13 font-medium">{node.name}</legend>
          <div className="flex items-center gap-2"><Checkbox id={`${id}-${node.id}`} checked={Boolean(assignment)} onCheckedChange={(checked) => {
            if (checked) setAssignments({ ...assignments, [node.id]: { capabilities: { shell: Boolean(node.machine?.shell), files: Boolean(node.machine?.files), browser: false, computer: false, developer_browser: false }, file_roots: node.machine?.roots ?? [], service_ids: [] } });
            else { const next = { ...assignments }; delete next[node.id]; setAssignments(next); }
          }} /><Label htmlFor={`${id}-${node.id}`}>Allow this Machine</Label></div>
          <p className="text-11 text-muted-foreground">{node.id} · Confirmations: {node.machine_confirm ?? "Not reported"}. Connection state does not establish permission.</p>
          {assignment && <>
            <div className="flex flex-wrap gap-4">{(["shell", "files"] as const).map((capability) => <div key={capability} className="flex items-center gap-2"><Checkbox id={`${id}-${node.id}-${capability}`} checked={assignment.capabilities[capability]} disabled={!node.machine?.[capability]} onCheckedChange={(checked) => update(node.id, { ...assignment, capabilities: { ...assignment.capabilities, [capability]: checked === true }, service_ids: capability === "shell" && !checked ? [] : assignment.service_ids })} /><Label htmlFor={`${id}-${node.id}-${capability}`}>{capability === "shell" ? "Commands" : "Files"}</Label></div>)}</div>
            <div className="space-y-1.5"><Label htmlFor={`${id}-${node.id}-roots`}>File roots (one per line)</Label><Textarea id={`${id}-${node.id}-roots`} value={assignment.file_roots.join("\n")} onChange={(event) => update(node.id, { ...assignment, file_roots: event.target.value.split("\n") })} /><p className="text-11 text-muted-foreground">Machine ceiling: {node.machine?.roots.join(", ") || "Not reported"}. The server and Node enforce the intersection.</p></div>
            <details className="space-y-2"><summary className="cursor-pointer text-12 font-medium">Gateway services · {assignment.service_ids.length} selected</summary><p className="text-11 text-muted-foreground">Default none. Each job must also explicitly request its services, within the live OAuth/Key and service permissions. This does not restrict ordinary shell network access.</p>
              {services.isLoading && <p role="status">Loading services…</p>}
              {services.error && <ErrorBanner message="Service choices are unavailable. Existing selections are retained; retry before adding services." onRetry={() => void services.refetch()} />}
              {availableServices.map((service) => <div key={service.id} className="flex items-center gap-2"><Checkbox id={`${id}-${node.id}-svc-${service.id}`} disabled={!assignment.capabilities.shell} checked={assignment.service_ids.includes(service.id)} onCheckedChange={(checked) => update(node.id, { ...assignment, service_ids: checked ? [...assignment.service_ids, service.id] : assignment.service_ids.filter((value) => value !== service.id) })} /><Label htmlFor={`${id}-${node.id}-svc-${service.id}`}>{service.label || service.catalog_service_name || service.slug}</Label></div>)}
              {assignment.service_ids.filter((serviceId) => !availableServices.some((service) => service.id === serviceId)).map((serviceId) => <div key={serviceId} className="flex items-center justify-between gap-2 text-11"><span>Unavailable service: {serviceId}</span><Button type="button" variant="ghost" onClick={() => update(node.id, { ...assignment, service_ids: assignment.service_ids.filter((value) => value !== serviceId) })}>Remove</Button></div>)}
            </details>
          </>}
        </fieldset>;
      })}
      <div className="space-y-1.5"><Label htmlFor={`${id}-expires`}>Grant expires (UTC)</Label><Input id={`${id}-expires`} type="datetime-local" disabled={busy} value={expiry?.slice(0, 16) ?? ""} onChange={(event) => { onSelectionChange?.(); form.setValue("expires_at", event.target.value ? `${event.target.value}:00.000Z` : ""); }} /><p className="text-11 text-muted-foreground">Default 30 days. Expiry stops new work and renewal; it does not delete the Machine or paused workspace. Authentication or funding may end earlier.</p></div>
      <FormSubmitErrors />
      <Button type="submit" disabled={busy || unavailable.length > 0} isLoading={submitting}>{submitLabel}</Button>
    </form>
  </Form>;
}
