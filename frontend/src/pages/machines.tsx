import { remoteMachineReportedProfile } from "@/lib/remote-machine-presentation";
import { remoteMachineLifecyclePending } from "@/lib/remote-machine-lifecycle";
import { useState } from "react";
import { Server } from "lucide-react";
import { useNavigate, useSearch } from "@tanstack/react-router";
import { useRemoteMachineAdmission } from "@/hooks/use-remote-machine-admission";
import { useRemoteMachineContinuity } from "@/hooks/use-remote-machine-continuity";
import {
  useMachineUpdates,
  useRemoteMachines,
  useRemoteMachineOperation,
  useRemoteMachineEntitlement,
} from "@/hooks/use-machines";
import { useNodes } from "@/hooks/use-nodes";
import { useOrgs } from "@/hooks/use-orgs";
import { useAuthStore } from "@/stores/auth-store";
import { parseMachinesSearch } from "@/lib/machine-search";
import { readRemoteMachineIntent } from "@/lib/remote-machine-intent";
import { RemoteMachineDetail } from "@/components/machines/remote-machine-detail";
import { RemoteMachineActivity } from "@/components/machines/remote-machine-activity";
import { MachineCard } from "@/components/shared/machine-card";
import { MachineActivity } from "@/components/shared/machine-activity";
import { MachineSettings } from "@/components/shared/machine-settings";
import { SavedLoginsPage } from "./saved-logins";
import { PageHeader } from "@/components/shared/page-header";
import { AddCtaButton } from "@/components/shared/add-cta-button";
import { ErrorBanner } from "@/components/shared/error-banner";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs";
import {
  Sheet,
  SheetContent,
  SheetHeader,
  SheetTitle,
  SheetDescription,
} from "@/components/ui/sheet";
import type { RemoteMachine } from "@/schemas/machines";

function RemoteProgress({ machine, children }: {
  readonly machine: RemoteMachine;
  readonly children: (progress: React.ReactNode, pending: boolean) => React.ReactNode;
}) {
  const actor = useAuthStore((state) => state.user?.id);
  const { operationId, acceptedAction, machineReadBehind, readError } = useRemoteMachineContinuity(machine.node_id, machine);
  const unresolvedActions = (["pause", "resume", "delete"] as const).filter(
    (action) => {
      if (!actor) return false;
      try {
        return Boolean(
          readRemoteMachineIntent(actor, `${machine.node_id}:${action}`),
        );
      } catch {
        // An unreadable saved request also requires explicit status/review.
        return true;
      }
    },
  );
  // Observe accepted work beyond the Settings drawer lifetime. Mutation retries
  // still use the original persisted operation/body/key in the lifecycle adapter.
  const query = useRemoteMachineOperation(
    operationId,
  );
  const operation =
    !query.error && query.data && query.data.id === operationId && query.data.machine_id === machine.node_id
      ? query.data
      : undefined;
  const active =
    operation &&
    !operation.terminal_at &&
    !["succeeded", "failed"].includes(operation.state);
  const action = acceptedAction ?? operation?.action;
  const lifecyclePending = remoteMachineLifecyclePending(machine, operationId, operation, machineReadBehind);
  const message =
    readError ?? (query.error && operationId
      ? "Operation status unavailable. Open Settings to check status."
      : active && operation.delete_retry_stopped
        ? "Deletion needs attention. Open Settings to retry the original deletion."
        : operation?.state === "failed"
          ? "The operation needs attention. Open Settings to review the confirmed result."
          : operationId && !operation && !acceptedAction
            ? "Current operation is awaiting confirmation. Open Settings to check status."
          : machineReadBehind && !operationId
            ? "Machine state is being refreshed. Open Settings to check status."
          : lifecyclePending ||
              [
                "provisioning",
                "stopping",
                "resuming",
                "deleting",
                "reconciling",
              ].includes(machine.resource_state)
            ? action === "pause"
              ? "Pausing your machine. Your workspace and free slot are kept."
              : action === "delete"
                ? "Deleting your machine. Your free slot stays occupied until deletion is confirmed."
                : action === "resume"
                  ? "Resuming your machine. Waiting for confirmation."
                  : action === "create"
                    ? "Setting up your machine. You can return later."
                  : machine.resource_state === "provisioning"
              ? "Setting up your machine. You can return later."
              : machine.resource_state === "stopping"
                ? "Pausing your machine. Your workspace and free slot are kept."
                : machine.resource_state === "deleting"
                  ? "Deleting your machine. Your free slot stays occupied until deletion is confirmed."
                  : machine.resource_state === "reconciling"
                    ? "Taking longer than usual. Open Settings to check status."
                    : "Resuming your machine. Waiting for confirmation."
            : null);
  const progress = message || unresolvedActions.length ? (
    <div
      role="status"
      className="space-y-2 rounded-lg border border-warning/30 p-3 text-12"
    >
      {unresolvedActions.map((action) => (
        <p key={action}>
          A previous {action} request has an unconfirmed result. Open Settings
          to check status before retrying.
        </p>
      ))}
      {message && <p>{message}</p>}
    </div>
  ) : null;
  return children(progress, lifecyclePending);
}
export function MachinesPage() {
  const admission = useRemoteMachineAdmission();
  const navigate = useNavigate();
  const search = parseMachinesSearch(
    useSearch({ strict: false }) as Record<string, unknown>,
  );
  const nodes = useNodes({ pollIntervalMs: 5000 });
  const updates = useMachineUpdates();
  const remote = useRemoteMachines();
  const entitlement = useRemoteMachineEntitlement(admission);
  const orgs = useOrgs();
  const userId = useAuthStore((state) => state.user?.id);
  const [drawer, setDrawer] = useState<
    | { id: string; view: "settings" | "activity"; action?: "pause" | "resume" }
    | undefined
  >(() =>
    search.machine && search.tab !== "logins"
      ? { id: search.machine, view: search.activity ? "activity" : "settings" }
      : undefined,
  );
  const [location, setLocation] = useState(
    JSON.stringify([userId, search.machine, search.tab, search.activity]),
  );
  const nextLocation = JSON.stringify([
    userId,
    search.machine,
    search.tab,
    search.activity,
  ]);
  if (location !== nextLocation) {
    setLocation(nextLocation);
    setDrawer(
      search.machine && search.tab !== "logins"
        ? drawer?.id === search.machine &&
          drawer.view === (search.activity ? "activity" : "settings")
          ? drawer
          : {
              id: search.machine,
              view: search.activity ? "activity" : "settings",
            }
        : undefined,
    );
  }
  const adminOrgs = new Set(
    orgs.data?.filter((org) => org.your_role === "admin").map((org) => org.id),
  );
  const hosted = remote.data ?? [];
  const hostedIds = new Set(hosted.map((machine) => machine.node_id));
  const selfHosted = (nodes.data ?? []).filter(
    (node) =>
      node.machine &&
      node.metadata?.provisioning_source !== "remote_machine" &&
      !hostedIds.has(node.id) &&
      (node.owner.id === userId ||
        (node.owner.kind === "org" && adminOrgs.has(node.owner.id))),
  );
  const selected = selfHosted.find((node) => node.id === drawer?.id);
  const selectedRemote = hosted.find(
    (machine) => machine.node_id === drawer?.id,
  );
  const isRemote = Boolean(drawer && !selected && !nodes.isPending);
  const name = selected?.name ?? selectedRemote?.name ?? "Remote machine";
  function open(
    id: string,
    view: "settings" | "activity",
    action?: "pause" | "resume",
  ) {
    setDrawer({ id, view, action });
    void navigate({
      to: "/assistant/machines",
      search: {
        machine: id,
        activity: view === "activity" ? "tools" : undefined,
      },
    });
  }
  function close() {
    setDrawer(undefined);
    void navigate({ to: "/assistant/machines", search: {} });
  }
  return (
    <div className="mx-auto max-w-5xl space-y-6">
      <Tabs
        value={search.tab ?? "machines"}
        onValueChange={(tab) => {
          setDrawer(undefined);
          void navigate({
            to: "/assistant/machines",
            search: { tab: tab === "logins" ? "logins" : undefined },
          });
        }}
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
                {admission && (
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
            {(admission || hosted.length > 0) && (
              <section
                aria-label="Personal free slot"
                className="flex flex-wrap items-center gap-3 rounded-xl border border-border/50 bg-card p-4 text-12"
              >
                <Server className="size-4 text-text-tertiary" />
                <div className="min-w-0 flex-1">
                  <p className="font-medium">Personal free slot</p>
                  <p className="text-11 text-muted-foreground">
                    Machine runtime is free. Model and external API usage may be
                    billed separately.
                  </p>
                </div>
                <Badge variant="secondary">
                  {entitlement.error || !entitlement.data
                    ? "Availability not confirmed"
                    : !entitlement.data.eligible
                      ? "Not eligible"
                      : entitlement.data.free_slot_available
                        ? "Available · 0 of 1"
                        : "1 of 1 in use"}
                </Badge>
              </section>
            )}
            {remote.isPending ? (
              <p role="status">Loading remote machines…</p>
            ) : remote.error ? (
              <ErrorBanner
                message="Remote machine management could not be loaded. This deployment may not expose it yet."
                onRetry={() => void remote.refetch()}
              />
            ) : (
              <>
                {!remote.discoveryComplete && (
                  <p role="status" className="text-12 text-muted-foreground">
                    Complete remote Machine discovery is not confirmed by this
                    server.
                  </p>
                )}
                <div className="grid gap-4 xl:grid-cols-2">
                  {hosted.map((machine) => {
                    const node = nodes.data?.find(
                      (item) =>
                        item.id === machine.node_id && item.owner.id === userId,
                    );
                    const presentation = machine.presentation;
                    const profile = presentation
                      ? remoteMachineReportedProfile(presentation)
                      : node?.machine;
                    const connected = Boolean(
                      ["ready", "degraded"].includes(machine.resource_state) &&
                      machine.authorization_state === "active" &&
                      (presentation
                        ? presentation.connected
                        : node?.is_connected),
                    );
                    return (
                      <RemoteProgress key={machine.node_id} machine={machine}>
                        {(progress, actionPending) => <MachineCard
                        id={machine.node_id}
                        name={machine.name ?? machine.sandbox_name}
                        owner="Personal account"
                        machine={profile}
                        connected={connected}
                        desktopReady={
                          presentation?.desktop_ready ?? profile?.computer_ready
                        }
                        version={
                          presentation
                            ? presentation.agent_version
                            : node?.metadata?.agent_version
                        }
                        remote={machine}
                        onSettings={() => open(machine.node_id, "settings")}
                        onActivity={() => open(machine.node_id, "activity")}
                        onAction={(action) =>
                          open(machine.node_id, "settings", action)
                        }
                        progress={progress}
                        actionPending={actionPending}
                      />}
                      </RemoteProgress>
                    );
                  })}
                </div>
                {!hosted.length && (
                  <div className="space-y-3 rounded-xl border border-dashed border-border p-6 text-12">
                    <p className="text-muted-foreground">
                      {remote.discoveryComplete
                        ? "No remote machines. Check your free slot before creating a workspace."
                        : "No remote machines were returned. Discovery is not yet complete."}
                    </p>
                    {admission && (
                      <Button
                        onClick={() =>
                          void navigate({
                            to: "/assistant/machines/new",
                            search: { where: "hosted" },
                          })
                        }
                      >
                        Create remote machine
                      </Button>
                    )}
                  </div>
                )}
              </>
            )}
          </section>
          <section aria-label="Self-hosted machines" className="space-y-3">
            <h3 className="text-15 font-semibold">Self-hosted machines</h3>
            {nodes.isPending || orgs.isPending ? (
              <p role="status">Loading machines…</p>
            ) : nodes.error || orgs.error ? (
              <p role="alert">Could not load machines.</p>
            ) : (
              !selfHosted.length && (
                <p className="rounded-lg border border-dashed border-border p-6 text-12 text-muted-foreground">
                  Add a machine to let NyxBot run commands, work with files and
                  use a desktop.
                </p>
              )
            )}
            <div className="grid gap-4 xl:grid-cols-2">
              {selfHosted.map((node) => (
                <MachineCard
                  key={node.id}
                  id={node.id}
                  name={node.name}
                  owner={node.owner.display_name}
                  machine={node.machine}
                  version={node.metadata?.agent_version}
                  connected={node.is_connected}
                  desktopReady={node.machine?.computer_ready}
                  update={updates.data?.find((row) => row.node_id === node.id)}
                  onSettings={() => open(node.id, "settings")}
                  onActivity={() => open(node.id, "activity")}
                />
              ))}
            </div>
          </section>
        </TabsContent>
        <TabsContent value="logins" className="mt-6">
          <SavedLoginsPage
            key={userId}
            onBackToSettings={
              search.machine
                ? () => open(search.machine!, "settings")
                : undefined
            }
          />
        </TabsContent>
      </Tabs>
      <Sheet
        open={Boolean(drawer)}
        onOpenChange={(value) => {
          if (!value) close();
        }}
      >
        <SheetContent className="flex w-full min-w-0 flex-col gap-0 overflow-hidden p-0 sm:max-w-lg">
          <SheetHeader className="shrink-0 p-5 pr-10">
            <SheetTitle>
              {drawer?.view === "activity"
                ? "Machine activity"
                : `${name} settings`}
            </SheetTitle>
            <SheetDescription>
              {drawer?.view === "activity"
                ? name
                : "Control confirmations, saved-login typing and specialist access."}
            </SheetDescription>
          </SheetHeader>
          {drawer &&
            (drawer.view === "activity" ? (
              isRemote ? (
                <RemoteMachineActivity
                  key={`${userId}:${drawer.id}`}
                  machineId={drawer.id}
                  initialTab={search.activity}
                />
              ) : (
                <MachineActivity
                  key={`${userId}:${drawer.id}`}
                  nodeId={drawer.id}
                />
              )
            ) : selected ? (
              <MachineSettings
                key={`${userId}:${selected.id}`}
                node={selected}
                canManage
                update={updates.data?.find(
                  (row) => row.node_id === selected.id,
                )}
              />
            ) : isRemote ? (
              <RemoteMachineDetail
                key={`${userId}:${drawer.id}`}
                machineId={drawer.id}
                initialMachine={selectedRemote}
                operationId={search.operation}
                confirmationId={search.confirmation}
                initialAgentId={search.agent}
                initialAction={drawer.action}
                node={
                  nodes.error
                    ? undefined
                    : nodes.data?.find(
                        (node) =>
                          node.id === drawer.id && node.owner.id === userId,
                      )
                }
                onClose={close}
                onActivity={() => open(drawer.id, "activity")}
                onLogins={() => {
                  setDrawer(undefined);
                  void navigate({
                    to: "/assistant/machines",
                    search: { tab: "logins", machine: drawer.id },
                  });
                }}
              />
            ) : (
              <p role="status">Loading machine…</p>
            ))}
        </SheetContent>
      </Sheet>
    </div>
  );
}
