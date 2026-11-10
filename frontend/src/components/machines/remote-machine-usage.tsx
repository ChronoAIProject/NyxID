import { getFormattingLocale } from "@/lib/formatting-locale";
import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";
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
  const { t } = useUiTranslation();
  return (
    <div className="space-y-1">
      <p className="font-medium">{label}</p>
      {volume ? (
        <>
          <p>
            {formatBytes(volume.used_bytes)}{t(" used")}{volume.capacity_bytes == null
              ? t(". Capacity: Unknown; no fixed capacity declared.")
              : t(" / {{value}} capacity.", { value: formatBytes(volume.capacity_bytes) })}
          </p>
          <p>{t("Available:")}{" "}
            {volume.available_bytes == null
              ? t("Unknown")
              : formatBytes(volume.available_bytes)}
            .
          </p>
          <p className="text-muted-foreground">{t("Measurement: ")}{volume.measurement}.{" "}
            {volume.complete
              ? t("Complete observation.")
              : t("Partial observation; usage may be higher.")}
          </p>
        </>
      ) : (
        <p>{t("Usage unknown; no current measurement was returned.")}</p>
      )}
    </div>
  );
}

export function RemoteMachineUsage({
  machineId,
}: {
  readonly machineId: string;
}) {
  const { t } = useUiTranslation();
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
      aria-label={t("Observed storage usage")}
      className="space-y-3 px-4 py-3 text-12"
    >
      {query.isPending ? (
        <p role="status">{t("Loading storage observations…")}</p>
      ) : query.error ? (
        <ErrorBanner
          message={t("Storage observations are unavailable. No empty-disk or free-space reading can be inferred.")}
          onRetry={() => void query.refetch()}
        />
      ) : usage ? (
        <>
          {!usage.available ? (
            <p>{t("No usage has been reported by the current runtime. Disk usage is unknown.")}</p>
          ) : (
            <>
              <p className="text-muted-foreground">
                {usage.stale
                  ? t("Stale observation — current usage may differ.")
                  : t("Latest reported observation.")}{" "}{t("Measured:")}{" "}
                {usage.sampled_at
                  ? new Date(usage.sampled_at).toLocaleString(getFormattingLocale())
                  : t("Unknown")}{t(". Received:")}{" "}
                {usage.received_at
                  ? new Date(usage.received_at).toLocaleString(getFormattingLocale())
                  : t("Unknown")}
                .
              </p>
              <Volume label={t("Workspace")} volume={usage.workspace} />
              <Volume label={t("Observed temporary paths")} volume={usage.temporary} />
              {usage.temporary?.complete === false && (
                <p className="text-muted-foreground">{t("Temporary usage is a lower bound from the reported paths, not complete Pod storage accounting. Unmeasured files can consume more space; remaining Pod quota cannot be inferred.")}</p>
              )}
              {usage.disk_full ? (
                <p role="alert" className="text-destructive">{t("The runtime reported a full disk. Writes and jobs may fail.")}</p>
              ) : null}
            </>
          )}
          <section
            aria-label={t("Platform temporary storage")}
            className="space-y-1"
          >
            <p className="font-medium">{t("Platform temporary storage")}</p>
            {usage.platform_storage ? (
              <>
                <p>
                  {podUsedBytes == null
                    ? t("Total Pod usage unknown")
                    : t("{{value}} reported used", { value: formatBytes(podUsedBytes) })}{t(". No fixed capacity declared. Available: Unknown.")}</p>
                <p>
                  {usage.platform_storage.stale
                    ? t("Stale platform observation; fresh Node reports do not refresh this reading.")
                    : t("Latest platform observation.")}{" "}{t("Source: ")}{usage.platform_storage.source}{t(". Received:")}{" "}
                  {new Date(
                    usage.platform_storage.received_at,
                  ).toLocaleString(getFormattingLocale())}
                  .
                </p>
                <p>
                  {usage.platform_storage.complete
                    ? t("Expected accounting components are covered; this does not guarantee quota enforcement.")
                    : t("Partial accounting; temporary usage may be higher.")}
                </p>
                {usage.platform_storage.components.length > 0 && (
                  <ul className="list-disc pl-5">
                    {usage.platform_storage.components.map(
                      (component, index) => (
                        <li key={`${component.kind}:${index}`}>
                          {component.kind.replaceAll("_", " ")}:{" "}
                          {formatBytes(component.used_bytes)}{t(", measured")}{" "}
                          {new Date(component.sampled_at).toLocaleString(getFormattingLocale())}.
                        </li>
                      ),
                    )}
                  </ul>
                )}
                {usage.platform_storage.missing_reasons.map((reason) => (
                  <p key={reason}>{reason.replaceAll("_", " ")}</p>
                ))}
                <p className="text-muted-foreground">{t("The reported Pod aggregate is shown once. Component readings and the Node path scan are separate observations and are not added to it. Workspace storage and memory-backed volumes are excluded.")}</p>
              </>
            ) : (
              <p>{t("Optional Pod monitoring is unavailable. Total Pod temporary usage is unknown; Node path scans cannot establish total Pod usage or remaining temporary allocation. Workspace reporting and normal Machine operations remain available.")}</p>
            )}
          </section>
          <p>{t("Node decoded file-write limit:")}{" "}
            {formatBytes(usage.write_limit_bytes)}{t(". This limits decoded content, not the size of the complete request.")}</p>
          <p>{t("NyxBot whole-JSON argument limit:")}{" "}
            {usage.assistant_arguments_json_limit_bytes === undefined
              ? t("Not reported by this server")
              : formatBytes(usage.assistant_arguments_json_limit_bytes)}{t(". Escaped content and every other argument count toward this smaller transport limit, so a write below the Node limit can still be too large for NyxBot.")}</p>
          <p>{t("Exact-edit original and result file limit:")}{" "}
            {usage.edit_file_limit_bytes === undefined
              ? t("Not reported by this server")
              : formatBytes(usage.edit_file_limit_bytes)}{t(". This does not mean an equally large edit request fits the transport; request and protocol limits still apply.")}</p>
          <p className="text-muted-foreground">{t("Temporary storage is managed by the deployment environment, with no fixed capacity commitment. Temporary data may be lost after pause or Pod recreation. Disk pressure can evict the Pod; full disks can cause writes and jobs to fail.")}</p>
          <p className="text-muted-foreground">{usage.recovery}</p>
        </>
      ) : null}
    </section>
  );
}
