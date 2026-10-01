import { useState } from "react";
import {
  useStartMachineUpdate,
  useVerifiedUpdaterImage,
} from "@/hooks/use-machines";
import {
  machineMigrationCommand,
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
  const busy = [
    "queued",
    "verifying",
    "downloading",
    "restarting",
    "manual_step",
  ].includes(status.phase);
  let command: string | undefined;
  if (!status.updater_ready && status.installation === "container") {
    try {
      command = machineMigrationCommand(
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
      {status.update_available ? (
        <Badge variant="warning">
          Update available ({status.target_version})
        </Badge>
      ) : null}
      {status.phase !== "idle" ? (
        <p role="status">
          Update: {status.phase.replaceAll("_", " ")}
          {status.code ? ` — ${status.code.replaceAll("_", " ")}` : ""}
        </p>
      ) : null}
      {!status.updater_ready ? (
        <div className="min-w-0 space-y-3 text-muted-foreground">
          {status.installation === "container" ? (
            <>
              <p>
                Run once on the computer running Docker. No pairing token
                needed.
              </p>
              <label className="block space-y-1">
                Docker container name
                <Input
                  value={container}
                  onChange={(event) => setContainer(event.target.value)}
                />
              </label>
              {command ? (
                <CopyableField label="Run on the Docker host" value={command} />
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
            update to let your agent watch reconnection.
          </p>
        </div>
      ) : null}
      {confirming ? (
        <div className="space-y-2 rounded-lg border border-border p-3">
          <p>
            Update to {status.target_version}? The machine restarts and
            interrupts its current work.
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
          {status.updater_ready ? "Update machine" : "Start guided update"}
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
