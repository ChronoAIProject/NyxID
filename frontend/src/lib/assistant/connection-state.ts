/**
 * Whether NyxID answered the most recent assistant request. A request that
 * fails before any response marks the connection lost; any response, even an
 * error, marks it back. Presentation only: requests are never gated on it.
 */
// Dev-server preview: `?offline=unreachable` holds the state as if NyxID
// were not answering. Compiled out of production builds.
const previewUnreachable =
  import.meta.env.DEV &&
  typeof window !== "undefined" &&
  new URLSearchParams(window.location.search).get("offline") === "unreachable";
let lost = previewUnreachable;
const listeners = new Set<() => void>();

function set(next: boolean) {
  if (next === lost || previewUnreachable) return;
  lost = next;
  for (const listener of listeners) listener();
}

export function markConnectionLost(): void {
  set(true);
}

export function markConnectionOk(): void {
  set(false);
}

export function isConnectionLost(): boolean {
  return lost;
}

export function subscribeConnection(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}
