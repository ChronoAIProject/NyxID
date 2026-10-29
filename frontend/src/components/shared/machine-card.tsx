import { Link } from "@tanstack/react-router";
import { History, Monitor, Pause, Play, Settings } from "lucide-react";
import type {
  MachineProfile,
  MachineUpdateStatus,
  RemoteMachine,
} from "@/schemas/machines";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { remoteMachineActionEligible } from "@/lib/remote-machine-lifecycle";
import {
  MachineIsolationBadge,
  MachineSharingBadge,
} from "./machine-isolation";
export function MachineCard({
  id,
  name,
  owner,
  machine,
  connected,
  version,
  desktopReady,
  remote,
  update,
  onSettings,
  onActivity,
  onAction,
  progress,
  actionPending,
}: {
  readonly id: string;
  readonly name: string;
  readonly owner: string;
  readonly machine?: MachineProfile | null;
  readonly connected: boolean;
  readonly version?: string | null;
  readonly desktopReady?: boolean;
  readonly remote?: RemoteMachine;
  readonly update?: MachineUpdateStatus;
  readonly onSettings: () => void;
  readonly onActivity: () => void;
  readonly onAction?: (action: "pause" | "resume") => void;
  readonly progress?: React.ReactNode;
  readonly actionPending?: boolean;
}) {
  const running =
    !remote ||
    (["ready", "degraded"].includes(remote.resource_state) &&
      remote.authorization_state === "active");
  const desktop = Boolean(
    machine?.browser ||
    machine?.computer ||
    remote?.capabilities?.some((v) => v === "browser" || v === "computer"),
  );
  const ready = connected && desktopReady === true && running;
  const capabilities = machine
    ? [
        machine.shell && "Commands",
        machine.files && "Files",
        (machine.browser ?? machine.computer) && "Browser",
        machine.computer && "Computer",
      ].filter(Boolean)
    : remote?.capabilities?.map(
        (v) =>
          ({
            shell: "Commands",
            files: "Files",
            browser: "Browser",
            computer: "Computer",
          })[v],
      );
  return (
    <article
      aria-label={name}
      className="space-y-3 rounded-xl border border-border/50 bg-card p-4"
    >
      <div className="flex flex-wrap items-center gap-2">
        <Monitor className="size-4 text-text-tertiary" />
        <h3 className="min-w-0 flex-1 truncate text-13 font-medium">{name}</h3>
        <span className="text-11 text-muted-foreground">
          {version ? `v${version}` : "Version not reported"}
        </span>
        {update?.update_available && (
          <Badge variant="warning">
            Update available ({update.target_version})
          </Badge>
        )}
        <Badge
          variant={
            remote?.resource_state === "failed"
              ? "destructive"
              : connected && running
                ? "success"
                : "secondary"
          }
        >
          {remote
            ? remote.resource_state === "ready"
              ? "Running"
              : remote.resource_state.replaceAll("_", " ")
            : connected
              ? "Connected"
              : "Disconnected"}
        </Badge>
        {machine && <MachineIsolationBadge machine={machine} />}
        <MachineSharingBadge />
      </div>
      <p className="text-12 text-muted-foreground">
        {capabilities?.join(" · ") || "Capabilities not reported"} ·{" "}
        {machine
          ? `${machine.os} / ${machine.arch}`
          : "Awaiting machine report"}
      </p>
      <p className="text-12 text-muted-foreground">
        {owner} · {connected && running ? "Connected" : "Disconnected"}
        {desktop && !ready
          ? " · Computer permissions or display unavailable"
          : ""}
      </p>
      {remote &&
        [
          "paused",
          "deleting",
          "provisioning",
          "resuming",
          "stopping",
          "reconciling",
        ].includes(remote.resource_state) && (
          <p className="text-11 text-muted-foreground">
            {remote.free_slot ? "Personal free slot · " : ""}
            {remote.resource_state === "paused"
              ? "Workspace kept. Resume to continue. The slot remains in use."
              : remote.resource_state === "deleting"
                ? "Deletion in progress. Your slot remains in use until deletion is confirmed."
                : [
                      "provisioning",
                      "resuming",
                      "stopping",
                      "reconciling",
                    ].includes(remote.resource_state)
                  ? "Waiting for platform confirmation. You can return later."
                  : ""}
          </p>
        )}
      {progress}
      <div className="flex flex-wrap gap-2">
        {desktop &&
          (ready ? (
            <Button asChild>
              <Link
                to="/assistant/machines/$nodeId/desktop"
                params={{ nodeId: id }}
              >
                Watch / Take over desktop
              </Link>
            </Button>
          ) : (
            <Button disabled>Watch / Take over desktop</Button>
          ))}
        <Button aria-label={`Activity for ${name}`} onClick={onActivity}>
          <History />
          Activity
        </Button>
        <Button aria-label={`Settings for ${name}`} onClick={onSettings}>
          <Settings />
          Settings
        </Button>
        {remote && !["deleting", "deleted"].includes(remote.resource_state) && (
          <Button
            disabled={
              actionPending ||
              !remoteMachineActionEligible(remote.resource_state, remote.resource_state === "paused" ? "resume" : "pause")
            }
            onClick={() =>
              onAction?.(
                remote.resource_state === "paused" ? "resume" : "pause",
              )
            }
          >
            {remote.resource_state === "paused" ? <Play /> : <Pause />}
            {remote.resource_state === "paused"
              ? "Resume"
              : remote.resource_state === "stopping"
                ? "Pausing…"
                : remote.resource_state === "resuming"
                  ? "Resuming…"
                  : remote.resource_state === "provisioning"
                    ? "Creating…"
                    : "Pause"}
          </Button>
        )}
      </div>
    </article>
  );
}
