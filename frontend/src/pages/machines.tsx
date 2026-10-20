import { useRemoteMachineAdmission } from "@/hooks/use-remote-machine-admission";
import {
  MachineIsolationBadge,
  MachineSharingBadge,
} from "@/components/shared/machine-isolation";
import { useState } from "react";
import { Link, useNavigate, useSearch } from "@tanstack/react-router";
import { Monitor, Settings, History } from "lucide-react";
import { useMachineUpdates, useRemoteMachines } from "@/hooks/use-machines";
import type { MachineUpdateStatus, RemoteMachine } from "@/schemas/machines";
import { RemoteMachineDetail } from "@/components/machines/remote-machine-detail";
import { ErrorBanner } from "@/components/shared/error-banner";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useNodes } from "@/hooks/use-nodes";
import { useOrgs } from "@/hooks/use-orgs";
import { useAuthStore } from "@/stores/auth-store";
import { parseMachinesSearch } from "@/lib/machine-search";
import { SavedLoginsPage } from "@/pages/saved-logins";
import { MachineActivity } from "@/components/shared/machine-activity";
import { MachineSettings } from "@/components/shared/machine-settings";
import { PageHeader } from "@/components/shared/page-header";
import { AddCtaButton } from "@/components/shared/add-cta-button";
import { NodeStatusBadge } from "@/components/shared/node-status-badge";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs";
import {
  Sheet,
  SheetContent,
  SheetHeader,
  SheetTitle,
  SheetDescription,
} from "@/components/ui/sheet";
import type { NodeInfo } from "@/types/nodes";

function MachineCard({
  node,
  onSettings,
  onActivity,
  update,
}: {
  readonly node: NodeInfo;
  readonly onSettings: () => void;
  readonly onActivity: () => void;
  readonly update?: MachineUpdateStatus;
}) {
  const machine = node.machine!;
  return (
    <article className="space-y-3 rounded-xl border border-border/50 bg-card p-4">
      <div className="flex flex-wrap items-center gap-2">
        <Monitor className="h-4 w-4 text-text-tertiary" />
        <h3 className="min-w-0 flex-1 truncate text-13 font-medium">
          {node.name}
        </h3>
        {update?.update_available ? (
          <Badge variant="warning">
            Update available ({update.target_version})
          </Badge>
        ) : null}
        <span className="text-11 text-muted-foreground">
          v{node.metadata?.agent_version ?? "unknown"}
        </span>
        <NodeStatusBadge status={node.status} isConnected={node.is_connected} />
        <MachineIsolationBadge machine={machine} />
        <MachineSharingBadge />
      </div>
      <p className="text-12 text-muted-foreground">
        {[
          machine.shell && "Commands",
          machine.files && "Files",
          (machine.browser ?? machine.computer) && "Browser",
          machine.computer && "Computer",
        ]
          .filter(Boolean)
          .join(" · ")}{" "}
        · {machine.os} / {machine.arch}
      </p>
      <p className="text-12 text-muted-foreground">
        {node.owner.display_name} ·{" "}
        {node.is_connected ? "Connected" : "Disconnected"}
        {machine.computer && !machine.computer_ready
          ? " · Computer permissions or display unavailable"
          : ""}
      </p>
      <div className="flex flex-wrap gap-2">
        {machine.computer || machine.browser ? (
          <Button asChild disabled={!node.is_connected}>
            <Link
              to="/assistant/machines/$nodeId/desktop"
              params={{ nodeId: node.id }}
              disabled={!node.is_connected}
            >
              Watch / Take over desktop
            </Link>
          </Button>
        ) : null}
        <Button onClick={onActivity}>
          <History />
          Activity
        </Button>
        <Button onClick={onSettings} aria-label={`Settings for ${node.name}`}>
          <Settings />
          Settings
        </Button>
      </div>
    </article>
  );
}

export function MachinesPage() {
  const remoteAdmission = useRemoteMachineAdmission();
  const navigate = useNavigate();
  const search = parseMachinesSearch(
    useSearch({ strict: false }) as Record<string, unknown>,
  );
  const nodes = useNodes({ pollIntervalMs: 5000 });
  const updates = useMachineUpdates();
  const remote = useRemoteMachines();
  const orgs = useOrgs();
  const userId = useAuthStore((state) => state.user?.id);
  const [activityId, setActivityId] = useState<string>();
  const [selectedId, setSelectedId] = useState<string | undefined>(
    search.machine,
  );
  const [linkedMachine, setLinkedMachine] = useState(search.machine);
  if (linkedMachine !== search.machine) {
    setLinkedMachine(search.machine);
    setSelectedId(search.machine);
  }
  const adminOrgs = new Set(
    orgs.data?.filter((org) => org.your_role === "admin").map((org) => org.id),
  );
  const hostedMachines = remote.data ?? [];
  const hostedIds = new Set(hostedMachines.map((machine) => machine.node_id));
  const machines = (nodes.data ?? []).filter(
    (node) =>
      node.machine &&
      node.metadata?.provisioning_source !== "remote_machine" &&
      !hostedIds.has(node.id) &&
      (node.owner.id === userId ||
        (node.owner.kind === "org" && adminOrgs.has(node.owner.id))),
  );
  const selected = machines.find((node) => node.id === selectedId);
  const selectedRemote = hostedMachines.find(
    (machine) => machine.node_id === selectedId,
  );
  // Deleted resources leave the active list but retain an owner-readable
  // tombstone. Resolve direct links independently so cleanup stays reviewable.
  const remoteDetailId =
    selectedRemote?.node_id ??
    (selectedId &&
    selectedId === search.machine &&
    !selected &&
    !nodes.isPending
      ? selectedId
      : undefined);
  function openRemote(machineId: string, operationId?: string) {
    setSelectedId(machineId);
    void navigate({
      to: "/assistant/machines",
      search: { machine: machineId, operation: operationId },
    });
  }
  return (
    <div className="space-y-6">
      <Tabs
        value={search.tab ?? "machines"}
        onValueChange={(tab) =>
          void navigate({
            to: "/assistant/machines",
            search: { tab: tab === "logins" ? "logins" : undefined },
          })
        }
      >
        <TabsList>
          <TabsTrigger value="machines">Machines</TabsTrigger>
          <TabsTrigger value="logins">Saved logins</TabsTrigger>
        </TabsList>
        <TabsContent value="machines" className="mt-6 space-y-6">
          <PageHeader
            title="Machines"
            description="Remote and self-hosted computers for authorized agents and clients."
            actions={
              <div className="flex flex-wrap gap-2">
                {remoteAdmission && (
                  <AddCtaButton
                    label="Create remote machine"
                    onClick={() =>
                      void navigate({
                        to: "/assistant/machines/new",
                        search: { where: "hosted" },
                      })
                    }
                  />
                )}
                <AddCtaButton
                  label="Add a machine"
                  onClick={() =>
                    void navigate({ to: "/assistant/machines/new" })
                  }
                />
              </div>
            }
          />
          <section aria-label="Remote machines" className="space-y-3">
            <h3 className="text-15 font-semibold">Remote machines</h3>
            <p className="text-12 text-muted-foreground">
              Personal hosted machines with a fixed workspace and selected
              capabilities. Resource state, runtime billing and execution
              authorization are independent.
            </p>
            {!remote.isPending &&
              !remote.error &&
              !remote.discoveryComplete && (
                <p role="status" className="text-12 text-muted-foreground">
                  Complete remote Machine discovery is not confirmed by this
                  server.
                </p>
              )}
            {remote.isPending ? (
              <p role="status" className="text-12">
                Loading remote machines…
              </p>
            ) : remote.error ? (
              <ErrorBanner
                message="Remote machine management could not be loaded. This deployment may not expose it yet."
                onRetry={() => void remote.refetch()}
              />
            ) : hostedMachines.length ? (
              <RemoteMachineList
                machines={hostedMachines}
                onOpen={openRemote}
              />
            ) : (
              <p className="rounded-lg border border-dashed border-border p-6 text-12 text-muted-foreground">
                No remote machines. Check your free slot and the configured
                profile before creating one.
              </p>
            )}
          </section>
          <h3 className="text-15 font-semibold">Self-hosted machines</h3>
          {nodes.isPending || orgs.isPending ? (
            <p role="status">Loading machines…</p>
          ) : null}
          {nodes.error || orgs.error ? (
            <p role="alert" className="text-12 text-destructive">
              Could not load machines.{" "}
              {nodes.error?.message ?? orgs.error?.message}
            </p>
          ) : null}
          {!nodes.isPending &&
          !orgs.isPending &&
          !nodes.error &&
          !orgs.error &&
          !machines.length ? (
            <p className="rounded-xl border border-border/50 p-6 text-12 text-muted-foreground">
              Add a machine to let NyxBot run commands, work with files and use
              a desktop.
            </p>
          ) : null}
          <div className="grid gap-4 xl:grid-cols-2">
            {machines.map((node) => (
              <MachineCard
                key={node.id}
                node={node}
                update={updates.data?.find((row) => row.node_id === node.id)}
                onSettings={() => setSelectedId(node.id)}
                onActivity={() => setActivityId(node.id)}
              />
            ))}
          </div>
        </TabsContent>
        <TabsContent value="logins" className="mt-6">
          <SavedLoginsPage />
        </TabsContent>
      </Tabs>
      <Sheet
        open={Boolean(remoteDetailId)}
        onOpenChange={(open) => {
          if (!open) {
            setSelectedId(undefined);
            void navigate({ to: "/assistant/machines", search: {} });
          }
        }}
      >
        <SheetContent className="w-full overflow-y-auto sm:max-w-3xl">
          <SheetHeader className="pr-8">
            <SheetTitle>Remote machine details</SheetTitle>
            <SheetDescription>
              Lifecycle, access, work history and billing for this machine.
            </SheetDescription>
          </SheetHeader>
          {remoteDetailId ? (
            <RemoteMachineDetail
              key={`${userId}:${remoteDetailId}:${search.operation ?? ""}:${search.confirmation ?? ""}`}
              operationId={search.operation}
              confirmationId={search.confirmation}
              machineId={remoteDetailId}
              initialMachine={selectedRemote}
              node={
                nodes.error
                  ? undefined
                  : nodes.data?.find(
                      (node) =>
                        node.id === remoteDetailId && node.owner.id === userId,
                    )
              }
            />
          ) : null}
        </SheetContent>
      </Sheet>
      <Sheet
        open={Boolean(activityId)}
        onOpenChange={(open) => {
          if (!open) setActivityId(undefined);
        }}
      >
        <SheetContent className="flex w-full min-w-0 flex-col gap-0 overflow-hidden p-0 sm:max-w-lg">
          <SheetHeader className="shrink-0 p-5 pr-10">
            <SheetTitle>Machine activity</SheetTitle>
            <SheetDescription>
              {machines.find((node) => node.id === activityId)?.name}
            </SheetDescription>
          </SheetHeader>
          {activityId && (
            <MachineActivity key={activityId} nodeId={activityId} />
          )}
        </SheetContent>
      </Sheet>
      <Sheet
        open={Boolean(selected)}
        onOpenChange={(open) => {
          if (!open) setSelectedId(undefined);
        }}
      >
        <SheetContent className="flex w-full min-w-0 flex-col gap-0 overflow-hidden p-0 sm:max-w-lg">
          <SheetHeader className="shrink-0 p-5 pr-10">
            <SheetTitle>{selected?.name} settings</SheetTitle>
            <SheetDescription>
              Control confirmations, saved-login typing and specialist access.
            </SheetDescription>
          </SheetHeader>
          {selected ? (
            <MachineSettings
              key={selected.id}
              node={selected}
              canManage
              update={updates.data?.find((row) => row.node_id === selected.id)}
            />
          ) : null}
        </SheetContent>
      </Sheet>
    </div>
  );
}

function RemoteMachineList({
  machines,
  onOpen,
}: {
  readonly machines: RemoteMachine[];
  readonly onOpen: (id: string) => void;
}) {
  const label = (value: string) =>
    value.replaceAll("_", " ").replace(/^./, (c) => c.toUpperCase());
  const state = (machine: RemoteMachine) => (
    <div className="flex flex-wrap gap-2">
      <Badge
        variant={
          machine.resource_state === "ready"
            ? "success"
            : machine.resource_state === "failed"
              ? "destructive"
              : "secondary"
        }
      >
        {label(machine.resource_state)}
      </Badge>
      <span className="text-11 text-muted-foreground">
        Billing: {label(machine.billing_state)} · Authorization:{" "}
        {label(machine.authorization_state)}
      </span>
    </div>
  );
  return (
    <>
      <div className="flex flex-col gap-3 md:hidden">
        {machines.map((machine) => (
          <article
            key={machine.node_id}
            className="space-y-3 rounded-xl border border-border/50 bg-card p-4"
          >
            <h4 className="break-all text-13 font-semibold">
              {machine.name ?? machine.sandbox_name}
            </h4>
            {state(machine)}
            <p className="text-11 text-muted-foreground">
              Generation {machine.runtime_generation} ·{" "}
              {machine.profile.workspace_gib} GiB workspace
            </p>
            <Button onClick={() => onOpen(machine.node_id)}>
              View details
            </Button>
          </article>
        ))}
      </div>
      <div className="hidden overflow-hidden rounded-xl border border-border/50 bg-card md:block">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Machine</TableHead>
              <TableHead>State</TableHead>
              <TableHead>Runtime</TableHead>
              <TableHead>
                <span className="sr-only">Details</span>
              </TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {machines.map((machine) => (
              <TableRow key={machine.node_id}>
                <TableCell>
                  <div className="font-medium">
                    {machine.name ?? machine.sandbox_name}
                  </div>
                  <div className="font-mono text-11 text-muted-foreground">
                    {machine.node_id}
                  </div>
                </TableCell>
                <TableCell>{state(machine)}</TableCell>
                <TableCell>
                  Generation {machine.runtime_generation}
                  <br />
                  <span className="text-11 text-muted-foreground">
                    {machine.profile.workspace_gib} GiB workspace
                  </span>
                </TableCell>
                <TableCell>
                  <Button
                    onClick={() => onOpen(machine.node_id)}
                    aria-label={`View ${machine.name ?? machine.sandbox_name}`}
                  >
                    Details
                  </Button>
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </div>
    </>
  );
}
