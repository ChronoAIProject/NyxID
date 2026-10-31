import { useState } from "react";
import { MachineActivity } from "@/components/shared/machine-activity";
import {
  RemoteMachineTimeline,
  RemoteMachineOperationCard,
} from "./remote-machine-detail";
import { useRemoteMachineOperation } from "@/hooks/use-machines";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from "@/components/ui/dialog";
export function RemoteMachineActivity({
  machineId,
  initialTab = "tools",
}: {
  readonly machineId: string;
  readonly initialTab?: "tools" | "events";
}) {
  const [tab, setTab] = useState(initialTab);
  const [linkedTab, setLinkedTab] = useState(initialTab);
  if (linkedTab !== initialTab) {
    setLinkedTab(initialTab);
    setTab(initialTab);
  }
  const [operationId, setOperationId] = useState<string>();
  const operation = useRemoteMachineOperation(operationId);
  return (
    <div className="min-h-0 min-w-0 flex-1 overflow-y-auto px-5 pb-5">
      <Tabs value={tab} onValueChange={(v) => setTab(v as typeof tab)}>
        <TabsList className="w-full justify-start">
          <TabsTrigger value="tools">Tool activity</TabsTrigger>
          <TabsTrigger value="events">Machine events</TabsTrigger>
        </TabsList>
        <TabsContent
          value="tools"
          forceMount
          className="data-[state=inactive]:hidden"
        >
          <MachineActivity nodeId={machineId} />
        </TabsContent>
        <TabsContent
          value="events"
          forceMount
          className="mt-4 data-[state=inactive]:hidden"
        >
          <RemoteMachineTimeline
            machineId={machineId}
            onOperation={setOperationId}
          />
        </TabsContent>
      </Tabs>
      {operationId && (
        <Dialog
          open
          onOpenChange={(v) => {
            if (!v) setOperationId(undefined);
          }}
        >
          <DialogContent scrollMode="body">
            <DialogHeader>
              <DialogTitle>Historical operation</DialogTitle>
              <DialogDescription>
                Read-only lifecycle history.
              </DialogDescription>
            </DialogHeader>
            {operation.error ? (
              <p role="alert">Operation history unavailable.</p>
            ) : operation.data?.machine_id === machineId &&
              operation.data.id === operationId ? (
              <RemoteMachineOperationCard operation={operation.data} />
            ) : (
              <p role="status">Loading operation…</p>
            )}
          </DialogContent>
        </Dialog>
      )}
    </div>
  );
}
