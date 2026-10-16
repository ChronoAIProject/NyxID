/** Keep both owner account-deletion confirmations on the same disclosed policy. */
export function AccountDeletionDisclosure() {
  return (
    <div className="space-y-3 text-12" role="note" aria-label="Machine data deletion and retained records">
      <p className="rounded-lg border border-destructive/40 bg-destructive/5 p-3">
        Deleting your account permanently deletes all of your managed Machines
        and their data, including paused /workspace disks and temporary files.
        This is irreversible: no snapshots, no recovery or restore.
      </p>
      <p className="text-muted-foreground">
        Access to Machine work and job output is blocked immediately. Resource
        cleanup targets 24 hours; missing stop or disk cleanup evidence keeps
        cleanup pending and escalates to operators.
      </p>
      <p className="text-muted-foreground">
        Raw job output is held only in memory for at most 1 hour after a job
        ends and may disappear sooner.
      </p>
      <p className="text-muted-foreground">
        Safe job metadata is retained for 30 days after the job ends, and
        operation history for 90 days after completion. Necessary deletion,
        retry, cleanup and settlement records remain until resource cleanup,
        billing and related retries finish. Audit and financial records follow
        the platform’s existing retention policy.
      </p>
    </div>
  );
}
