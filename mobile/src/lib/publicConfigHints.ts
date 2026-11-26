/** Launch-local discovery only; account permission remains authoritative. */
let crashDiagnosticsOffered = false;
const listeners = new Set<() => void>();

export function getCrashDiagnosticsOffered(): boolean {
  return crashDiagnosticsOffered;
}

export function cachePublicConfigHints(payload: unknown): void {
  const offered = payload !== null && typeof payload === "object" &&
    (payload as { crash_diagnostics_offered?: unknown }).crash_diagnostics_offered === true;
  if (offered === crashDiagnosticsOffered) return;
  crashDiagnosticsOffered = offered;
  for (const listener of listeners) listener();
}

export function subscribePublicConfigHints(listener: () => void): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}
