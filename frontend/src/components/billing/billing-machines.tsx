import { RemoteMachineServiceUsage } from "@/components/machines/remote-machine-service-usage";
import { useState } from "react";
import { Link } from "@tanstack/react-router";
import { z } from "zod";
import { zodResolver } from "@hookform/resolvers/zod";
import { useRemoteMachine, useRemoteMachines } from "@/hooks/use-machines";
import { RemoteMachineBilling } from "@/components/machines/remote-machine-billing";
import { DetailSection } from "@/components/shared/detail-section";
import { ErrorBanner } from "@/components/shared/error-banner";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
  useAppForm,
} from "@/components/ui/form";
import { useAuthStore } from "@/stores/auth-store";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

const retainedMachineLookupSchema = z.object({
  machineId: z.string().trim().uuid("Enter the retained Machine ID as a UUID."),
});

export function BillingMachines() {
  const [open, setOpen] = useState(false);
  const ownerId = useAuthStore((state) => state.user?.id);
  return (
    <DetailSection title="Machine runtime">
      <div className="space-y-4 px-4 py-3">
        <p className="text-12 text-muted-foreground">
          Review an accepted runtime rate, funded intervals and pending
          responsibility separately from gateway service usage. Paused workspace
          storage is free. Adding account funds never resumes a machine
          automatically.
        </p>
        <p className="text-12 text-muted-foreground">
          New Machine fixed profile: 2 vCPU, 2 GiB memory and 20 GiB /workspace.
          Each eligible personal account has one free slot
          with no expiry or monthly runtime limit. Paused Machines and
          unfinished cleanup still occupy that slot; additional Machines require
          paid review. Retained Machines keep their historical specifications.
        </p>
        <p className="text-12 text-muted-foreground">
          Temporary storage is managed by the deployment environment, with no
          fixed capacity commitment, selectable size or extra storage charge.
          Temporary data may be lost after pause or Pod recreation. Full disks
          can cause writes to fail, and disk pressure can evict the Pod. Review
          Machine details for storage observations and recovery guidance.
        </p>
        <p className="text-12 text-muted-foreground">
          Paid runtime has no per-instance spending cap. Machines and services
          share account allowances, eligible grants and wallet funds; an
          estimate is not a reserved duration. Resume is manual and requires a
          new accepted quote after earlier liability is resolved. Service usage
          is charged separately, including on a free Machine.
        </p>
        <p className="text-12 text-muted-foreground">
          Paused workspaces are retained indefinitely without a storage fee.
          Deletion is irreversible, with no snapshots or restore. Workspace and
          volume cleanup targets 24 hours and remains pending until stop,
          cleanup and accounting evidence is verified. A retained tombstone and
          necessary billing evidence remain; the free slot is released only
          after finalization. Job metadata lasts 30 days and terminal operation
          history 90 days; live output lasts at most one hour after completion
          and may disappear sooner.
        </p>
        <Button aria-expanded={open} onClick={() => setOpen((value) => !value)}>
          {open ? "Hide machine runtime" : "Review machine runtime"}
        </Button>
        {open ? <MachineRuntimeSelection key={ownerId} /> : null}
      </div>
    </DetailSection>
  );
}

function MachineRuntimeSelection() {
  const machines = useRemoteMachines(true, true);
  const [selectedId, setSelectedId] = useState<string>();
  const [lookupId, setLookupId] = useState<string>();
  const lookup = useRemoteMachine(lookupId);
  const form = useAppForm<z.infer<typeof retainedMachineLookupSchema>>({
    resolver: zodResolver(retainedMachineLookupSchema),
    defaultValues: { machineId: "" },
  });
  const mismatch = Boolean(
    lookupId && lookup.data && lookup.data.node_id !== lookupId,
  );
  const selected = lookupId
    ? !lookup.error && !mismatch
      ? lookup.data
      : undefined
    : !machines.error
      ? machines.data?.find((machine) => machine.node_id === selectedId)
      : undefined;
  return (
    <div className="space-y-4">
      {machines.isPending && (
        <p role="status" className="text-12">
          Loading your remote machines…
        </p>
      )}
      {machines.error && (
        <ErrorBanner
          message="Remote machine management is unavailable. No runtime balance or accepted rate can be inferred from this result."
          onRetry={() => void machines.refetch()}
        />
      )}
      {!machines.isPending && !machines.error && !machines.data?.length ? (
        <p className="text-12 text-muted-foreground">
          No active or retained Machines were returned. Use a known Machine ID
          below to look up an owner-authorized retained billing record.
        </p>
      ) : null}
      {!machines.isPending &&
        !machines.error &&
        !machines.discoveryComplete && (
          <p role="status" className="text-12 text-muted-foreground">
            This server did not confirm complete Machine discovery. Known-ID
            lookup remains available.
          </p>
        )}
      {!machines.error && Boolean(machines.data?.length) && (
        <Select
          value={lookupId ? "" : (selectedId ?? "")}
          onValueChange={(id) => {
            setSelectedId(id);
            setLookupId(undefined);
          }}
        >
          <SelectTrigger aria-label="Machine runtime billing selection">
            <SelectValue placeholder="Choose a remote machine" />
          </SelectTrigger>
          <SelectContent>
            {machines.data?.map((machine) => (
              <SelectItem key={machine.node_id} value={machine.node_id}>
                {machine.sandbox_name} · {machine.resource_state} ·{" "}
                {machine.node_id}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      )}
      <Form {...form}>
        <form
          className="space-y-3"
          onSubmit={form.handleSubmit(({ machineId }) => {
            setLookupId(machineId);
            setSelectedId(undefined);
          })}
        >
          <FormField
            control={form.control}
            name="machineId"
            render={({ field }) => (
              <FormItem>
                <FormLabel>
                  Known Machine ID, including a deleted Machine
                </FormLabel>
                <FormControl>
                  <Input
                    {...field}
                    autoComplete="off"
                    placeholder="Machine UUID"
                  />
                </FormControl>
                <FormMessage />
              </FormItem>
            )}
          />
          <p className="text-11 text-muted-foreground">
            The list includes retained deleted Machines. This lookup also reads
            an owner-authorized retained record; an ID does not grant access or
            restore the workspace.
          </p>
          <Button type="submit">Look up retained Machine billing</Button>
        </form>
      </Form>
      {lookupId && lookup.isPending && (
        <p role="status" className="text-12">
          Looking up the retained Machine record…
        </p>
      )}
      {lookupId && (lookup.error || mismatch) && (
        <ErrorBanner
          message="The retained Machine record could not be verified for this account. No billing state or zero balance has been inferred."
          onRetry={() => void lookup.refetch()}
        />
      )}
      {selected ? (
        <>
          {selected.resource_state === "deleted" && (
            <p role="status" className="text-12 text-muted-foreground">
              This Machine is deleted. Retained runtime contracts remain
              available; this view does not restore execution or workspace data.
            </p>
          )}
          <Button asChild variant="link">
            <Link
              to="/assistant/machines"
              search={{ machine: selected.node_id }}
            >
              {selected.resource_state === "deleted"
                ? "View retained Machine details"
                : "Manage this machine"}
            </Link>
          </Button>
          <RemoteMachineBilling
            key={selected.node_id}
            machineId={selected.node_id}
            billingState={selected.billing_state}
          />
          <RemoteMachineServiceUsage machineId={selected.node_id} />
        </>
      ) : null}
    </div>
  );
}
