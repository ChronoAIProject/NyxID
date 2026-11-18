/**
 * The iOS recognizer reports peak input as -2..10 (-60..0 dBFS, below 0 is
 * inaudible). The meter shows -50..0 dBFS as 0..1.
 */
export function volumeLevel(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.min(1, Math.max(0, value / 10));
}

export function formatElapsed(ms: number): string {
  const seconds = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}
