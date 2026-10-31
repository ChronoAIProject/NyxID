import type { RemoteMachine } from "@/schemas/machines";

/** Complete creation policies remain available without filling the free summary. */
export function RemoteMachineCreatePolicies({
  profile,
  policyVersion,
  dataPolicyVersion,
  expanded = false,
}: {
  readonly profile?: RemoteMachine["profile"];
  readonly policyVersion?: string;
  readonly dataPolicyVersion?: string;
  readonly expanded?: boolean;
}) {
  return (
    <details open={expanded} className="border-t border-border pt-3">
      <summary className="cursor-pointer font-medium focus-visible:outline-primary">
        Limits and data policies
      </summary>
      <div className="mt-3 space-y-4 text-muted-foreground">
        <section aria-label="Runtime limits" className="space-y-2">
          <h4 className="font-medium text-foreground">Runtime limits</h4>
          <p>
            {profile ? (
              <>
                Profile: {profile.profile_version}. New Machine fixed
                specification: {profile.cpu_millis / 1000} vCPU ·{" "}
                {profile.memory_mib / 1024} GiB memory · {profile.workspace_gib}{" "}
                GiB /workspace.{" "}
              </>
            ) : (
              "Machine specification is unavailable until preflight succeeds. "
            )}
            Memory includes the supervisor, browser, desktop and output cache.
            Workspace expansion and automatic upgrades are not available.
            Bandwidth and IOPS are not unlimited.
          </p>
          <p>
            Sharing is set per Agent. Shared legacy Agents and external clients
            share their configured OS user and workspace; file roots do not
            isolate shell commands. No inbound ports, public IP, persistent
            environment variables, snapshots or restore. Job environment
            variables are temporary.
          </p>
          <p>
            At most two jobs run concurrently. Default command timeout is 120
            seconds, maximum one hour. Each output cache is bounded to 1 MiB;
            the Machine cache is bounded to 32 MiB. Raw output remains only on
            the Node, lasts at most one hour after termination, and may
            disappear sooner.
          </p>
        </section>
        <section aria-label="Storage and data policies" className="space-y-2">
          <h4 className="font-medium text-foreground">
            Storage and data policies
          </h4>
          <p>
            Each personal account has one free slot, with no entitlement expiry
            or monthly runtime limit. A paused Machine still occupies the slot
            and retains its workspace indefinitely until owner-confirmed
            deletion. Cleanup and accounting finalization must complete before
            the slot becomes available again.
          </p>
          <p>
            The legacy /workspace and separated workspaces are retained on the
            protected persistent volume, together with supervisor-owned
            identity, generation and tombstone metadata. HOME, browser profiles,
            downloads and tmp remain temporary; mode changes do not move shared
            files or copy cookies. Recovery must verify the original volume and
            retained metadata; failure keeps execution unavailable.
          </p>
          <p>
            Temporary storage is managed by the deployment environment, with no
            fixed capacity commitment, selectable size or extra storage charge.
            HOME, /tmp, logs and the writable layer remain temporary. Temporary
            data may be lost after pause or Pod recreation. Full disks can cause
            writes and jobs to fail, and disk pressure can evict the Pod.
          </p>
          <p>
            Job metadata remains for 30 days and terminal operation history for
            90 days. Deletion queues workspace cleanup, targets completion
            within 24 hours, and retains necessary billing, cleanup and
            idempotency evidence. Underlying storage follows its reclaim policy;
            physical storage release is not verified. Paused workspace storage
            is free.
          </p>
        </section>
        {policyVersion || dataPolicyVersion ? (
          <p>
            Policy: {policyVersion}. Data policy: {dataPolicyVersion}.
          </p>
        ) : null}
      </div>
    </details>
  );
}
