import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { Label } from "@/components/ui/label";
import {
  GroupedList,
  GroupedRow,
  GroupedInput,
} from "@/components/ui/grouped-list";
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
  grouped = false,
}: {
  readonly name: string;
  readonly status: MachineUpdateStatus;
  readonly grouped?: boolean;
}) {
  useUiTranslation();
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
  if (grouped)
    return (
      <GroupedList
        title={translateUi("Version and updates")}
        footer={
          <>
            {legacy ? (
              <p role="status">{translateUi("Updater predates self-update. Replace it once")}</p>
            ) : (
              status.updater?.phase !== "current" &&
              status.updater && (
                <p role="status">{translateUi("Updater update ")}{status.updater.phase}
                  {status.updater.target_version
                    ? translateUi(" ({{target_version}})", { target_version: status.updater.target_version })
                    : ""}
                  {status.updater.code ? translateUi(" — {{code}}", { code: status.updater.code }) : ""}
                </p>
              )
            )}
            {status.updater_guidance && (
              <p role="alert">{status.updater_guidance}</p>
            )}
            {status.updater?.digest && (
              <details>
                <summary>{translateUi("Updater image digest")}</summary>
                <code className="break-all">{status.updater.digest}</code>
              </details>
            )}
            {status.phase !== "idle" && (
              <p role="status">{translateUi("Update: ")}{status.phase.replaceAll("_", " ")}
                {status.code ? translateUi(" — {{code}}", { code: status.code }) : ""}
              </p>
            )}
            {status.guidance && <p role="alert">{status.guidance}</p>}
            {manual && (
              <>
                {status.installation === "container" ? (
                  <>
                    <p>
                      {legacy
                        ? translateUi("Replace only the updater on the computer running Docker. Keep the machine container and update volume.")
                        : translateUi("Run once on the computer running Docker. No pairing token needed.")}
                    </p>
                    {!command && (
                      <p role="status">
                        {updater.data?.image
                          ? translateUi("Enter a valid container name.")
                          : translateUi("Verifying the updater image, try again shortly.")}
                      </p>
                    )}
                    <details>
                      <summary>{translateUi("How updates work")}</summary>
                      <p>{translateUi("The command preserves your container’s identity, workspace and settings. The updater has Docker socket access (host root). It verifies official NyxID images and restores the old container if reconnection fails.")}</p>
                    </details>
                  </>
                ) : (
                  <p>{translateUi("On this machine, update NyxID with")}{" "}
                    <code>nyxid update --version {status.target_version}</code>{translateUi(", then run ")}<code>nyxid node machine-updater install</code>{translateUi(" as the supervisor user. For a separated VM, add")}{" "}
                    <code>--system</code>{translateUi("; for a named profile, add")}{" "}
                    <code>--profile</code>{translateUi(" with that profile's name. For a custom configuration directory, add ")}<code>--config</code>{" "}{translateUi("with its path. The command restarts the node daemon.")}</p>
                )}
                <p>{translateUi("Paste the command in that computer’s terminal and wait. Start guided update to let your agent watch")}{" "}
                  {legacy ? translateUi("the new updater report its version") : translateUi("reconnection")}.
                </p>
              </>
            )}
            {confirming && (
              <p role="alert">
                {legacy
                  ? translateUi("Watch the one-time updater replacement for {{value1}}? Run the pinned host command above; the machine stays running.", { value1: status.target_version })
                  : translateUi("Update to {{value1}}? The machine restarts and interrupts its current work.", { value1: status.target_version })}
              </p>
            )}
            {start.error && (
              <p role="alert" className="text-destructive">
                {start.error.message}
              </p>
            )}
            {start.data?.reason && <p role="alert">{start.data.reason}</p>}
          </>
        }
      >
        <GroupedRow title={translateUi("Installed")} accessory={status.current_version} />
        <GroupedRow
          title={translateUi("Latest")}
          description={status.update_available ? translateUi("Update available") : undefined}
          accessory={status.target_version}
        />
        {status.updater && (
          <GroupedRow title={translateUi("Updater")} accessory={status.updater.version} />
        )}
        {manual && status.installation === "container" && (
          <>
            <GroupedRow asChild className="block space-y-0.5">
              <Label>
                <span className="text-text-tertiary">{translateUi("Docker container name")}</span>
                <GroupedInput
                  value={container}
                  onChange={(event) => setContainer(event.target.value)}
                />
              </Label>
            </GroupedRow>
            {command && (
              <GroupedRow>
                <CopyableField
                  grouped
                  label={
                    legacy
                      ? translateUi("Replace updater on the Docker host")
                      : translateUi("Run on the Docker host")
                  }
                  value={command}
                />
              </GroupedRow>
            )}
          </>
        )}
        {confirming ? (
          <>
            <GroupedRow asChild interactive>
              <button
                type="button"
                disabled={start.isPending}
                onClick={() =>
                  start.mutate(undefined, {
                    onSuccess: () => setConfirming(false),
                  })
                }
              >{translateUi("Confirm update")}</button>
            </GroupedRow>
            <GroupedRow asChild interactive>
              <button
                type="button"
                disabled={start.isPending}
                onClick={() => setConfirming(false)}
              >{translateUi("Cancel")}</button>
            </GroupedRow>
          </>
        ) : (
          <GroupedRow asChild interactive>
            <button
              type="button"
              disabled={busy || start.isPending}
              onClick={() => setConfirming(true)}
            >
              {manual ? translateUi("Start guided update") : translateUi("Update machine")}
            </button>
          </GroupedRow>
        )}
      </GroupedList>
    );
  return (
    <div className="min-w-0 space-y-3">
      <p>{translateUi("Installed: ")}{status.current_version}{translateUi(" · Latest: ")}{status.target_version}
      </p>
      {legacy ? (
        <p role="status">{translateUi("Updater predates self-update. Replace it once")}</p>
      ) : status.updater ? (
        <div className="min-w-0 space-y-2">
          <p>{translateUi("Updater: ")}{status.updater.version}</p>
          {status.updater.phase !== "current" ? (
            <p role="status" className="break-words">{translateUi("Updater update ")}{status.updater.phase}
              {status.updater.target_version ? translateUi(" ({{target_version}})", { target_version: status.updater.target_version }) : ""}
              {status.updater.code ? translateUi(" — {{code}}", { code: status.updater.code }) : ""}
            </p>
          ) : null}
          {status.updater_guidance ? (
            <p role="alert" className="break-words">{status.updater_guidance}</p>
          ) : null}
          {status.updater.digest ? (
            <details>
              <summary className="cursor-pointer">{translateUi("Updater image digest")}</summary>
              <code className="block break-all pt-2 text-14">{status.updater.digest}</code>
            </details>
          ) : null}
        </div>
      ) : status.installation === "container" && status.updater_ready ? (
        <p>{translateUi("Updater: version unavailable")}</p>
      ) : null}
      {status.update_available ? (
        <Badge variant="warning">{translateUi("Update available (")}{status.target_version})
        </Badge>
      ) : null}
      {status.phase !== "idle" ? (
        <p role="status" className="break-words">{translateUi("Update: ")}{status.phase.replaceAll("_", " ")}
          {status.code ? translateUi(" — {{code}}", { code: status.code }) : ""}
        </p>
      ) : null}
      {status.guidance ? <p role="alert" className="break-words">{status.guidance}</p> : null}
      {manual ? (
        <div className="min-w-0 space-y-3 text-muted-foreground">
          {status.installation === "container" ? (
            <>
              <p>
                {legacy
                  ? translateUi("Replace only the updater on the computer running Docker. Keep the machine container and update volume.")
                  : translateUi("Run once on the computer running Docker. No pairing token needed.")}
              </p>
              <Label className="block space-y-1">{translateUi("Docker container name")}<Input
                  value={container}
                  onChange={(event) => setContainer(event.target.value)}
                />
              </Label>
              {command ? (
                <CopyableField
                  label={legacy ? translateUi("Replace updater on the Docker host") : translateUi("Run on the Docker host")}
                  value={command}
                />
              ) : (
                <p role="status">
                  {updater.data?.image
                    ? translateUi("Enter a valid container name.")
                    : translateUi("Verifying the updater image, try again shortly.")}
                </p>
              )}
              <details>
                <summary className="cursor-pointer text-foreground">{translateUi("How updates work")}</summary>
                <p className="pt-2">{translateUi("The command preserves your container’s identity, workspace and settings. The updater has Docker socket access (host root). It verifies official NyxID images and restores the old container if reconnection fails.")}</p>
              </details>
            </>
          ) : (
            <>
              <p>{translateUi("On this machine, update NyxID with")}{" "}
                <code>nyxid update --version {status.target_version}</code>{translateUi(", then run ")}<code>nyxid node machine-updater install</code>{translateUi(" as the supervisor user. For a separated VM, add ")}<code>--system</code>{translateUi("; for a named profile, add ")}<code>--profile</code>{translateUi(" with that profile's name. For a custom configuration directory, add")}{" "}
                <code>--config</code>{translateUi(" with its path. The command restarts the node daemon.")}</p>
            </>
          )}
          <p>{translateUi("Paste the command in that computer’s terminal and wait. Start guided update to let your agent watch")}{" "}
            {legacy ? translateUi("the new updater report its version") : translateUi("reconnection")}.
          </p>
        </div>
      ) : null}
      {confirming ? (
        <div className="space-y-2 rounded-lg border border-border p-3">
          <p>
            {legacy
              ? translateUi("Watch the one-time updater replacement for {{value1}}? Run the pinned host command above; the machine stays running.", { value1: status.target_version })
              : translateUi("Update to {{value1}}? The machine restarts and interrupts its current work.", { value1: status.target_version })}
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
            >{translateUi("Confirm update")}</Button>
            <Button type="button" onClick={() => setConfirming(false)}>{translateUi("Cancel")}</Button>
          </div>
        </div>
      ) : (
        <Button
          type="button"
          disabled={busy || start.isPending}
          onClick={() => setConfirming(true)}
        >
          {manual ? translateUi("Start guided update") : translateUi("Update machine")}
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
