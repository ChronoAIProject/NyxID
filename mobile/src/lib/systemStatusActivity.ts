// Capture owners overlap (for example QR scanning while a voice call ends).
// Releasing one owner must not restore the rail while another still uses it.
const owners = new Set<symbol>();
const listeners = new Set<() => void>();

export const systemStatusActivityActive = () => owners.size > 0;

export function subscribeSystemStatusActivity(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** Claim from a camera/microphone capture-start event; release on teardown. */
export function claimSystemStatusActivity() {
  const owner = Symbol("system status activity");
  const wasActive = systemStatusActivityActive();
  owners.add(owner);
  if (!wasActive) listeners.forEach((listener) => listener());
  return () => {
    if (!owners.delete(owner) || owners.size) return;
    listeners.forEach((listener) => listener());
  };
}
