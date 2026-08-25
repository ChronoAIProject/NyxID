export function resolveDeviceLoginDeadlineMs(
  expiresAt: string,
  secondsRemaining: number | null,
  nowMs = Date.now(),
): number {
  if (secondsRemaining !== null) {
    return nowMs + Math.max(0, secondsRemaining) * 1000;
  }
  const parsed = Date.parse(expiresAt);
  return Number.isFinite(parsed) ? parsed : nowMs;
}

export function secondsUntilDeviceLoginDeadline(
  deadlineMs: number,
  nowMs = Date.now(),
): number {
  return Math.max(0, Math.ceil((deadlineMs - nowMs) / 1000));
}

export function formatDeviceLoginRelativeTime(
  value: string,
  nowMs = Date.now(),
): string {
  const timestamp = Date.parse(value);
  if (!Number.isFinite(timestamp)) return "Unknown";
  const seconds = Math.max(0, Math.floor((nowMs - timestamp) / 1000));
  if (seconds < 60) return `${seconds} second${seconds === 1 ? "" : "s"} ago`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes} minute${minutes === 1 ? "" : "s"} ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} hour${hours === 1 ? "" : "s"} ago`;
  const days = Math.floor(hours / 24);
  return `${days} day${days === 1 ? "" : "s"} ago`;
}
