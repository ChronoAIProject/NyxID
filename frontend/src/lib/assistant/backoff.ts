export interface BackoffPolicy {
  readonly floorMs: number;
  readonly baseMs: number;
  readonly capMs: number;
  readonly deadlineMs: number;
}

export const PROJECTION_BACKOFF_POLICY: BackoffPolicy = {
  floorMs: 250,
  baseMs: 500,
  capMs: 30_000,
  deadlineMs: 90_000,
};

export const CREATE_RECOVERY_BACKOFF_POLICY: BackoffPolicy = {
  floorMs: 250,
  baseMs: 500,
  capMs: 8_000,
  deadlineMs: 60_000,
};

export function nextBackoffDelay(
  policy: BackoffPolicy,
  attempt: number,
  random: () => number = Math.random,
): number {
  const ceiling = Math.max(
    policy.floorMs,
    Math.min(policy.capMs, policy.baseMs * 2 ** Math.max(0, attempt)),
  );
  const sample = Math.min(1, Math.max(0, random()));
  return Math.round(policy.floorMs + sample * (ceiling - policy.floorMs));
}
