/** NyxID-authored pending note, mirrored from assistant_engine_policy.rs. */
export const CAPACITY_ACKNOWLEDGEMENT = "Got it — I'm a bit busy right now and will start on this shortly.";

/** Elapsed wait rounded up to a whole second, matching the server formatter. */
export function formatCapacityWait(ms: number): string {
  const total = Math.ceil(Math.max(0, ms) / 1000);
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  const parts: string[] = [];
  if (minutes > 0) parts.push(`${minutes} minute${minutes === 1 ? "" : "s"}`);
  if (seconds > 0 || !parts.length) parts.push(`${seconds} second${seconds === 1 ? "" : "s"}`);
  return parts.join(" ");
}
