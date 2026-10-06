import { useState } from "react";
import {
  useStartMachineUpdate,
  useVerifiedUpdaterImage,
} from "@/hooks/use-machines";
import {
  machineMigrationCommand,
  machineCompanionReplacementCommand,
  type MachineUpdateStatus,
} from "@/schemas/machines";
import { CopyableField } from "@/components/shared/copyable-field";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";

export function MachineUpdate({
  name,
  status,
}: {
  readonly name: string;
  readonly status: MachineUpdateStatus;
}) {
  const start = useStartMachineUpdate(status.node_id);
  const updater = useVerifiedUpdaterImage();
  const [confirming, setConfirming] = useState(false);
  const [container, setContainer] = useState(name);
  const legacy = status.updater?.phase === "legacy";
  const manual = legacy || !status.updater_ready;
  const busy = [
    "queued",
    "verifying",
    "downloading",
    "restarting",
    "manual_step",
  ].includes(status.phase) || status.updater?.phase === "pending";
  let command: string | undefined;
  if (manual && status.installation === "container") {
    try {
      const renderCommand = legacy
        ? machineCompanionReplacementCommand
        : machineMigrationCommand;
      command = renderCommand(
        container,
        status.target_version,
        updater.data?.version === status.target_version
          ? (updater.data.image ?? undefined)
          : undefined,
      );
    } catch {
      /* Input validation is displayed below. */
    }
  }
  return (
    <div className="min-w-0 space-y-3">
      <p>
        Installed: {status.current_version} · Latest: {status.target_version}
      </p>
      {legacy ? (
        <p role="status">Updater predates self-update. Replace it once</p>
      ) : status.updater ? (
        <div className="min-w-0 space-y-2">
          <p>Updater: {status.updater.version}</p>
          {status.updater.phase !== "current" ? (
            <p role="status" className="break-words">
              Updater update {status.updater.phase}
              {status.updater.target_version ? ` (${status.updater.target_version})` : ""}
              {status.updater.code ? ` — ${status.updater.code}` : ""}
            </p>
          ) : null}
          {status.updater_guidance ? (
            <p role="alert" className="break-words">{status.updater_guidance}</p>
          ) : null}
          {status.updater.digest ? (
            <details>
              <summary className="cursor-pointer">Updater image digest</summary>
              <code className="block break-all pt-2 text-xs">{status.updater.digest}</code>
            </details>
          ) : null}
        </div>
      ) : status.installation === "container" && status.updater_ready ? (
        <p>Updater: version unavailable</p>
      ) : null}
      {status.update_available ? (
        <Badge variant="warning">
          Update available ({status.target_version})
        </Badge>
      ) : null}
      {status.phase !== "idle" ? (
        <p role="status" className="break-words">
          Update: {status.phase.replaceAll("_", " ")}
          {status.code ? ` — ${status.code}` : ""}
        </p>
      ) : null}
      {status.guidance ? <p role="alert" className="break-words">{status.guidance}</p> : null}
      {manual ? (
        <div className="min-w-0 space-y-3 text-muted-foreground">
          {status.installation === "container" ? (
            <>
              <p>
                {legacy
                  ? "Replace only the updater on the computer running Docker. Keep the machine container and update volume."
                  : "Run once on the computer running Docker. No pairing token needed."}
              </p>
              <label className="block space-y-1">
                Docker container name
                <Input
                  value={container}
                  onChange={(event) => setContainer(event.target.value)}
                />
              </label>
              {command ? (
                <CopyableField
                  label={legacy ? "Replace updater on the Docker host" : "Run on the Docker host"}
                  value={command}
                />
              ) : (
                <p role="status">
                  {updater.data?.image
                    ? "Enter a valid container name."
                    : "Verifying the updater image, try again shortly."}
                </p>
              )}
              <details>
                <summary className="cursor-pointer text-foreground">
                  How updates work
                </summary>
                <p className="pt-2">
                  The command preserves your container’s identity, workspace and
                  settings. The updater has Docker socket access (host root). It
                  verifies official NyxID images and restores the old container
                  if reconnection fails.
                </p>
              </details>
            </>
          ) : (
            <>
              <p>
                On this machine, update NyxID with{" "}
                <code>nyxid update --version {status.target_version}</code>,
                then run <code>nyxid node machine-updater install</code> as the
                supervisor user. For a separated VM, add <code>--system</code>;
                for a named profile, add <code>--profile</code> with that
                profile's name. For a custom configuration directory, add{" "}
                <code>--config</code> with its path. The command restarts the
                node daemon.
              </p>
            </>
          )}
          <p>
            Paste the command in that computer’s terminal and wait. Start guided
            update to let your agent watch{" "}
            {legacy ? "the new updater report its version" : "reconnection"}.
          </p>
        </div>
      ) : null}
      {confirming ? (
        <div className="space-y-2 rounded-lg border border-border p-3">
          <p>
            {legacy
              ? `Watch the one-time updater replacement for ${status.target_version}? Run the pinned host command above; the machine stays running.`
              : `Update to ${status.target_version}? The machine restarts and interrupts its current work.`}
          </p>
          <div className="flex flex-wrap gap-2">
            <Button
              type="button"
              disabled={start.isPending}
              onClick={() => {
                start.mutate(undefined, {
                  onSuccess: () => setConfirming(false),
                });
              }}
            >
              Confirm update
            </Button>
            <Button type="button" onClick={() => setConfirming(false)}>
              Cancel
            </Button>
          </div>
        </div>
      ) : (
        <Button
          type="button"
          disabled={busy || start.isPending}
          onClick={() => setConfirming(true)}
        >
          {manual ? "Start guided update" : "Update machine"}
        </Button>
      )}
      {start.error ? (
        <p role="alert" className="text-destructive">
          {start.error.message}
        </p>
      ) : null}
      {start.data?.reason ? <p role="alert">{start.data.reason}</p> : null}
    </div>
  );
}
