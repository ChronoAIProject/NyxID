import { useRemoteMachineUsage } from "@/hooks/use-remote-machine-usage";
import { ErrorBanner } from "@/components/shared/error-banner";
import type { RemoteMachineUsage as Usage } from "@/schemas/remote-machine-usage";

function formatBytes(value: number) {
  return `${value.toLocaleString()} bytes`;
}

function Volume({
  label,
  volume,
}: {
  readonly label: string;
  readonly volume: Usage["workspace"] | Usage["temporary"];
}) {
  return (
    <div className="space-y-1">
      <p className="font-medium">{label}</p>
      {volume ? (
        <>
          <p>
            {formatBytes(volume.used_bytes)} used
            {volume.capacity_bytes == null
              ? ". Capacity: Unknown; no fixed capacity declared."
              : ` / ${formatBytes(volume.capacity_bytes)} capacity.`}
          </p>
          <p>
            Available:{" "}
            {volume.available_bytes == null
              ? "Unknown"
              : formatBytes(volume.available_bytes)}
            .
          </p>
          <p className="text-muted-foreground">
            Measurement: {volume.measurement}.{" "}
            {volume.complete
              ? "Complete observation."
              : "Partial observation; usage may be higher."}
          </p>
        </>
      ) : (
        <p>Usage unknown; no current measurement was returned.</p>
      )}
    </div>
  );
}

export function RemoteMachineUsage({
  machineId,
}: {
  readonly machineId: string;
}) {
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
      aria-label="Observed storage usage"
      className="space-y-3 px-4 py-3 text-12"
    >
      {query.isPending ? (
        <p role="status">Loading storage observations…</p>
      ) : query.error ? (
        <ErrorBanner
          message="Storage observations are unavailable. No empty-disk or free-space reading can be inferred."
          onRetry={() => void query.refetch()}
        />
      ) : usage ? (
        <>
          {!usage.available ? (
            <p>
              No usage has been reported by the current runtime. Disk usage is
              unknown.
            </p>
          ) : (
            <>
              <p className="text-muted-foreground">
                {usage.stale
                  ? "Stale observation — current usage may differ."
                  : "Latest reported observation."}{" "}
                Measured:{" "}
                {usage.sampled_at
                  ? new Date(usage.sampled_at).toLocaleString()
                  : "Unknown"}
                . Received:{" "}
                {usage.received_at
                  ? new Date(usage.received_at).toLocaleString()
                  : "Unknown"}
                .
              </p>
              <Volume label="Workspace" volume={usage.workspace} />
              <Volume label="Observed temporary paths" volume={usage.temporary} />
              {usage.temporary?.complete === false && (
                <p className="text-muted-foreground">
                  Temporary usage is a lower bound from the reported paths, not
                  complete Pod storage accounting. Unmeasured files can consume
                  more space; remaining Pod quota cannot be inferred.
                </p>
              )}
              {usage.disk_full ? (
                <p role="alert" className="text-destructive">
                  The runtime reported a full disk. Writes and jobs may fail.
                </p>
              ) : null}
            </>
          )}
          <section
            aria-label="Platform temporary storage"
            className="space-y-1"
          >
            <p className="font-medium">Platform temporary storage</p>
            {usage.platform_storage ? (
              <>
                <p>
                  {podUsedBytes == null
                    ? "Total Pod usage unknown"
                    : `${formatBytes(podUsedBytes)} reported used`}
                  . No fixed capacity declared. Available: Unknown.
                </p>
                <p>
                  {usage.platform_storage.stale
                    ? "Stale platform observation; fresh Node reports do not refresh this reading."
                    : "Latest platform observation."}{" "}
                  Source: {usage.platform_storage.source}. Received:{" "}
                  {new Date(
                    usage.platform_storage.received_at,
                  ).toLocaleString()}
                  .
                </p>
                <p>
                  {usage.platform_storage.complete
                    ? "Expected accounting components are covered; this does not guarantee quota enforcement."
                    : "Partial accounting; temporary usage may be higher."}
                </p>
                {usage.platform_storage.components.length > 0 && (
                  <ul className="list-disc pl-5">
                    {usage.platform_storage.components.map(
                      (component, index) => (
                        <li key={`${component.kind}:${index}`}>
                          {component.kind.replaceAll("_", " ")}:{" "}
                          {formatBytes(component.used_bytes)}, measured{" "}
                          {new Date(component.sampled_at).toLocaleString()}.
                        </li>
                      ),
                    )}
                  </ul>
                )}
                {usage.platform_storage.missing_reasons.map((reason) => (
                  <p key={reason}>{reason.replaceAll("_", " ")}</p>
                ))}
                <p className="text-muted-foreground">
                  The reported Pod aggregate is shown once. Component readings
                  and the Node path scan are separate observations and are not
                  added to it. Workspace storage and memory-backed volumes are
                  excluded.
                </p>
              </>
            ) : (
              <p>
                Optional Pod monitoring is unavailable. Total Pod temporary
                usage is unknown; Node path scans cannot establish total Pod
                usage or remaining temporary allocation. Workspace reporting
                and normal Machine operations remain available.
              </p>
            )}
          </section>
          <p>
            Node decoded file-write limit:{" "}
            {formatBytes(usage.write_limit_bytes)}. This limits decoded content,
            not the size of the complete request.
          </p>
          <p>
            NyxBot whole-JSON argument limit:{" "}
            {usage.assistant_arguments_json_limit_bytes === undefined
              ? "Not reported by this server"
              : formatBytes(usage.assistant_arguments_json_limit_bytes)}
            . Escaped content and every other argument count toward this smaller
            transport limit, so a write below the Node limit can still be too
            large for NyxBot.
          </p>
          <p>
            Exact-edit original and result file limit:{" "}
            {usage.edit_file_limit_bytes === undefined
              ? "Not reported by this server"
              : formatBytes(usage.edit_file_limit_bytes)}
            . This does not mean an equally large edit request fits the
            transport; request and protocol limits still apply.
          </p>
          <p className="text-muted-foreground">
            Temporary storage is managed by the deployment environment, with no
            fixed capacity commitment. Temporary data may be lost after pause
            or Pod recreation. Disk pressure can evict the Pod; full disks can
            cause writes and jobs to fail.
          </p>
          <p className="text-muted-foreground">{usage.recovery}</p>
        </>
      ) : null}
    </section>
  );
}
