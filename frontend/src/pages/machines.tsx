import { useState } from "react";
import { Link, useNavigate, useSearch } from "@tanstack/react-router";
import { Monitor, Settings } from "lucide-react";
import { useNodes } from "@/hooks/use-nodes";
import { useOrgs } from "@/hooks/use-orgs";
import { useAuthStore } from "@/stores/auth-store";
import { parseMachinesSearch } from "@/lib/machine-search";
import { SavedLoginsPage } from "@/pages/saved-logins";
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
}: {
  readonly node: NodeInfo;
  readonly onSettings: () => void;
}) {
  const machine = node.machine!;
  return (
    <article className="space-y-3 rounded-xl border border-border/50 bg-card p-4">
      <div className="flex flex-wrap items-center gap-2">
        <Monitor className="h-4 w-4 text-text-tertiary" />
        <h3 className="min-w-0 flex-1 truncate text-13 font-medium">
          {node.name}
        </h3>
        <NodeStatusBadge status={node.status} isConnected={node.is_connected} />
        {machine.shell && !machine.browser_isolated ? (
          <Badge variant="warning">Not isolated</Badge>
        ) : null}
      </div>
      <p className="text-12 text-muted-foreground">
        {[
          machine.shell && "Commands",
          machine.files && "Files",
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
        {machine.computer ? (
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
  const orgs = useOrgs();
  const userId = useAuthStore((state) => state.user?.id);
  const [selectedId, setSelectedId] = useState<string>();
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
                onSettings={() => setSelectedId(node.id)}
              />
            ))}
          </div>
        </TabsContent>
        <TabsContent value="logins" className="mt-6">
          <SavedLoginsPage />
        </TabsContent>
      </Tabs>
      <Sheet
        open={Boolean(selected)}
        onOpenChange={(open) => {
          if (!open) setSelectedId(undefined);
        }}
      >
        <SheetContent className="w-full space-y-6 overflow-y-auto sm:max-w-lg">
          <SheetHeader>
            <SheetTitle>{selected?.name} settings</SheetTitle>
            <SheetDescription>
              Control confirmations, saved-login typing and specialist access.
            </SheetDescription>
          </SheetHeader>
          {selected ? (
            <MachineSettings key={selected.id} node={selected} canManage />
          ) : null}
        </SheetContent>
      </Sheet>
    </div>
  );
}
