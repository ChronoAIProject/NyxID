import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { CompactChrome } from "@/components/ui/compact-title";
import { TEXT_STYLES } from "@/lib/typography";
import {
  GroupedPage,
  GroupedSheet as Sheet,
  GroupedSheetContent as SheetContent,
} from "@/components/ui/grouped-dialog";
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
import { WorkspacePageHeader } from "@/components/assistant/workspace-page-header";
import { AddCtaButton } from "@/components/shared/add-cta-button";
import { NodeStatusBadge } from "@/components/shared/node-status-badge";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs";
import {
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
  useUiTranslation();
  const machine = node.machine!;
  return (
    <article className="space-y-3 rounded-xl border border-border/50 bg-card p-4">
      <div className="flex flex-wrap items-center gap-2">
        <Monitor className="h-4 w-4 text-text-tertiary" />
        <h3 data-machine-card-title="" className={`min-w-0 flex-1 truncate ${TEXT_STYLES.cardTitle}`}>
          {node.name}
        </h3>
        {update?.update_available ? (
          <Badge variant="warning">{translateUi("Update available (")}{update.target_version})
          </Badge>
        ) : null}
        <span className="text-13 text-muted-foreground">
          v{node.metadata?.agent_version ?? translateUi("unknown")}
        </span>
        <NodeStatusBadge status={node.status} isConnected={node.is_connected} />
        <MachineIsolationBadge machine={machine} />
        <MachineSharingBadge />
      </div>
      <p className="text-14 text-muted-foreground">
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
      <p className="text-14 text-muted-foreground">
        {node.owner.display_name} ·{" "}
        {node.is_connected ? translateUi("Connected") : translateUi("Disconnected")}
        {machine.computer && !machine.computer_ready
          ? translateUi(" · Computer permissions or display unavailable")
          : ""}
      </p>
      <div className="flex flex-wrap gap-2">
        {machine.computer || machine.browser ? (
          <Button asChild disabled={!node.is_connected}>
            <Link
              to="/assistant/machines/$nodeId/desktop"
              params={{ nodeId: node.id }}
              disabled={!node.is_connected}
            >{translateUi("Watch / Take over desktop")}</Link>
          </Button>
        ) : null}
        <Button onClick={onActivity}>
          <History />{translateUi("Activity")}</Button>
        <Button onClick={onSettings} aria-label={translateUi("Settings for {{value1}}", { value1: node.name })}>
          <Settings />{translateUi("Settings")}</Button>
      </div>
    </article>
  );
}

export function MachinesPage() {
  useUiTranslation();
  return (
    <GroupedPage>
      <MachinesContent />
    </GroupedPage>
  );
}

function MachinesContent() {
  useUiTranslation();
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
    <div className="shell-page-stack">
      <WorkspacePageHeader
        title={translateUi("Machines")}
        description={translateUi("Remote and self-hosted computers for authorized agents and clients.")}
        actions={
          <div className="flex flex-wrap gap-2">
            {remoteAdmission && (
              <AddCtaButton
                label={translateUi("Create remote machine")}
                onClick={() =>
                  void navigate({
                    to: "/assistant/machines/new",
                    search: { where: "hosted" },
                  })
                }
              />
            )}
            <AddCtaButton
              label={translateUi("Add a machine")}
              onClick={() =>
                void navigate({ to: "/assistant/machines/new" })
              }
            />
          </div>
        }
      />
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
          <TabsTrigger value="machines">{translateUi("Machines")}</TabsTrigger>
          <TabsTrigger value="logins">{translateUi("Saved logins")}</TabsTrigger>
        </TabsList>
        <TabsContent value="machines" className="mt-6 space-y-6">
          <section aria-label={translateUi("Remote machines")} className="space-y-3">
            <h3 className="type-card-title">{translateUi("Remote machines")}</h3>
            <p className="text-14 text-muted-foreground">{translateUi("Personal hosted machines with a fixed workspace and selected capabilities. Resource state, runtime billing and execution authorization are independent.")}</p>
            {!remote.isPending &&
              !remote.error &&
              !remote.discoveryComplete && (
                <p role="status" className="text-14 text-muted-foreground">{translateUi("Complete remote Machine discovery is not confirmed by this server.")}</p>
              )}
            {remote.isPending ? (
              <p role="status" className="text-14">{translateUi("Loading remote machines…")}</p>
            ) : remote.error ? (
              <ErrorBanner
                message={translateUi("Remote machine management could not be loaded. This deployment may not expose it yet.")}
                onRetry={() => void remote.refetch()}
              />
            ) : hostedMachines.length ? (
              <RemoteMachineList
                machines={hostedMachines}
                onOpen={openRemote}
              />
            ) : (
              <p className="rounded-lg border border-dashed border-border p-6 text-14 text-muted-foreground">{translateUi("No remote machines. Check your free slot and the configured profile before creating one.")}</p>
            )}
          </section>
          <h3 className="type-card-title">{translateUi("Self-hosted machines")}</h3>
          {nodes.isPending || orgs.isPending ? (
            <p role="status">{translateUi("Loading machines…")}</p>
          ) : null}
          {nodes.error || orgs.error ? (
            <p role="alert" className="text-14 text-destructive">{translateUi("Could not load machines.")}{" "}
              {nodes.error?.message ?? orgs.error?.message}
            </p>
          ) : null}
          {!nodes.isPending &&
          !orgs.isPending &&
          !nodes.error &&
          !orgs.error &&
          !machines.length ? (
            <p className="rounded-xl border border-border/50 p-6 text-14 text-muted-foreground">{translateUi("Add a machine to let NyxBot run commands, work with files and use a desktop.")}</p>
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
            <SheetTitle>{translateUi("Remote machine details")}</SheetTitle>
            <SheetDescription>{translateUi("Lifecycle, access, work history and billing for this machine.")}</SheetDescription>
          </SheetHeader>
          {remoteDetailId ? (
            <RemoteMachineDetail
              key={`${userId}:${remoteDetailId}:${search.operation ?? ""}:${search.confirmation ?? ""}`}
              operationId={search.operation}
              confirmationId={search.confirmation}
              machineId={remoteDetailId}
              onClose={() => {
                setSelectedId(undefined);
                void navigate({ to: "/assistant/machines", search: {} });
              }}
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
            <SheetTitle>{translateUi("Machine activity")}</SheetTitle>
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
          <CompactChrome><SheetHeader className="shrink-0 p-5 pr-10 [[data-compact=true]_&]:py-2">
            <SheetTitle className="[[data-compact=true]_&]:text-15">{selected?.name}{translateUi(" settings")}</SheetTitle>
            <SheetDescription className="[[data-compact=true]_&]:hidden">{translateUi("Control confirmations, saved-login typing and specialist access.")}</SheetDescription>
          </SheetHeader></CompactChrome>
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
  useUiTranslation();
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
      <span className="text-13 text-muted-foreground">{translateUi("Billing: ")}{label(machine.billing_state)}{translateUi(" · Authorization:")}{" "}
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
            <h4 className="break-all type-card-title">
              {machine.name ?? machine.sandbox_name}
            </h4>
            {state(machine)}
            <p className="text-13 text-muted-foreground">{translateUi("Generation ")}{machine.runtime_generation} ·{" "}
              {machine.profile.workspace_gib}{translateUi(" GiB workspace")}</p>
            <Button onClick={() => onOpen(machine.node_id)}>{translateUi("View details")}</Button>
          </article>
        ))}
      </div>
      <div className="hidden overflow-hidden rounded-xl border border-border/50 bg-card md:block">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>{translateUi("Machine")}</TableHead>
              <TableHead>{translateUi("State")}</TableHead>
              <TableHead>{translateUi("Runtime")}</TableHead>
              <TableHead>
                <span className="sr-only">{translateUi("Details")}</span>
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
                  <div className="font-mono text-13 text-muted-foreground">
                    {machine.node_id}
                  </div>
                </TableCell>
                <TableCell>{state(machine)}</TableCell>
                <TableCell>{translateUi("Generation ")}{machine.runtime_generation}
                  <br />
                  <span className="text-13 text-muted-foreground">
                    {machine.profile.workspace_gib}{translateUi(" GiB workspace")}</span>
                </TableCell>
                <TableCell>
                  <Button
                    onClick={() => onOpen(machine.node_id)}
                    aria-label={translateUi("View {{value1}}", { value1: machine.name ?? machine.sandbox_name })}
                  >{translateUi("Details")}</Button>
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </div>
    </>
  );
}
