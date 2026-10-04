import { MachineIsolationBadge, MachineSharingBadge } from "@/components/shared/machine-isolation";
import { useState } from "react";
import { Link, useNavigate, useSearch } from "@tanstack/react-router";
import { Monitor, Settings, History } from "lucide-react";
import { useMachineUpdates } from "@/hooks/use-machines";
import type { MachineUpdateStatus } from "@/schemas/machines";
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
        <h3 className="min-w-0 flex-1 truncate text-[13px] font-medium">
          {node.name}
        </h3>
        {update?.update_available ? (
          <Badge variant="warning">
            Update available ({update.target_version})
          </Badge>
        ) : null}
        <span className="text-[11px] text-muted-foreground">
          v{node.metadata?.agent_version ?? "unknown"}
        </span>
        <NodeStatusBadge status={node.status} isConnected={node.is_connected} />
        <MachineIsolationBadge machine={machine} />
        <MachineSharingBadge />
      </div>
      <p className="text-[12px] text-muted-foreground">
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
      <p className="text-[12px] text-muted-foreground">
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
        <Button onClick={onActivity}><History />Activity</Button>
        <Button onClick={onSettings} aria-label={`Settings for ${node.name}`}>
          <Settings />
          Settings
        </Button>
      </div>
    </article>
  );
}

export function MachinesPage() {
  const navigate = useNavigate();
  const search = parseMachinesSearch(
    useSearch({ strict: false }) as Record<string, unknown>,
  );
  const nodes = useNodes({ pollIntervalMs: 5000 });
  const updates = useMachineUpdates();
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
  const machines = (nodes.data ?? []).filter(
    (node) =>
      node.machine &&
      (node.owner.id === userId ||
        (node.owner.kind === "org" && adminOrgs.has(node.owner.id))),
  );
  const selected = machines.find((node) => node.id === selectedId);
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
            description="Computers NyxBot and your specialists can work on."
            actions={
              <AddCtaButton
                label="Add a machine"
                onClick={() => void navigate({ to: "/assistant/machines/new" })}
              />
            }
          />
          {nodes.isPending || orgs.isPending ? (
            <p role="status">Loading machines…</p>
          ) : null}
          {nodes.error || orgs.error ? (
            <p role="alert" className="text-[12px] text-destructive">
              Could not load machines.{" "}
              {nodes.error?.message ?? orgs.error?.message}
            </p>
          ) : null}
          {!nodes.isPending &&
          !orgs.isPending &&
          !nodes.error &&
          !orgs.error &&
          !machines.length ? (
            <p className="rounded-xl border border-border/50 p-6 text-[12px] text-muted-foreground">
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
      <Sheet open={Boolean(activityId)} onOpenChange={(open) => { if (!open) setActivityId(undefined); }}>
        <SheetContent className="flex w-full min-w-0 flex-col gap-0 overflow-hidden p-0 sm:max-w-lg">
          <SheetHeader className="shrink-0 p-5 pr-10">
            <SheetTitle>Machine activity</SheetTitle>
            <SheetDescription>{machines.find((node) => node.id === activityId)?.name}</SheetDescription>
          </SheetHeader>
          {activityId && <MachineActivity key={activityId} nodeId={activityId} />}
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
