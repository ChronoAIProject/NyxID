/** Bound to the policy accepted by DELETE /users/me after the native confirmation. */
export const ACCOUNT_DELETION_POLICY_VERSION = "machine-data";

export const ACCOUNT_DELETION_DISCLOSURE =
  "Deleting your account permanently deletes all of your managed Machines and their data, including paused /workspace disks and temporary files. This is irreversible: no snapshots, no recovery or restore.\n\n" +
  "Access to Machine work and job output is blocked immediately. Resource cleanup targets 24 hours; missing stop or disk cleanup evidence keeps cleanup pending and escalates to operators.\n\n" +
  "Raw job output is held only in memory for at most 1 hour after a job ends and may disappear sooner. Safe job metadata is retained for 30 days after the job ends, and operation history for 90 days after completion. Necessary deletion, retry, cleanup and settlement records remain until resource cleanup, billing and related retries finish. Audit and financial records follow the platform’s existing retention policy.";
