/** Loudness of one 16-bit little-endian PCM packet, mapped from -50..-10 dBFS to 0..1. */
export function pcmLevel(bytes: ArrayBuffer): number {
  const samples = Math.floor(bytes.byteLength / 2);
  if (!samples) return 0;
  const view = new DataView(bytes);
  let sum = 0;
  for (let i = 0; i < samples; i++) {
    const value = view.getInt16(i * 2, true) / 32768;
    sum += value * value;
  }
  const rms = Math.sqrt(sum / samples);
  if (rms <= 0) return 0;
  return Math.min(1, Math.max(0, (20 * Math.log10(rms) + 50) / 40));
}

export function formatElapsed(ms: number): string {
  const seconds = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}
