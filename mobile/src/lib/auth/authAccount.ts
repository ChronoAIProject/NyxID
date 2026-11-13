/** Successful account admission is separate from credential-operation epochs. */
let accountId: string | null = null;
const listeners = new Set<() => void>();

export function getAuthAccountId(): string | null {
  return accountId;
}

export function setAuthAccountId(next: string | null): void {
  if (next === accountId) return;
  accountId = next;
  for (const listener of listeners) listener();
}

export function subscribeAuthAccount(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}
