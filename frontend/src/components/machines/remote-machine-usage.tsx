import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { getFormattingLocale } from "@/lib/formatting-locale";
import { useRemoteMachineUsage } from "@/hooks/use-remote-machine-usage";
import { ErrorBanner } from "@/components/shared/error-banner";
import type { RemoteMachineUsage as Usage } from "@/schemas/remote-machine-usage";

function formatBytes(value: number) {
  return translateUi("{{value}} bytes", {
    value: value.toLocaleString(getFormattingLocale()),
  });
}

function Volume({
  label,
  volume,
}: {
  readonly label: string;
  readonly volume: Usage["workspace"] | Usage["temporary"];
}) {
  useUiTranslation();
  return (
    <div className="space-y-1">
      <p className="font-medium">{label}</p>
      {volume ? (
        <>
          <p>
            {formatBytes(volume.used_bytes)}{translateUi(" used")}{volume.capacity_bytes == null
              ? translateUi(". Capacity: Unknown; no fixed capacity declared.")
              : translateUi(" / {{value1}} capacity.", { value1: formatBytes(volume.capacity_bytes) })}
          </p>
          <p>{translateUi("Available:")}{" "}
            {volume.available_bytes == null
              ? translateUi("Unknown")
              : formatBytes(volume.available_bytes)}
            .
          </p>
          <p className="text-muted-foreground">{translateUi("Measurement: ")}{volume.measurement}.{" "}
            {volume.complete
              ? translateUi("Complete observation.")
              : translateUi("Partial observation; usage may be higher.")}
          </p>
        </>
      ) : (
        <p>{translateUi("Usage unknown; no current measurement was returned.")}</p>
      )}
    </div>
  );
}

export function RemoteMachineUsage({
  machineId,
}: {
  readonly machineId: string;
}) {
  useUiTranslation();
  const query = useRemoteMachineUsage(machineId);
  const usage = query.data;
  // Older servers may expose a partial component sum as used_bytes. It cannot
  // establish whole-Pod usage without the independently reported aggregate.
  const platformAggregate = usage?.platform_storage?.components.find(
    (component) => component.kind === "pod_ephemeral_aggregate",
  );
  const podUsedBytes =
    platformAggregate?.used_bytes === usage?.platform_storage?.used_bytes
      ? usage?.platform_storage?.used_bytes
      : null;
  return (
    <section
      aria-label={translateUi("Observed storage usage")}
      className="space-y-3 px-4 py-3 text-14"
    >
      {query.isPending ? (
        <p role="status">{translateUi("Loading storage observations…")}</p>
      ) : query.error ? (
        <ErrorBanner
          message={translateUi("Storage observations are unavailable. No empty-disk or free-space reading can be inferred.")}
          onRetry={() => void query.refetch()}
        />
      ) : usage ? (
        <>
          {!usage.available ? (
            <p>{translateUi("No usage has been reported by the current runtime. Disk usage is unknown.")}</p>
          ) : (
            <>
              <p className="text-muted-foreground">
                {usage.stale
                  ? translateUi("Stale observation — current usage may differ.")
                  : translateUi("Latest reported observation.")}{" "}{translateUi("Measured:")}{" "}
                {usage.sampled_at
                  ? new Date(usage.sampled_at).toLocaleString(getFormattingLocale())
                  : translateUi("Unknown")}
                {translateUi(". Received:")}{" "}
                {usage.received_at
                  ? new Date(usage.received_at).toLocaleString(getFormattingLocale())
                  : translateUi("Unknown")}
                .
              </p>
              <Volume label={translateUi("Workspace")} volume={usage.workspace} />
              <Volume label={translateUi("Observed temporary paths")} volume={usage.temporary} />
              {usage.temporary?.complete === false && (
                <p className="text-muted-foreground">{translateUi("Temporary usage is a lower bound from the reported paths, not complete Pod storage accounting. Unmeasured files can consume more space; remaining Pod quota cannot be inferred.")}</p>
              )}
              {usage.disk_full ? (
                <p role="alert" className="text-destructive">{translateUi("The runtime reported a full disk. Writes and jobs may fail.")}</p>
              ) : null}
            </>
          )}
          <section
            aria-label={translateUi("Platform temporary storage")}
            className="space-y-1"
          >
            <p className="font-medium">{translateUi("Platform temporary storage")}</p>
            {usage.platform_storage ? (
              <>
                <p>
                  {podUsedBytes == null
                    ? translateUi("Total Pod usage unknown")
                    : translateUi("{{value1}} reported used", { value1: formatBytes(podUsedBytes) })}
                  {translateUi(". No fixed capacity declared. Available: Unknown. ")}</p>
                <p>
                  {usage.platform_storage.stale
                    ? translateUi("Stale platform observation; fresh Node reports do not refresh this reading.")
                    : translateUi("Latest platform observation.")}{" "}{translateUi("Source: ")}{usage.platform_storage.source}{translateUi(". Received:")}{" "}
                  {new Date(
                    usage.platform_storage.received_at,
                  ).toLocaleString(getFormattingLocale())}
                  .
                </p>
                <p>
                  {usage.platform_storage.complete
                    ? translateUi("Expected accounting components are covered; this does not guarantee quota enforcement.")
                    : translateUi("Partial accounting; temporary usage may be higher.")}
                </p>
                {usage.platform_storage.components.length > 0 && (
                  <ul className="list-disc pl-5">
                    {usage.platform_storage.components.map(
                      (component, index) => (
                        <li key={`${component.kind}:${index}`}>
                          {component.kind.replaceAll("_", " ")}:{" "}
                          {formatBytes(component.used_bytes)}{translateUi(", measured")}{" "}
                          {new Date(component.sampled_at).toLocaleString(getFormattingLocale())}.
                        </li>
                      ),
                    )}
                  </ul>
                )}
                {usage.platform_storage.missing_reasons.map((reason) => (
                  <p key={reason}>{reason.replaceAll("_", " ")}</p>
                ))}
                <p className="text-muted-foreground">{translateUi("The reported Pod aggregate is shown once. Component readings and the Node path scan are separate observations and are not added to it. Workspace storage and memory-backed volumes are excluded.")}</p>
              </>
            ) : (
              <p>{translateUi("Optional Pod monitoring is unavailable. Total Pod temporary usage is unknown; Node path scans cannot establish total Pod usage or remaining temporary allocation. Workspace reporting and normal Machine operations remain available.")}</p>
            )}
          </section>
          <p>{translateUi("Node decoded file-write limit:")}{" "}
            {formatBytes(usage.write_limit_bytes)}{translateUi(". This limits decoded content, not the size of the complete request. ")}</p>
          <p>{translateUi("NyxBot whole-JSON argument limit:")}{" "}
            {usage.assistant_arguments_json_limit_bytes === undefined
              ? translateUi("Not reported by this server")
              : formatBytes(usage.assistant_arguments_json_limit_bytes)}
            {translateUi(". Escaped content and every other argument count toward this smaller transport limit, so a write below the Node limit can still be too large for NyxBot. ")}</p>
          <p>{translateUi("Exact-edit original and result file limit:")}{" "}
            {usage.edit_file_limit_bytes === undefined
              ? translateUi("Not reported by this server")
              : formatBytes(usage.edit_file_limit_bytes)}
            {translateUi(". This does not mean an equally large edit request fits the transport; request and protocol limits still apply. ")}</p>
          <p className="text-muted-foreground">{translateUi("Temporary storage is managed by the deployment environment, with no fixed capacity commitment. Temporary data may be lost after pause or Pod recreation. Disk pressure can evict the Pod; full disks can cause writes and jobs to fail.")}</p>
          <p className="text-muted-foreground">{usage.recovery}</p>
        </>
      ) : null}
    </section>
  );
}
